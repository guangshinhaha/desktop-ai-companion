import ExecutorCore
import Foundation

// executor-macos replay <task.json> --inputs <file> [--pid <n>] [--approval-file <file>] [--ceiling <seconds>]
// Exit codes: 0 finished, 1 usage or error, 2 diverged or refused, 4 not approved.

let replayUsage = """
usage: executor-macos replay <task.json> --inputs <file> [--pid <n>] [--approval-file <file>] [--ceiling <seconds>]
  --inputs         JSON object of input name to value for this run (read once, never written anywhere)
  --pid            the one browser process to drive. Without it, more than one candidate stops the run as ambiguous
  --approval-file  (test builds only; a release build refuses it) answer approvals from this file, one line per approval, instead of the terminal.
                   Plain file, yours, not writable by others. There is no flag that approves
  --ceiling        seconds a step may wait for its element and its checks (default 30)
Before any step that may commit, the run prints what it will do and waits for y. Anything else stops it.
"""

func replayFail(_ msg: String, code: Int32 = 1) -> Never {
    FileHandle.standardError.write(Data("error: \(msg)\n".utf8))
    exit(code)
}

/// Everything printed passes through here: control and format characters other than the line break
/// are shown as \u{...}, so task text cannot move the cursor or rewrite the card (issue #76).
func say(_ s: String) {
    let clean = s.split(separator: "\n", omittingEmptySubsequences: false).map { ApprovalGate.safe(String($0)) }.joined(separator: "\n")
    FileHandle.standardOutput.write(Data((clean + "\n").utf8))
}

func runReplay(_ args: [String]) -> Never {
    var taskPath: String?
    var inputsPath: String?
    var approvalPath: String?
    var pid: pid_t?
    var ceiling = Replayer.defaultCeiling
    var i = 0
    func value(_ flag: String) -> String {
        i += 1
        guard i < args.count, !args[i].hasPrefix("--") else { replayFail("\(flag) needs a value\n\(replayUsage)") }
        return args[i]
    }
    while i < args.count {
        switch args[i] {
        case "--inputs": inputsPath = value("--inputs")
        case "--approval-file": approvalPath = value("--approval-file")
        case "--pid":
            guard let p = Int32(value("--pid")), p > 1 else { replayFail("--pid must be a process id") }
            pid = p
        case "--ceiling":
            guard let c = Double(value("--ceiling")), c > 0, c <= 3600 else {
                replayFail("--ceiling must be a number of seconds above 0 and at most 3600")
            }
            ceiling = c
        case let a where a.hasPrefix("--"): replayFail("unknown argument \(a)\n\(replayUsage)")
        default:
            guard taskPath == nil else { replayFail("one task file only\n\(replayUsage)") }
            taskPath = args[i]
        }
        i += 1
    }
    if approvalPath != nil, let why = ApprovalFilePolicy.refusal() { replayFail(why, code: 2) }
    guard let taskPath else { replayFail("a task file is required\n\(replayUsage)") }
    guard let inputsPath else { replayFail("--inputs is required\n\(replayUsage)") }

    // Everything that can be checked without touching the app is checked first.
    let task: ReplayTask
    do {
        guard let d = FileManager.default.contents(atPath: taskPath) else { replayFail("cannot read \(taskPath)") }
        task = try ReplayTask.load(from: d)
    } catch { replayFail("task not usable: \(error)") }
    let inputs: RunInputs
    do {
        guard let d = FileManager.default.contents(atPath: inputsPath) else { replayFail("cannot read \(inputsPath)") }
        inputs = try RunInputs.load(from: d, for: task)
    } catch { replayFail("inputs not usable: \(error)") }

    let source: ApprovalSource = approvalPath.map { FileApprovalSource(path: $0) } ?? TerminalApprovalSource()
    // Run values print only on a terminal; into a pipe or a file the card says they are hidden (issue #76).
    let gate = ApprovalGate(source: source, out: say, showValues: isatty(STDOUT_FILENO) == 1)

    let app: AXApp
    do { app = try AXApp(bundleId: task.bundle, pid: pid, requireUnique: true) } catch {
        let ambiguous: Bool
        if case AXApp.Failure.ambiguous = error { ambiguous = true } else { ambiguous = false }
        replayFail("\(error)", code: ambiguous ? 2 : 1)
    }

    say("replay: \(ApprovalGate.safe(task.name)) (\(task.steps.count) steps) in \(task.bundle) pid \(app.pid)")
    let report = Replayer(task: task, inputs: inputs, host: app, gate: gate, ceiling: ceiling, log: say).run()
    func list(_ ids: [String]) -> String { ids.isEmpty ? "none" : ids.joined(separator: ", ") }
    switch report.outcome {
    case .finished:
        say("run finished: all \(report.completed.count) steps done")
        exit(0)
    case .diverged(let step, let reason):
        say("run stopped: step \(step) diverged")
        say("  reason: \(reason)")
        say("  completed: \(list(report.completed))")
        say("  not run: \(list(report.notRun))")
        exit(2)
    case .notApproved(let step):
        say("run stopped: step \(step) was not approved, and nothing was done for it")
        say("  completed: \(list(report.completed))")
        say("  not run: \(list(report.notRun))")
        exit(4)
    case .stopped(let step, _):
        // The terminal command has no stop control; this is here so the switch stays complete.
        say("run stopped at step \(step)")
        say("  completed: \(list(report.completed))")
        say("  not run: \(list(report.notRun))")
        exit(2)
    case .refused(let why):
        say("run refused: \(why)")
        say("  completed: none")
        say("  not run: \(list(report.notRun))")
        exit(2)
    }
}
