import AppKit
import ApplicationServices
import CoreGraphics
import Foundation

/// What the recorder may ask of an element. There is deliberately no way to read a field's value:
/// `string(_:)` refuses `AXValue`, and `staticText()` reads it only from an element whose role is
/// `AXStaticText` (a label, never a field). A password field is therefore unreadable by construction,
/// and tests assert that no `AXValue` request is ever made for one.
public protocol ElementReading {
    var pid: pid_t { get }
    /// Role, subrole, title, description, placeholder, DOM id and the like. Never `AXValue`.
    func string(_ attribute: String) -> String?
    /// The text of a static-text element. Nil for any other role.
    func staticText() -> String?
    /// A related element (`AXParent`, `AXTitleUIElement`).
    func element(_ attribute: String) -> ElementReading?
    func children() -> [ElementReading]
    func same(as other: ElementReading) -> Bool
}

/// Where the recorder looks things up. The macOS one wraps the accessibility API.
public protocol AccessibilitySource {
    func element(at point: CGPoint) -> ElementReading?
    func focusedElement() -> ElementReading?
    func bundleId(forPid pid: pid_t) -> String?
}

public enum AXAttributePolicy {
    /// The one rule for reading a field's content: never through `string(_:)`, and through
    /// `staticText()` only for static text.
    public static func mayRead(attribute: String, role: String?) -> Bool {
        if attribute == kAXValueAttribute as String { return role == (kAXStaticTextRole as String) }
        return true
    }
}

/// `ElementReading` over a live `AXUIElement`.
public struct AXElementReader: ElementReading {
    let el: AXUIElement

    public init(_ el: AXUIElement) {
        self.el = el
        AXUIElementSetMessagingTimeout(el, 1)
    }

    public var pid: pid_t {
        var p: pid_t = 0
        AXUIElementGetPid(el, &p)
        return p
    }

    private func copy(_ attr: String) -> CFTypeRef? {
        var v: CFTypeRef?
        return AXUIElementCopyAttributeValue(el, attr as CFString, &v) == .success ? v : nil
    }

    public func string(_ attribute: String) -> String? {
        guard attribute != (kAXValueAttribute as String) else { return nil }
        guard let s = copy(attribute) as? String, !s.isEmpty else { return nil }
        return s
    }

    public func staticText() -> String? {
        guard AXAttributePolicy.mayRead(attribute: kAXValueAttribute as String, role: string(kAXRoleAttribute)) else { return nil }
        guard let s = copy(kAXValueAttribute) as? String, !s.isEmpty else { return nil }
        return s
    }

    public func element(_ attribute: String) -> ElementReading? {
        guard let v = copy(attribute), CFGetTypeID(v) == AXUIElementGetTypeID() else { return nil }
        return AXElementReader(v as! AXUIElement)
    }

    public func children() -> [ElementReading] {
        guard let arr = copy(kAXChildrenAttribute) as? [AXUIElement] else { return [] }
        return arr.map { AXElementReader($0) }
    }

    public func same(as other: ElementReading) -> Bool {
        guard let o = other as? AXElementReader else { return false }
        return CFEqual(el, o.el)
    }
}

public struct AXSystemSource: AccessibilitySource {
    let system = AXUIElementCreateSystemWide()

    public init() { AXUIElementSetMessagingTimeout(system, 1) }

    public func element(at point: CGPoint) -> ElementReading? {
        var out: AXUIElement?
        guard AXUIElementCopyElementAtPosition(system, Float(point.x), Float(point.y), &out) == .success, let out else { return nil }
        return AXElementReader(out)
    }

    /// The frontmost app's focused element. The system-wide query fails (`cannotComplete`) in some
    /// browser states, including while a password field has focus.
    public func focusedElement() -> ElementReading? {
        var targets: [AXUIElement] = []
        if let app = NSWorkspace.shared.frontmostApplication {
            targets.append(AXUIElementCreateApplication(app.processIdentifier))
        }
        targets.append(system)
        for t in targets {
            AXUIElementSetMessagingTimeout(t, 1)
            var v: CFTypeRef?
            if AXUIElementCopyAttributeValue(t, kAXFocusedUIElementAttribute as CFString, &v) == .success,
               let v, CFGetTypeID(v) == AXUIElementGetTypeID() { return AXElementReader(v as! AXUIElement) }
        }
        return nil
    }

    public func bundleId(forPid pid: pid_t) -> String? {
        NSRunningApplication(processIdentifier: pid)?.bundleIdentifier
    }
}
