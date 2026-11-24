import ApplicationServices
import Foundation

/// `ReplayHost` over one app process. Acts only through accessibility actions and attributes
/// (`AXPress`, `AXValue`), only on elements of this process, and never with a synthetic mouse or key event.
extension AXApp: ReplayHost {
    public func now() -> TimeInterval { ProcessInfo.processInfo.systemUptime }

    private func live(_ node: UINode) -> AXUIElement? {
        guard let h = node.handle, CFGetTypeID(h as CFTypeRef) == AXUIElementGetTypeID() else { return nil }
        let el = h as! AXUIElement
        var p: pid_t = 0
        guard AXUIElementGetPid(el, &p) == .success, p == pid else { return nil }
        AXUIElementSetMessagingTimeout(el, 2)
        return el
    }

    private static func name(_ e: AXError) -> String { "AX error \(e.rawValue)" }

    public func press(_ node: UINode) -> ActionResult {
        guard let el = live(node) else { return .failed("element is not in the target process") }
        let e = AXUIElementPerformAction(el, kAXPressAction as CFString)
        return e == .success ? .ok : .failed(Self.name(e))
    }

    public func setValue(_ node: UINode, to text: String) -> ActionResult {
        guard let el = live(node) else { return .failed("element is not in the target process") }
        let e = AXUIElementSetAttributeValue(el, kAXValueAttribute as CFString, text as CFString)
        return e == .success ? .ok : .failed(Self.name(e))
    }

    public func value(of node: UINode) -> String? {
        guard let el = live(node) else { return nil }
        var v: CFTypeRef?
        guard AXUIElementCopyAttributeValue(el, kAXValueAttribute as CFString, &v) == .success, let v else { return nil }
        if let s = v as? String { return s }
        if let n = v as? NSNumber { return n.stringValue }
        return nil
    }

    /// Opens the pop-up with `AXPress`, waits for its menu on accessibility events, and presses the one
    /// item whose text is `option`. The menu closes on the press; on any failure it is cancelled.
    public func choose(_ node: UINode, option: String) -> ActionResult {
        guard let el = live(node) else { return .failed("element is not in the target process") }
        let want = Resolver.squash(option)
        let opened = AXUIElementPerformAction(el, kAXPressAction as CFString)
        guard opened == .success else { return .failed(Self.name(opened)) }
        var menu: AXUIElement?
        var matches: [AXUIElement] = []
        let r: Waiting.Result<Void> = Waiting.until(ceiling: 5, wait: wait, now: now) {
            menu = Self.children(el).first { Self.attr($0, kAXRoleAttribute) == kAXMenuRole }
            guard let menu else { return .keepWaiting }
            let items = Self.children(menu).filter { Self.attr($0, kAXRoleAttribute) == kAXMenuItemRole }
            if items.count < 2 { return .keepWaiting }
            matches = items.filter { (Self.attr($0, kAXTitleAttribute) ?? Self.attr($0, kAXValueAttribute)).map(Resolver.squash) == want }
            return .done(())
        }
        func cancel() { if let menu { _ = AXUIElementPerformAction(menu, kAXCancelAction as CFString) } }
        guard case .done = r else { cancel(); return .failed("the pop-up did not open") }
        guard matches.count == 1 else {
            cancel()
            return .failed(matches.isEmpty ? "no option with that label" : "more than one option with that label")
        }
        let e = AXUIElementPerformAction(matches[0], kAXPressAction as CFString)
        if e != .success { cancel(); return .failed(Self.name(e)) }
        return .ok
    }

    private static func attr(_ e: AXUIElement, _ k: String) -> String? {
        var v: CFTypeRef?
        guard AXUIElementCopyAttributeValue(e, k as CFString, &v) == .success, let s = v as? String, !s.isEmpty else { return nil }
        return s
    }

    private static func children(_ e: AXUIElement) -> [AXUIElement] {
        var v: CFTypeRef?
        guard AXUIElementCopyAttributeValue(e, kAXChildrenAttribute as CFString, &v) == .success else { return [] }
        return (v as? [AXUIElement]) ?? []
    }
}
