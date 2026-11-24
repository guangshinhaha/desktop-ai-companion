import Foundation

/// The events of TD section 7.2. The shell appends them to the audit chain; the executor only names them.
public enum AuditEventKind: String, CaseIterable {
    case runStarted = "run_started"
    case approved
    case rejected
    case stepDiverged = "step_diverged"
    case recovered
    case runFinished = "run_finished"
    case itemMarkedByOfficer = "item_marked_by_officer"
    case taskSaved = "task_saved"
    case taskShared = "task_shared"
    case taskHandedOver = "task_handed_over"
}

/// One thing to record. `shown` holds what the officer was shown, with the values they approved
/// (decision 0005); it travels to the shell's encrypted store and nowhere else.
public struct AuditEvent {
    public var kind: AuditEventKind
    public var step: String?
    public var shown: String?
    public var item: Int?
    public init(_ kind: AuditEventKind, step: String? = nil, shown: String? = nil, item: Int? = nil) {
        self.kind = kind; self.step = step; self.shown = shown; self.item = item
    }
}

/// Where audit events go. `record` returns false when the record could not be written. The replayer
/// treats that as a reason to stop: nothing is done that the trail does not know about.
public protocol AuditSink {
    func record(_ event: AuditEvent) -> Bool
}
