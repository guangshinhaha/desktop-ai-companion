import Foundation

/// Minimal JSON-RPC 2.0 over stdio, one JSON message per line (TD 3.2).
///
/// Shell to executor: `ping`, and `replay.start {task, inputs, pid?, ceiling?}`. While a replay runs the shell
/// may also send the notifications `replay.pause`, `replay.resume` and `replay.stop` (no id, no reply), and the
/// executor sends `run.progress {state: started|done|paused|resumed, step, say, index, total}` (no id, no wait).
/// A stop ends the run with outcome `stopped`, after the executor records `run_stopped`. While a replay runs the
/// executor sends the shell two requests and waits for each answer:
///   `approval.request {card}`  -> `{approved: true|false}`. Only the JSON value `true` approves.
///   `audit.append {event, step?, shown?, item?}` -> `{seq}`. Anything else stops the run.
/// There is no timeout on either, and no answer (the shell closing its end) is a no. The officer's
/// identity, the task reference and the run id are the shell's: the executor never names them.
public enum RPC {
    public static let version = "0.1.0"

    /// Finds the app a task drives. The default is the accessibility adapter; tests inject a fake.
    public typealias HostFactory = (ReplayTask, pid_t?) throws -> ReplayHost

    public static func accessibilityHost(_ task: ReplayTask, _ pid: pid_t?) throws -> ReplayHost {
        try AXApp(bundleId: task.bundle, pid: pid, requireUnique: true)
    }

    /// Handles one request line. Returns the response line, or nil for a notification (no id).
    public static func handle(line: String) -> String? {
        guard let data = line.data(using: .utf8),
              let obj = try? JSONSerialization.jsonObject(with: data) as? [String: Any]
        else { return encode(["jsonrpc": "2.0", "id": NSNull(), "error": err(-32700, "Parse error")]) }
        let id = obj["id"]
        guard obj["jsonrpc"] as? String == "2.0", let method = obj["method"] as? String else {
            return encode(["jsonrpc": "2.0", "id": id ?? NSNull(), "error": err(-32600, "Invalid Request")])
        }
        guard let id else { return nil }
        switch method {
        case "ping":
            return encode(["jsonrpc": "2.0", "id": id, "result": ["pong": true, "version": version]])
        default:
            return encode(["jsonrpc": "2.0", "id": id, "error": err(-32601, "Method not found")])
        }
    }

    /// Reads requests until stdin closes, then returns. Blocks on read, so it is idle between requests.
    /// `input` waits for the next line; `poll` returns a line only if one is already there, so a stop is seen
    /// while a step waits. With no `input`, both read standard input. Without `poll` (tests) a run cannot be
    /// stopped from outside, except while it waits on an answer.
    public static func serve(input: (() -> String?)? = nil,
                             poll: (() -> String?)? = nil,
                             output: @escaping (String) -> Void = { FileHandle.standardOutput.write(Data(($0 + "\n").utf8)) },
                             hostFactory: @escaping HostFactory = RPC.accessibilityHost) {
        let stdin = LineReader(fd: STDIN_FILENO)
        let read: () -> String? = input ?? { stdin.readLine() }
        let peek: (() -> String?)? = input == nil ? { stdin.pollLine() } : poll
        let talk = Conversation(input: read, output: output)
        while let line = read() {
            if line.trimmingCharacters(in: .whitespaces).isEmpty { continue }
            if let req = parse(line), req["method"] as? String == "replay.start", let id = req["id"] {
                let control = RunControl(poll: peek, block: read)
                talk.control = control
                output(replayStart(id: id, params: req["params"] as? [String: Any] ?? [:], talk: talk, control: control, hostFactory: hostFactory))
                talk.control = nil
                continue
            }
            if let reply = handle(line: line) { output(reply) }
        }
    }

    // MARK: replay

    static func replayStart(id: Any, params: [String: Any], talk: Conversation, control: RunControl, hostFactory: HostFactory) -> String {
        func fail(_ code: Int, _ m: String) -> String { encode(["jsonrpc": "2.0", "id": id, "error": err(code, m)]) }
        guard let taskObj = params["task"] as? [String: Any], let inputsObj = params["inputs"] as? [String: Any],
              let taskData = try? JSONSerialization.data(withJSONObject: taskObj),
              let inputsData = try? JSONSerialization.data(withJSONObject: inputsObj) else {
            return fail(-32602, "replay.start needs task and inputs objects")
        }
        let task: ReplayTask
        do { task = try ReplayTask.load(from: taskData) } catch { return fail(-32602, "task not usable: \(error)") }
        let inputs: RunInputs
        do { inputs = try RunInputs.load(from: inputsData, for: task) } catch { return fail(-32602, "inputs not usable: \(error)") }
        var ceiling = Replayer.defaultCeiling
        if let c = (params["ceiling"] as? NSNumber)?.doubleValue {
            guard c > 0, c <= 3600 else { return fail(-32602, "ceiling must be above 0 and at most 3600") }
            ceiling = c
        }
        var pid: pid_t?
        if let p = (params["pid"] as? NSNumber)?.int32Value {
            guard p > 1 else { return fail(-32602, "pid must be a process id") }
            pid = p
        }
        let host: ReplayHost
        do { host = try hostFactory(task, pid) } catch { return fail(-32000, "\(error)") }

        let gate = ApprovalGate(source: RPCApprovalSource(talk: talk), out: { _ in })
        let report = Replayer(task: task, inputs: inputs, host: host, gate: gate, ceiling: ceiling,
                              audit: RPCAudit(talk: talk), control: control, progress: { p in
            // A notification: no id, nothing waits for it. Step names and the task's say line only.
            talk.output(encode(["jsonrpc": "2.0", "method": "run.progress", "params": [
                "state": p.kind.rawValue, "step": p.step, "say": p.say, "index": p.index, "total": p.total]]))
        }, log: { _ in }).run()
        var result: [String: Any] = ["completed": report.completed, "not_run": report.notRun]
        switch report.outcome {
        case .finished: result["outcome"] = "finished"
        case .diverged(let step, let reason): result["outcome"] = "diverged"; result["step"] = step; result["reason"] = reason; result["interface_changed"] = report.interfaceChanged
        case .notApproved(let step): result["outcome"] = "not_approved"; result["step"] = step
        case .refused(let why): result["outcome"] = "refused"; result["reason"] = why
        case .stopped(let step, let actionDone): result["outcome"] = "stopped"; result["step"] = step; result["action_done"] = actionDone
        }
        return encode(["jsonrpc": "2.0", "id": id, "result": result])
    }

    /// Requests the executor sends to the shell, and the wait for each answer.
    final class Conversation {
        let input: () -> String?
        let output: (String) -> Void
        /// Set while a replay runs, so a stop that arrives while a question is open is seen.
        var control: RunControl?
        private var n = 0
        init(input: @escaping () -> String?, output: @escaping (String) -> Void) { self.input = input; self.output = output }

        /// The answer's `result`, or nil for an error, end of input, or a malformed answer. Lines for other
        /// ids are not answers and are skipped.
        /// A stop ends the wait only when `cancellable` (the officer's question). An audit append is always
        /// waited for: the trail must hold the record, and a stop is then handled by the run, not mistaken
        /// for a failed write.
        func ask(_ method: String, _ params: [String: Any], cancellable: Bool = false) -> [String: Any]? {
            n += 1
            let id = "x\(n)"
            output(RPC.encode(["jsonrpc": "2.0", "id": id, "method": method, "params": params]))
            while let line = input() {
                // A stop, pause or resume sent while the officer is being asked. A stop is a no to the question.
                if control?.apply(line: line) == true {
                    if cancellable && control?.state == .stopped { return nil }
                    continue
                }
                guard let o = RPC.parse(line), o["id"] as? String == id, o["method"] == nil else { continue }
                return o["result"] as? [String: Any]
            }
            return nil
        }
    }

    final class RPCApprovalSource: ApprovalSource {
        let talk: Conversation
        init(talk: Conversation) { self.talk = talk }
        func nextAnswer() -> String? { nil }
        func answer(for card: ApprovalCard) -> String? {
            guard let r = talk.ask("approval.request", ["card": card.json], cancellable: true) else { return nil }
            // Only the JSON value true. A string, a number or a missing field is a no.
            if let a = r["approved"] as? NSNumber, CFGetTypeID(a) == CFBooleanGetTypeID(), a.boolValue { return "y" }
            return "n"
        }
    }

    final class RPCAudit: AuditSink {
        let talk: Conversation
        init(talk: Conversation) { self.talk = talk }
        func record(_ e: AuditEvent) -> Bool {
            var p: [String: Any] = ["event": e.kind.rawValue]
            if let s = e.step { p["step"] = s }
            // The shell refuses a field over 16 KiB; refuse here too, so a card too big to keep is never approved.
            if let s = e.shown { guard s.utf8.count <= 16 * 1024 else { return false }; p["shown"] = s }
            if let i = e.item { p["item"] = i }
            guard let r = talk.ask("audit.append", p), let seq = r["seq"] as? NSNumber, CFGetTypeID(seq) != CFBooleanGetTypeID() else { return false }
            return seq.int64Value > 0
        }
    }

    static func parse(_ line: String) -> [String: Any]? {
        (try? JSONSerialization.jsonObject(with: Data(line.utf8))) as? [String: Any]
    }

    private static func err(_ code: Int, _ message: String) -> [String: Any] { ["code": code, "message": message] }

    static func encode(_ obj: [String: Any]) -> String {
        let data = (try? JSONSerialization.data(withJSONObject: obj, options: [.sortedKeys])) ?? Data("{}".utf8)
        return String(decoding: data, as: UTF8.self)
    }
}
