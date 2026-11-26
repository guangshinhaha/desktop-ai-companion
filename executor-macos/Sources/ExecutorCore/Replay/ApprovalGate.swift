import Foundation

/// Proof that the officer approved one step in this run (TD section 7.1, PRD E-4).
/// The initialiser is `fileprivate`: nothing outside this file can make one, so the code that acts on
/// a step that needs approval can only be called with a value that came out of `ApprovalGate.request`.
public struct Approval {
    public let stepId: String
    fileprivate init(stepId: String) { self.stepId = stepId }
}

/// Where the officer's answer comes from. There are two sources and no third:
/// a terminal, or an explicit approval file. Piped standard input, an environment variable, a flag
/// that says yes and a default are all absent on purpose.
public protocol ApprovalSource {
    /// One answer line for the approval just shown, or nil when there is none (end of input,
    /// no terminal, no such line). The caller counts nil as no.
    func nextAnswer() -> String?
    /// Same, for a source that shows the card itself (the shell). The default is `nextAnswer`.
    func answer(for card: ApprovalCard) -> String?
}

public extension ApprovalSource {
    func answer(for card: ApprovalCard) -> String? { nextAnswer() }
}

/// Reads from the terminal the process is attached to. Not a terminal means no answer, so
/// `yes | executor-macos replay ...` and a closed stdin both stop at the first approval.
public final class TerminalApprovalSource: ApprovalSource {
    private let fd: Int32
    public init(fd: Int32 = STDIN_FILENO) { self.fd = fd }
    public func nextAnswer() -> String? {
        guard isatty(fd) == 1 else { return nil }
        // Keys typed before the question appeared were not an answer to it.
        tcflush(fd, TCIFLUSH)
        return readLine(strippingNewline: true)
    }
}

/// `--approval-file <path>`: line N answers the Nth approval of the run. The file is read when each
/// approval is asked, so a person can write the line after seeing the question. It must be a plain file
/// that only its owner can write, so another user or a symlink cannot answer for the officer.
public final class FileApprovalSource: ApprovalSource {
    private let path: String
    private var asked = 0
    public init(path: String) { self.path = path }

    public func nextAnswer() -> String? {
        asked += 1
        var st = stat()
        guard lstat(path, &st) == 0, (st.st_mode & S_IFMT) == S_IFREG, st.st_uid == getuid(),
              st.st_mode & (S_IWGRP | S_IWOTH) == 0,
              let data = FileManager.default.contents(atPath: path), data.count <= 4096,
              let text = String(data: data, encoding: .utf8) else { return nil }
        // Only a line ended by a newline is an answer. The last piece, after the final newline, is a
        // line still being typed (or nothing) and never counts.
        var lines = text.split(separator: "\n", omittingEmptySubsequences: false).map(String.init)
        lines.removeLast()
        return asked <= lines.count ? lines[asked - 1] : nil
    }
}

public enum ApprovalOutcome {
    case approved(Approval)
    /// Anything but a plain y. The run stops.
    case rejected
}

/// What the officer is shown for one step (TD 7.1). Built from the step and the element found on screen,
/// never from the task's own words alone, so a summary cannot hide the action.
public struct ApprovalCard {
    public struct Pair { public var name: String; public var value: String }

    public var stepId: String
    public var task: String
    public var app: String
    /// The task's summary with this run's inputs filled in, or nil when the task gives none.
    public var summary: String?
    public var willDo: String
    public var unknown: Bool
    /// What the task claims. The executor cannot check it.
    public var undoableClaim: Bool
    public var inputs: [Pair]
    /// Fixed values the task types, each with the field it goes into (issue #76).
    public var constants: [Pair]

    /// The card as a JSON object for the shell. Holds values: it goes to the officer's own screen only.
    public var json: [String: Any] {
        func pairs(_ p: [Pair]) -> [[String: String]] { p.map { ["name": $0.name, "value": $0.value] } }
        var o: [String: Any] = ["step": stepId, "task": task, "app": app, "will_do": willDo, "unknown": unknown,
                                "undoable_claim": undoableClaim, "inputs": pairs(inputs), "constants": pairs(constants)]
        if let summary { o["summary"] = summary }
        return o
    }

    /// The lines of the card. `showValues` false hides this run's input values (a pipe, a log).
    public func lines(showValues: Bool) -> [String] {
        var l = ["", "APPROVAL NEEDED: step \(stepId) in \(app)"]
        if unknown { l.append("  (the recording could not tell whether this step commits, so it is treated as one that does)") }
        let said = summary.map { showValues ? $0 : "(hidden: output is not a terminal)" } ?? "(the task gives no summary for this step)"
        l.append("  Task says: " + said)
        l.append("  Will do: \(willDo)")
        l.append("  Task says it can be undone: \(undoableClaim ? "yes" : "no")")
        l.append("  Values this run enters:")
        for p in inputs { l.append("    \(p.name) = \"\(showValues ? p.value : "<hidden>")\"") }
        if !constants.isEmpty {
            l.append("  Fixed values the task types:")
            for p in constants { l.append("    into \(p.name): \"\(p.value)\"") }
        }
        return l
    }

    /// The card with its values, sent with the audit event. The shell stores its own copy of the card (#107).
    public var shown: String { lines(showValues: true).filter { !$0.isEmpty }.joined(separator: "\n") }
}

public final class ApprovalGate {
    let source: ApprovalSource
    let out: (String) -> Void
    let showValues: Bool

    /// `showValues` false (standard output is a pipe or a file) prints the card without this run's values.
    public init(source: ApprovalSource, out: @escaping (String) -> Void, showValues: Bool = true) {
        self.source = source
        self.out = out
        self.showValues = showValues
    }

    /// Shows the card for `step`, filled with this run's inputs, and waits for the answer.
    /// Only `y` or `Y` approves. `n`, anything else, an empty line and end of input all reject.
    public func request(step: ReplayTask.Step, task: ReplayTask, inputs: RunInputs, target: UINode) -> ApprovalOutcome {
        let c = Self.makeCard(step: step, task: task, inputs: inputs, target: target)
        out((c.lines(showValues: showValues) + ["Type y to approve, n to stop. Anything else stops the run: "]).joined(separator: "\n"))
        let decision = Self.isYes(source.answer(for: c))
        out(decision ? "approved: \(Self.safe(step.id))" : "not approved: \(Self.safe(step.id))")
        return decision ? .approved(Approval(stepId: step.id)) : .rejected
    }

    static func isYes(_ answer: String?) -> Bool {
        guard let a = answer?.trimmingCharacters(in: .whitespaces) else { return false }
        return a == "y" || a == "Y"
    }

    /// The text the officer sees, with values. Everything that comes from the task file or the page goes
    /// through `safe`, and "Will do" is built from the step and the element found on screen.
    static func card(step: ReplayTask.Step, task: ReplayTask, inputs: RunInputs, target: UINode) -> String {
        (makeCard(step: step, task: task, inputs: inputs, target: target).lines(showValues: true)
         + ["Type y to approve, n to stop. Anything else stops the run: "]).joined(separator: "\n")
    }

    public static func makeCard(step: ReplayTask.Step, task: ReplayTask, inputs: RunInputs, target: UINode) -> ApprovalCard {
        let what = "the \(safe(target.role)) \"\(clip(safe(target.label ?? "no label")))\""
        let willDo: String
        switch step.action {
        case .press: willDo = "press \(what)"
        case .setValue: willDo = "type \(valueName(step)) into \(what)"
        case .select: willDo = "choose \(valueName(step)) in \(what)"
        }
        let said = (step.approvalSummary?.isEmpty ?? true) ? nil : fill(step.approvalSummary ?? "", inputs: inputs)
        let ins = task.inputs.map { ApprovalCard.Pair(name: safe($0.name), value: safe(inputs.secret($0.name)?.reveal() ?? "")) }
        var consts: [ApprovalCard.Pair] = []
        for s in task.steps {
            if case .constant(let c)? = s.value {
                let field = s.target.label ?? s.target.labelMatches ?? s.target.domId ?? "step \(s.id)"
                consts.append(ApprovalCard.Pair(name: clip(safe(field)), value: safe(c)))
            }
        }
        return ApprovalCard(stepId: safe(step.id), task: safe(task.name), app: safe(task.bundle), summary: said, willDo: willDo,
                            unknown: step.consequential == .unknown, undoableClaim: step.undoable, inputs: ins, constants: consts)
    }

    static func valueName(_ step: ReplayTask.Step) -> String {
        switch step.value {
        case .input(let n)?: return "input \(safe(n))"
        case .constant?: return "a fixed value"
        case nil: return "a value"
        }
    }

    static func clip(_ s: String) -> String { s.count > 80 ? String(s.prefix(80)) + "..." : s }

    /// Replaces each `{name}` once. A value that itself holds `{other}` is not expanded again.
    static func fill(_ summary: String, inputs: RunInputs) -> String {
        var out = ""
        var rest = Substring(summary)
        while let open = rest.firstIndex(of: "{") {
            out += safe(String(rest[..<open]))
            guard let close = rest[open...].firstIndex(of: "}") else { rest = rest[open...]; break }
            let name = String(rest[rest.index(after: open)..<close])
            if let v = inputs.secret(name) { out += safe(v.reveal()) } else { out += "{\(safe(name))}" }
            rest = rest[rest.index(after: close)...]
        }
        return out + safe(String(rest))
    }

    /// Control, format (right-to-left override and the like), separator and private-use characters
    /// show as `\u{XXXX}`, so a value cannot move the cursor, clear the line or reorder the text
    /// to make the approval say something else.
    public static func safe(_ s: String) -> String {
        var out = ""
        for u in s.unicodeScalars {
            switch u.properties.generalCategory {
            case .control, .format, .lineSeparator, .paragraphSeparator, .privateUse, .surrogate, .unassigned:
                out += "\\u{" + String(u.value, radix: 16, uppercase: true) + "}"
            default:
                out.unicodeScalars.append(u)
            }
        }
        return out
    }
}
