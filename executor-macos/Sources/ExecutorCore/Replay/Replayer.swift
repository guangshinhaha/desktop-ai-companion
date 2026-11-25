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
        /// The officer stopped the run. Nothing was done after the stop. `actionDone` is true when the step
        /// had been acted on but its checks were not confirmed yet: look at the app for that one.
        case stopped(step: String, actionDone: Bool)
    }
    public var outcome: Outcome
    public var completed: [String]
    public var notRun: [String]
}

/// One line of the live progress view (PRD C-3). Names and the task's own `say` text only, never run values.
public struct ReplayProgress: Equatable {
    public enum Kind: String { case started, done, paused, resumed }
    public var kind: Kind
    public var step: String
    public var say: String
    /// 1-based position of the step, and the number of steps.
    public var index: Int
    public var total: Int
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
    let audit: AuditSink?
    let control: RunControl?
    let progress: ((ReplayProgress) -> Void)?

    public init(task: ReplayTask, inputs: RunInputs, host: ReplayHost, gate: ApprovalGate,
                ceiling: TimeInterval = Replayer.defaultCeiling, audit: AuditSink? = nil,
                control: RunControl? = nil, progress: ((ReplayProgress) -> Void)? = nil, log: @escaping (String) -> Void) {
        self.task = task; self.inputs = inputs; self.host = host; self.gate = gate
        self.ceiling = ceiling; self.audit = audit; self.control = control; self.progress = progress; self.log = log
    }

    /// The step whose approval is in the trail but whose action has not been taken. A stop in that gap says so.
    private var approvedNotActed: String?
    private var cancelled: (() -> Bool)? { control.map { c in { c.isStopped } } }
    private var stopAsked: Bool { control?.isStopped ?? false }

    /// Runs the task and records it (TD 7.2): `run_started` first, then per step what the officer decided,
    /// then `step_diverged` if it stopped and `run_finished` always. When a record cannot be written the run
    /// stops, so nothing is done that the trail does not know about.
    public func run() -> ReplayReport {
        if let audit, !audit.record(AuditEvent(.runStarted)) {
            return ReplayReport(outcome: .refused("the audit trail could not be written; nothing was done"),
                                completed: [], notRun: task.steps.map(\.id))
        }
        let report = runSteps()
        if let audit {
            var outcome = "finished"
            switch report.outcome {
            case .diverged(let step, let reason):
                // Reasons name steps and elements, never run values (see the report type).
                _ = audit.record(AuditEvent(.stepDiverged, step: step, shown: ApprovalGate.safe(reason)))
                outcome = "diverged"
            case .stopped(let step, let actionDone):
                // Step ids and counts only. This is the record of the stop (PRD C-4).
                func list(_ ids: [String]) -> String { ids.isEmpty ? "none" : ids.joined(separator: ", ") }
                var shown = "Stopped by the officer at step \(step). Completed: \(list(report.completed)). Not run: \(list(report.notRun))."
                if let s = approvedNotActed { shown += " The approval for step \(s) was recorded and its action was not taken." }
                if actionDone { shown += " The step's action was done and its check was not confirmed." }
                _ = audit.record(AuditEvent(.runStopped, step: step, shown: ApprovalGate.safe(shown)))
                outcome = "stopped"
            case .notApproved: outcome = "not_approved"
            case .refused: outcome = "refused"
            case .finished: break
            }
            _ = audit.record(AuditEvent(.runFinished, shown: outcome))
        }
        return report
    }

    private func runSteps() -> ReplayReport {
        var done: [String] = []
        func report(_ o: ReplayReport.Outcome, at index: Int) -> ReplayReport {
            ReplayReport(outcome: o, completed: done, notRun: task.steps[index...].map(\.id))
        }

        func stopReport(at index: Int, actionDone: Bool = false) -> ReplayReport {
            ReplayReport(outcome: .stopped(step: task.steps[index].id, actionDone: actionDone),
                         completed: done, notRun: task.steps[index...].map(\.id))
        }

        for pre in task.preconditions {
            if let why = awaitCheck(pre, step: nil, ceiling: ceiling) {
                if stopAsked { return stopReport(at: 0) }
                return report(.refused("precondition not met: \(why)"), at: 0)
            }
        }
        log("precondition(s) met: \(task.preconditions.count)")

        for (i, step) in task.steps.enumerated() {
            let here = { (k: ReplayProgress.Kind) in
                ReplayProgress(kind: k, step: step.id, say: ApprovalGate.safe(step.say), index: i + 1, total: self.task.steps.count)
            }
            // Pause holds the run here, between steps. A stop is checked after, and again before every action.
            control?.holdWhilePaused(onPause: { progress?(here(.paused)) }, onResume: { progress?(here(.resumed)) })
            if stopAsked { return stopReport(at: i) }
            progress?(here(.started))
            log("step \(ApprovalGate.safe(step.id)): \(ApprovalGate.safe(step.say))")
            // The run's ceiling caps a step's own timeout; a task cannot lengthen the run.
            let stepCeiling = min(step.timeoutSeconds ?? ceiling, ceiling)

            // 1. Find the element. Two matches or none is a divergence, never a guess.
            var match: Match
            switch findTarget(step, ceiling: stepCeiling) {
            case .success(let m): match = m
            case .failure(let d):
                if d.stopped { return stopReport(at: i) }
                return report(.diverged(step: step.id, reason: d.reason), at: i)
            }
            log("  found: \(ApprovalGate.safe(match.locator.summary))")

            // 2. The gate. Nothing below runs for a step that needs approval unless the officer said y.
            var approval: Approval?
            let gated = Self.mustAsk(step, live: match.node)
            if gated {
                let shown = ApprovalGate.makeCard(step: step, task: task, inputs: inputs, target: match.node).shown
                switch gate.request(step: step, task: task, inputs: inputs, target: match.node) {
                case .rejected:
                    // A stop while the card is open answers it no; the stop is what is recorded.
                    if stopAsked { return stopReport(at: i) }
                    _ = audit?.record(AuditEvent(.rejected, step: step.id, shown: shown))
                    return report(.notApproved(step: step.id), at: i)
                case .approved(let a):
                    // A pause pressed while the card was open holds here, before the approval is recorded,
                    // so nothing the officer approved is acted on while the run is paused.
                    control?.holdWhilePaused(onPause: { progress?(here(.paused)) }, onResume: { progress?(here(.resumed)) })
                    if stopAsked { return stopReport(at: i) }
                    // The record comes before the action. No record, no action.
                    if let audit, !audit.record(AuditEvent(.approved, step: step.id, shown: shown)) {
                        return report(.diverged(step: step.id, reason: "the audit trail could not be written; nothing was done for this step"), at: i)
                    }
                    approvedNotActed = step.id
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
                case .found(let m) where m.node.role == match.node.role && m.node.label == match.node.label
                                         && Self.mustAsk(step, live: m.node):
                    match = m
                default:
                    return report(.diverged(step: step.id, reason: "the element changed or went away after approval; nothing was done"), at: i)
                }
            }

            // 3. Act. Last look at the stop: after this, nothing is done for a step the officer stopped.
            if stopAsked { return stopReport(at: i) }
            approvedNotActed = nil
            let acted = perform(step, on: match.node, approval: approval)
            // 4. Wait for the checks.
            switch acted {
            case .failed(let why):
                // A failed action stops the run even if a check already passes: that check never proved the action happened.
                let tail = gated ? " The step may or may not have taken effect: check the app before running again." : ""
                return report(.diverged(step: step.id, reason: "the action failed (\(why)).\(tail)"), at: i)
            case .ok:
                if let why = awaitChecks(step, ceiling: stepCeiling, gated: gated) {
                    if stopAsked { return stopReport(at: i, actionDone: true) }
                    return report(.diverged(step: step.id, reason: why), at: i)
                }
            }
            done.append(step.id)
            progress?(here(.done))
            log("  done: \(ApprovalGate.safe(step.id))")
        }
        return ReplayReport(outcome: .finished, completed: done, notRun: [])
    }

    // MARK: finding

    struct Divergence: Error { let reason: String; var stopped = false }

    func findTarget(_ step: ReplayTask.Step, ceiling: TimeInterval) -> Swift.Result<Match, Divergence> {
        let r: Waiting.Result<Match> = Waiting.until(ceiling: ceiling, wait: host.wait, now: host.now, cancelled: cancelled) {
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
        case .cancelled: return .failure(Divergence(reason: "stopped by the officer", stopped: true))
        }
    }

    // MARK: acting

    /// The task's own rule, plus what is actually on screen: a commit-looking label, or a set_value or
    /// select that landed on something that is not a field. The descriptor may carry no label at all.
    static func mustAsk(_ step: ReplayTask.Step, live node: UINode) -> Bool {
        if step.needsApproval { return true }
        if node.label.map(ReplayTask.looksLikeCommit) ?? false { return true }
        switch step.action {
        case .press: return true
        case .setValue: return !ReplayTask.textRoles.contains(node.role)
        case .select: return !ReplayTask.choiceRoles.contains(node.role)
        }
    }

    /// The only place that acts. A step that needs approval must arrive with the approval for that very step.
    func perform(_ step: ReplayTask.Step, on node: UINode, approval: Approval?) -> ActionResult {
        if Self.mustAsk(step, live: node), approval?.stepId != step.id {
            return .failed("no approval for this step")
        }
        switch step.action {
        case .press:
            return host.press(node)
        case .setValue, .select:
            guard let ref = step.value, let v = inputs.resolve(ref) else { return .failed("no value for this step") }
            if step.action == .setValue { return host.setValue(node, to: v.reveal()) }
            // Opening a pop-up and pressing the option it already shows leaves Chrome's select with nothing
            // chosen (issue #70: the first option, which is selected from page load). Nothing to do then;
            // the step's value_equals check still reads the field and decides.
            if host.value(of: node) == v.reveal() { return .ok }
            return host.choose(node, option: v.reveal())
        }
    }

    // MARK: checks

    /// Waits until every check of the step passes. Returns nil on success, or why not.
    func awaitChecks(_ step: ReplayTask.Step, ceiling: TimeInterval, gated: Bool) -> String? {
        var last = "no window"
        let r: Waiting.Result<Void> = Waiting.until(ceiling: ceiling, wait: host.wait, now: host.now, cancelled: cancelled) {
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
            let tail = gated ? " The step may or may not have taken effect: check the app before running again." : ""
            return "check not met within \(Int(ceiling)) s: \(last)." + tail
        case .stopped(let why): return why
        case .cancelled: return "stopped by the officer"
        }
    }

    func awaitCheck(_ c: ReplayTask.Check, step: ReplayTask.Step?, ceiling: TimeInterval) -> String? {
        var last = "no window"
        let r: Waiting.Result<Void> = Waiting.until(ceiling: ceiling, wait: host.wait, now: host.now, cancelled: cancelled) {
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

    /// A dialog on screen that the step did not expect. Described by role only, never by its text.
    func unexpectedDialog(in root: UINode, step: ReplayTask.Step) -> String? {
        let expected = step.expect.contains { c in
            c.target.map { $0.role.map(Self.dialogRoles.contains) ?? false } ?? false
        }
        if expected { return nil }
        for n in [root] + root.descendants where Self.isDialog(n) {
            // Never quote the dialog's text: it can hold a value from this run (a name, an email).
            return "a dialog"
        }
        return nil
    }

    static func isDialog(_ n: UINode) -> Bool {
        dialogRoles.contains(n.role) || (n.subrole.map { $0 == "AXDialog" || $0 == "AXApplicationDialog" || $0 == "AXSystemDialog" } ?? false)
    }
}
