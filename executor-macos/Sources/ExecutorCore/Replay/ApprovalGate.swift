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

public final class ApprovalGate {
    let source: ApprovalSource
    let out: (String) -> Void

    public init(source: ApprovalSource, out: @escaping (String) -> Void) {
        self.source = source
        self.out = out
    }

    /// Shows the card for `step`, filled with this run's inputs, and waits for the answer.
    /// Only `y` or `Y` approves. `n`, anything else, an empty line and end of input all reject.
    public func request(step: ReplayTask.Step, task: ReplayTask, inputs: RunInputs, target: UINode) -> ApprovalOutcome {
        out(Self.card(step: step, task: task, inputs: inputs, target: target))
        let answer = source.nextAnswer()
        let decision = Self.isYes(answer)
        out(decision ? "approved: \(step.id)" : "not approved: \(step.id)")
        return decision ? .approved(Approval(stepId: step.id)) : .rejected
    }

    static func isYes(_ answer: String?) -> Bool {
        guard let a = answer?.trimmingCharacters(in: .whitespaces) else { return false }
        return a == "y" || a == "Y"
    }

    /// The text the officer sees. The only place a run's input values are printed. Everything that comes
    /// from the task file or the page goes through `safe`, and "Will do" is built from the step and the
    /// element found on screen, not from the task's own words, so a summary cannot hide the action.
    static func card(step: ReplayTask.Step, task: ReplayTask, inputs: RunInputs, target: UINode) -> String {
        var lines = ["", "APPROVAL NEEDED: step \(safe(step.id)) in \(safe(task.bundle))"]
        if step.consequential == .unknown { lines.append("  (the recording could not tell whether this step commits, so it is treated as one that does)") }
        let said = (step.approvalSummary?.isEmpty ?? true) ? "(the task gives no summary for this step)" : fill(step.approvalSummary ?? "", inputs: inputs)
        lines.append("  Task says: " + said)
        let what = "the \(safe(target.role)) \"\(clip(safe(target.label ?? "no label")))\""
        switch step.action {
        case .press: lines.append("  Will do: press \(what)")
        case .setValue: lines.append("  Will do: type \(valueName(step)) into \(what)")
        case .select: lines.append("  Will do: choose \(valueName(step)) in \(what)")
        }
        lines.append("  Can be undone: \(step.undoable ? "yes" : "no")")
        lines.append("  Values this run enters:")
        for spec in task.inputs {
            let v = inputs.secret(spec.name)?.reveal() ?? ""
            lines.append("    \(safe(spec.name)) = \"\(safe(v))\"")
        }
        lines.append("Type y to approve, n to stop. Anything else stops the run: ")
        return lines.joined(separator: "\n")
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
    static func safe(_ s: String) -> String {
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
