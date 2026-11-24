import Foundation

public struct ReplayReport {
    public enum Outcome: Equatable {
        case finished
        /// The step could not be done or its check never passed. The reason holds names and labels, never values.
        case diverged(step: String, reason: String)
        /// The officer did not approve the step. Nothing was done for it.
        case notApproved(step: String)
        /// Refused before the first action (precondition, unsupported task).
        case refused(String)
    }
    public var outcome: Outcome
    public var completed: [String]
    public var notRun: [String]
}

/// Replays a task step by step (PRD E-1, E-4; TD 5.2, 5.3, 7.1).
/// For each step: find the element, ask the officer first if the step may commit, act through an
/// accessibility action, then wait for the step's checks. Any surprise stops the run. It never retries,
/// never clicks through an unexpected dialog and never guesses between two matching elements.
public final class Replayer {
    public static let defaultCeiling: TimeInterval = 30

    let task: ReplayTask
    let inputs: RunInputs
    let host: ReplayHost
    let gate: ApprovalGate
    let ceiling: TimeInterval
    let log: (String) -> Void

    public init(task: ReplayTask, inputs: RunInputs, host: ReplayHost, gate: ApprovalGate,
                ceiling: TimeInterval = Replayer.defaultCeiling, log: @escaping (String) -> Void) {
        self.task = task; self.inputs = inputs; self.host = host; self.gate = gate
        self.ceiling = ceiling; self.log = log
    }

    public func run() -> ReplayReport {
        var done: [String] = []
        func report(_ o: ReplayReport.Outcome, at index: Int) -> ReplayReport {
            ReplayReport(outcome: o, completed: done, notRun: task.steps[index...].map(\.id))
        }

        for pre in task.preconditions {
            if let why = awaitCheck(pre, step: nil, ceiling: ceiling) { return report(.refused("precondition not met: \(why)"), at: 0) }
        }
        log("precondition(s) met: \(task.preconditions.count)")

        for (i, step) in task.steps.enumerated() {
            log("step \(step.id): \(step.say)")
            let stepCeiling = step.timeoutSeconds ?? ceiling

            // 1. Find the element. Two matches or none is a divergence, never a guess.
            var match: Match
            switch findTarget(step, ceiling: stepCeiling) {
            case .success(let m): match = m
            case .failure(let d): return report(.diverged(step: step.id, reason: d.reason), at: i)
            }
            log("  found: \(match.locator.summary)")

            // 2. The gate. Nothing below runs for a step that needs approval unless the officer said y.
            var approval: Approval?
            if step.needsApproval {
                switch gate.request(step: step, task: task, inputs: inputs) {
                case .rejected:
                    return report(.notApproved(step: step.id), at: i)
                case .approved(let a):
                    approval = a
                }
                // The officer may have taken a while. Look again, once, and act only on what is there now.
                guard let snap = host.snapshot() else {
                    return report(.diverged(step: step.id, reason: "the window is gone after approval; nothing was done"), at: i)
                }
                if let d = unexpectedDialog(in: snap, step: step) {
                    return report(.diverged(step: step.id, reason: "\(d) appeared after approval; nothing was clicked"), at: i)
                }
                switch Resolver.resolve(step.target, in: snap) {
                case .found(let m) where m.node.role == match.node.role && m.node.label == match.node.label:
                    match = m
                default:
                    return report(.diverged(step: step.id, reason: "the element changed or went away after approval; nothing was done"), at: i)
                }
            }

            // 3. Act.
            let acted = perform(step, on: match.node, approval: approval)
            // 4. Wait for the checks.
            switch acted {
            case .failed(let why):
                // An action can report failure after taking effect (a press that navigates). Look once, without waiting.
                if case .none = checksFailure(step, root: host.snapshot()) {
                    log("  action reported \(why), but the checks pass")
                } else {
                    let tail = step.needsApproval ? " The step may or may not have taken effect: check the app before running again." : ""
                    return report(.diverged(step: step.id, reason: "the action failed (\(why)).\(tail)"), at: i)
                }
            case .ok:
                if let why = awaitChecks(step, ceiling: stepCeiling) {
                    return report(.diverged(step: step.id, reason: why), at: i)
                }
            }
            done.append(step.id)
            log("  done: \(step.id)")
        }
        return ReplayReport(outcome: .finished, completed: done, notRun: [])
    }

    // MARK: finding

    struct Divergence: Error { let reason: String }

    func findTarget(_ step: ReplayTask.Step, ceiling: TimeInterval) -> Swift.Result<Match, Divergence> {
        let r: Waiting.Result<Match> = Waiting.until(ceiling: ceiling, wait: host.wait, now: host.now) {
            guard let root = host.snapshot() else { return .keepWaiting }
            if let d = unexpectedDialog(in: root, step: step) { return .stop("\(d) is open before the step; nothing was clicked") }
            switch Resolver.resolve(step.target, in: root) {
            case .found(let m): return .done(m)
            case .ambiguous(let a): return .stop("ambiguous: \(a.candidates.count) elements match the target equally (\(a.locator.summary))")
            case .notFound: return .keepWaiting
            }
        }
        switch r {
        case .done(let m): return .success(m)
        case .ceiling: return .failure(Divergence(reason: "element not found within \(Int(ceiling)) s"))
        case .stopped(let why): return .failure(Divergence(reason: why))
        }
    }

    // MARK: acting

    /// The only place that acts. A step that needs approval must arrive with the approval for that very step.
    func perform(_ step: ReplayTask.Step, on node: UINode, approval: Approval?) -> ActionResult {
        if step.needsApproval, approval?.stepId != step.id {
            return .failed("no approval for this step")
        }
        switch step.action {
        case .press:
            return host.press(node)
        case .setValue, .select:
            guard let ref = step.value, let v = inputs.resolve(ref) else { return .failed("no value for this step") }
            return step.action == .setValue ? host.setValue(node, to: v.reveal()) : host.choose(node, option: v.reveal())
        }
    }

    // MARK: checks

    /// Waits until every check of the step passes. Returns nil on success, or why not.
    func awaitChecks(_ step: ReplayTask.Step, ceiling: TimeInterval) -> String? {
        var last = "no window"
        let r: Waiting.Result<Void> = Waiting.until(ceiling: ceiling, wait: host.wait, now: host.now) {
            guard let root = host.snapshot() else { return .keepWaiting }
            guard let failing = checksFailure(step, root: root) else { return .done(()) }
            // A passing check wins over a dialog that the step expects; otherwise a new dialog is a stop.
            if let d = unexpectedDialog(in: root, step: step) { return .stop("unexpected dialog: \(d) appeared and was not clicked. The step's action was done and the dialog is still open; recovery is Phase 1") }
            last = failing
            return .keepWaiting
        }
        switch r {
        case .done: return nil
        case .ceiling:
            let tail = step.needsApproval ? " The step may or may not have taken effect: check the app before running again." : ""
            return "check not met within \(Int(ceiling)) s: \(last)." + tail
        case .stopped(let why): return why
        }
    }

    func awaitCheck(_ c: ReplayTask.Check, step: ReplayTask.Step?, ceiling: TimeInterval) -> String? {
        var last = "no window"
        let r: Waiting.Result<Void> = Waiting.until(ceiling: ceiling, wait: host.wait, now: host.now) {
            guard let root = host.snapshot() else { return .keepWaiting }
            if let f = failure(of: c, step: step, root: root) { last = f; return .keepWaiting }
            return .done(())
        }
        if case .done = r { return nil }
        return last
    }

    /// nil when every check passes, else a description of the first one that does not.
    func checksFailure(_ step: ReplayTask.Step, root: UINode?) -> String? {
        guard let root else { return "no window" }
        for c in step.expect { if let f = failure(of: c, step: step, root: root) { return f } }
        return nil
    }

    func failure(of c: ReplayTask.Check, step: ReplayTask.Step?, root: UINode) -> String? {
        switch c.kind {
        case .exists, .appears:
            guard let t = c.target else { return "\(c.kind.rawValue) has no target" }
            if case .found = Resolver.resolve(t, in: root) { return nil }
            return "\(c.kind.rawValue) \(describe(t)) is not true yet"
        case .disappears:
            guard let t = c.target else { return "disappears has no target" }
            if case .notFound = Resolver.resolve(t, in: root) { return nil }
            return "disappears \(describe(t)) is not true yet"
        case .windowTitle:
            guard let m = c.matches else { return "window_title has no pattern" }
            if let title = root.label, Resolver.glob(m, title) { return nil }
            return "window title does not match \"\(m)\""
        case .valueEquals:
            guard let t = c.target ?? step?.target, let ref = c.value, let want = inputs.resolve(ref) else {
                return "value_equals has nothing to compare"
            }
            guard case .found(let m) = Resolver.resolve(t, in: root) else { return "value_equals: the field is not found" }
            guard let have = host.value(of: m.node) else { return "value_equals: the field value cannot be read" }
            // Neither side is printed: only the fact that they differ.
            return have == want.reveal() ? nil : "value_equals: the field does not hold the run's input"
        }
    }

    func describe(_ d: Descriptor) -> String {
        "role=\(d.role ?? "-") label=\(d.label.map { "\"\($0)\"" } ?? d.labelMatches.map { "~\"\($0)\"" } ?? "-")"
    }

    // MARK: dialogs

    static let dialogRoles: Set<String> = ["AXDialog", "AXSheet", "AXPopover"]

    /// A dialog on screen that the step did not expect. Described by role and, when it is plain fixed
    /// text, its title; never by anything that could be data.
    func unexpectedDialog(in root: UINode, step: ReplayTask.Step) -> String? {
        let expected = step.expect.contains { c in
            c.target.map { $0.role.map(Self.dialogRoles.contains) ?? false } ?? false
        }
        if expected { return nil }
        for n in [root] + root.descendants where Self.isDialog(n) {
            let title = ExpectProposer.stable(n.label) ?? n.descendants.lazy.compactMap { ExpectProposer.stable($0.role == "AXStaticText" ? $0.label : nil) }.first
            return "a dialog" + (title.map { " (\"\($0)\")" } ?? "")
        }
        return nil
    }

    static func isDialog(_ n: UINode) -> Bool {
        dialogRoles.contains(n.role) || (n.subrole.map { $0 == "AXDialog" || $0 == "AXApplicationDialog" || $0 == "AXSystemDialog" } ?? false)
    }
}
