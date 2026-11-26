import Foundation

/// `executor-macos replay` runs a task with no shell, so nothing is written to the audit trail, and its
/// `--approval-file` answers approvals with no card seen by anyone (issues #75, #107). It exists for the test
/// harness against the stand-in app. A release build refuses it.
public enum TerminalReplayPolicy {
    #if DEBUG
    public static let isDebugBuild = true
    #else
    public static let isDebugBuild = false
    #endif

    /// nil when terminal `replay` may run, else why not.
    public static func refusal(debugBuild: Bool = isDebugBuild) -> String? {
        debugBuild ? nil : "replay is for test builds only and is not available in a release build, because it records no audit trail; run the task from the companion shell"
    }
}
