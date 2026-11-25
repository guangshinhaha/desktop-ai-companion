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
    /// True only for a divergence that means the target's interface is not what was recorded: the target was
    /// not found or matched more than one element, or the step's own check never held. False for anything else
    /// (a dialog, a failed audit write, a closed window, a failed action), so a re-verification flag is never
    /// raised for a reason that is not the interface (#42).
    public var interfaceChanged: Bool = false
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
/// accessibility action, then wait for the step's checks. It never guesses between two matching elements.
/// Without a `recovery` source any surprise stops the run. With one (#34), a divergence asks for help within
/// strict bounds (see `recover`); recovery never approves, never acts on anything consequential, and never
/// runs for a step that needs approval once its action was taken.
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
    let recovery: RecoverySource?
    let recoveryPolicy: RecoveryPolicy

    public init(task: ReplayTask, inputs: RunInputs, host: ReplayHost, gate: ApprovalGate,
                ceiling: TimeInterval = Replayer.defaultCeiling, audit: AuditSink? = nil,
                control: RunControl? = nil, recovery: RecoverySource? = nil, recoveryPolicy: RecoveryPolicy = RecoveryPolicy(),
                progress: ((ReplayProgress) -> Void)? = nil, log: @escaping (String) -> Void) {
        self.task = task; self.inputs = inputs; self.host = host; self.gate = gate
        self.ceiling = ceiling; self.audit = audit; self.control = control; self.progress = progress; self.log = log
        self.recovery = recovery; self.recoveryPolicy = recoveryPolicy
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
        func report(_ o: ReplayReport.Outcome, at index: Int, interfaceChanged: Bool = false) -> ReplayReport {
            ReplayReport(outcome: o, completed: done, notRun: task.steps[index...].map(\.id), interfaceChanged: interfaceChanged)
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

            let hold: () -> Void = { self.control?.holdWhilePaused(onPause: { self.progress?(here(.paused)) }, onResume: { self.progress?(here(.resumed)) }) }

            // 1. Find the element. Two matches is a divergence, never a guess. None, or a dialog in the way,
            // may be recovered: nothing has been done for this step yet.
            var match: Match
            switch findTarget(step, ceiling: stepCeiling) {
            case .success(let m): match = m
            case .failure(let d):
                if d.stopped { return stopReport(at: i) }
                guard recovery != nil, !d.reason.hasPrefix(Self.ambiguousPrefix) else {
                    return report(.diverged(step: step.id, reason: d.reason), at: i, interfaceChanged: d.interface)
                }
                switch recover(step, divergence: d.reason, hold: hold, resume: { self.findTarget(step, ceiling: stepCeiling) }) {
                case .recovered(let m): match = m
                case .stopped: return stopReport(at: i)
                case .gaveUp(let why): return report(.diverged(step: step.id, reason: why), at: i, interfaceChanged: d.interface)
                }
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
                if let (why, interface) = awaitChecks(step, ceiling: stepCeiling, gated: gated) {
                    if stopAsked { return stopReport(at: i, actionDone: true) }
                    // A gated step's action may have taken effect: it is never recovered or retried. The officer looks.
                    guard recovery != nil, !gated else {
                        return report(.diverged(step: step.id, reason: why), at: i, interfaceChanged: interface)
                    }
                    // Recovery waits for the checks again; it never does the step's action a second time.
                    let recheck: () -> Swift.Result<Void, Divergence> = {
                        guard let (w, iface) = self.awaitChecks(step, ceiling: stepCeiling, gated: false) else { return .success(()) }
                        return .failure(Divergence(reason: w, stopped: self.stopAsked, interface: iface))
                    }
                    switch recover(step, divergence: why, hold: hold, resume: recheck) {
                    case .recovered: break
                    case .stopped: return stopReport(at: i, actionDone: true)
                    case .gaveUp(let w): return report(.diverged(step: step.id, reason: w), at: i, interfaceChanged: interface)
                    }
                }
            }
            done.append(step.id)
            progress?(here(.done))
            log("  done: \(ApprovalGate.safe(step.id))")
        }
        return ReplayReport(outcome: .finished, completed: done, notRun: [])
    }

    // MARK: recovery

    enum RecoveryEnd<T> { case recovered(T), stopped, gaveUp(String) }

    /// Up to three attempts on one step (TD 5.4), then the officer. Each attempt: look at the window, build the
    /// request (the one builder), ask, check the answer, record it, and only then act. A recovery act is written
    /// to the trail as `recovered` before it is done; a refused answer as `step_diverged`. A stop is honoured
    /// before asking, after the answer, and before acting; a pause holds before each attempt.
    func recover<T>(_ step: ReplayTask.Step, divergence first: String, hold: () -> Void,
                    resume: () -> Swift.Result<T, Divergence>) -> RecoveryEnd<T> {
        guard let recovery else { return .gaveUp(first) }
        let officer = "The officer is asked to look."
        var last = first
        let max = RecoveryPolicy.maxAttempts
        for attempt in 1...max {
            hold()
            if stopAsked { return .stopped }
            guard let root = host.snapshot() else { return .gaveUp("\(last). Recovery could not see the window. \(officer)") }
            let built: RecoveryRequest.Built
            switch RecoveryRequest.build(step: step, task: task, inputs: inputs, root: root, divergence: ApprovalGate.safe(last),
                                         attempt: attempt, policy: recoveryPolicy, readValue: { self.host.value(of: $0) }) {
            case .failure(let r): return .gaveUp("\(first). Recovery was not asked: \(r.reason). \(officer)")
            case .success(let b): built = b
            }
            log("  recovery: asking (attempt \(attempt) of \(max))")
            let reply = recovery.ask(built.json)
            if stopAsked { return .stopped }
            guard let reply else { return .gaveUp("\(last). Recovery gave no answer. \(officer)") }
            let head = "Recovery at step \(step.id) (attempt \(attempt) of \(max))"
            func refused(_ why: String) -> String? {
                // The refusal is a record too. No record, no further attempt.
                if let audit, !audit.record(AuditEvent(.stepDiverged, step: step.id, shown: ApprovalGate.safe("\(head): the answer was refused: \(why). Nothing was done."))) {
                    return "\(first). The audit trail could not be written; nothing was done. \(officer)"
                }
                return nil
            }
            func written(_ shown: String) -> Bool {
                guard let audit else { return true }
                return audit.record(AuditEvent(.recovered, step: step.id, shown: ApprovalGate.safe(shown)))
            }
            switch RecoveryAnswer.parse(reply, step: step, task: task) {
            case .failure(let rej):
                if let stop = refused(rej.reason) { return .gaveUp(stop) }
                if rej.malformed { return .gaveUp("\(last). Recovery's answer was not usable (\(rej.reason)). \(officer)") }
                last = "recovery's answer was refused: \(rej.reason)"
                continue
            case .success(.askOfficer(let why)):
                return .gaveUp("\(last). Recovery asked for the officer: \(why)")
            case .success(.retry):
                guard written("\(head): wait for the step again. Why: \(last).") else {
                    return .gaveUp("\(first). The audit trail could not be written; nothing was done. \(officer)")
                }
            case .success(.act(let kind, let ref)):
                guard let now = host.snapshot() else { return .gaveUp("\(last). Recovery could not see the window. \(officer)") }
                let node: UINode
                switch RecoveryAnswer.check(kind, ref: ref, built: built, step: step, task: task, now: now) {
                case .failure(let rej):
                    if let stop = refused(rej.reason) { return .gaveUp(stop) }
                    last = "recovery's answer was refused: \(rej.reason)"
                    continue
                case .success(let n): node = n
                }
                let what = kind == .press ? "press" : "type the step's own input into"
                guard written("\(head): \(what) \(node.role) \"\(node.label ?? "")\". Why: \(last).") else {
                    return .gaveUp("\(first). The audit trail could not be written; nothing was done. \(officer)")
                }
                if stopAsked { return .stopped }
                let result: ActionResult
                switch kind {
                case .press: result = host.press(node)
                case .setValue:
                    guard let ref = step.value, let v = inputs.resolve(ref) else { return .gaveUp("\(first). No value for this step. \(officer)") }
                    result = host.setValue(node, to: v.reveal())
                }
                if case .failed(let why) = result { return .gaveUp("\(first). Recovery's action failed (\(why)). \(officer)") }
            }
            switch resume() {
            case .success(let t):
                log("  recovered")
                return .recovered(t)
            case .failure(let d):
                if d.stopped || stopAsked { return .stopped }
                if d.reason.hasPrefix(Self.ambiguousPrefix) { return .gaveUp(d.reason) }
                last = d.reason
            }
        }
        return .gaveUp("\(last). Recovery was tried \(max) times on this step. \(officer)")
    }

    // MARK: finding

    struct Divergence: Error { let reason: String; var stopped = false; var interface = false }
    static let ambiguousPrefix = "ambiguous: "

    func findTarget(_ step: ReplayTask.Step, ceiling: TimeInterval) -> Swift.Result<Match, Divergence> {
        var sawWindow = false // no window at all is not an interface change: the officer closed it, or the app quit
        let r: Waiting.Result<Match> = Waiting.until(ceiling: ceiling, wait: host.wait, now: host.now, cancelled: cancelled) {
            guard let root = host.snapshot() else { sawWindow = false; return .keepWaiting }
            sawWindow = true
            if let d = unexpectedDialog(in: root, step: step) { return .stop("\(d) is open before the step; nothing was clicked") }
            switch Resolver.resolve(step.target, in: root) {
            case .found(let m): return .done(m)
            case .ambiguous(let a): return .stop("\(Self.ambiguousPrefix)\(a.candidates.count) elements match the target equally (\(a.locator.summary))")
            case .notFound: return .keepWaiting
            }
        }
        switch r {
        case .done(let m): return .success(m)
        case .ceiling: return .failure(Divergence(reason: "element not found within \(Int(ceiling)) s", interface: sawWindow))
        case .stopped(let why): return .failure(Divergence(reason: why, interface: why.hasPrefix(Self.ambiguousPrefix)))
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
    func awaitChecks(_ step: ReplayTask.Step, ceiling: TimeInterval, gated: Bool) -> (String, Bool)? {
        var last = "no window"
        var sawWindow = false // a window that is gone is not an interface change
        let r: Waiting.Result<Void> = Waiting.until(ceiling: ceiling, wait: host.wait, now: host.now, cancelled: cancelled) {
            guard let root = host.snapshot() else { sawWindow = false; return .keepWaiting }
            sawWindow = true
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
            return ("check not met within \(Int(ceiling)) s: \(last)." + tail, sawWindow)
        case .stopped(let why): return (why, false)
        case .cancelled: return ("stopped by the officer", false)
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
