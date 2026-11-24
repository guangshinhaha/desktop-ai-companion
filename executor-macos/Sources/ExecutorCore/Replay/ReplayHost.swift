import Foundation

/// The live app the replay drives. The macOS adapter is `AXApp`; tests use a scripted fake.
/// Every method acts through accessibility actions. There is no synthetic click or keystroke.
public protocol ReplayHost: AnyObject {
    /// The chosen window as a tree of roles and labels, or nil when there is no window yet.
    func snapshot() -> UINode?
    /// Blocks until the app sends an accessibility event or `timeout` seconds pass.
    func wait(_ timeout: TimeInterval)
    func now() -> TimeInterval
    /// `AXPress` on the element.
    func press(_ node: UINode) -> ActionResult
    /// Sets the field's value, as the accessibility API's `AXValue`.
    func setValue(_ node: UINode, to text: String) -> ActionResult
    /// Chooses the option whose label is `option` in a pop-up.
    func choose(_ node: UINode, option: String) -> ActionResult
    /// The field's current value, for a `value_equals` check. In memory only; the caller never prints it.
    func value(of node: UINode) -> String?
}

public enum ActionResult: Equatable {
    case ok
    case failed(String)
}

/// TD section 5.3: re-evaluate on each accessibility event; for apps that miss events, re-check on a
/// backoff that starts at 100 ms and stops at 2 s. The one place the client polls, and only while a
/// step is waiting. `host.wait` returns as soon as an event arrives, so a step that is already true
/// does not wait at all.
public enum Waiting {
    public static let backoffStart: TimeInterval = 0.1
    public static let backoffCap: TimeInterval = 2.0

    public enum Poll<T> {
        case done(T)
        case keepWaiting
        case stop(String)
    }

    public enum Result<T> {
        case done(T)
        case ceiling
        case stopped(String)
    }

    public static func until<T>(
        ceiling: TimeInterval,
        wait: (TimeInterval) -> Void,
        now: () -> TimeInterval,
        poll: () -> Poll<T>
    ) -> Result<T> {
        let deadline = now() + ceiling
        var backoff = backoffStart
        while true {
            switch poll() {
            case .done(let v): return .done(v)
            case .stop(let why): return .stopped(why)
            case .keepWaiting: break
            }
            let remaining = deadline - now()
            if remaining <= 0 { return .ceiling }
            wait(min(backoff, remaining))
            backoff = min(backoff * 2, backoffCap)
        }
    }
}
