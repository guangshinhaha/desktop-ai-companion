import Foundation

/// `--approval-file` answers every approval from a file, with no card seen by anyone (issue #75).
/// It exists for the test harness against the stand-in app. A release build refuses it.
public enum ApprovalFilePolicy {
    #if DEBUG
    public static let isDebugBuild = true
    #else
    public static let isDebugBuild = false
    #endif

    /// nil when `--approval-file` may be used, else why not.
    public static func refusal(debugBuild: Bool = isDebugBuild) -> String? {
        debugBuild ? nil : "--approval-file is for the test harness and is not available in a release build; approvals come from the officer in the shell or a terminal"
    }
}
