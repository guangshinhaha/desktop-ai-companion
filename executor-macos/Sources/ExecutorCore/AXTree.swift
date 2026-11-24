import ApplicationServices
import AppKit
import Foundation

/// Reads one running app's accessibility tree into `UINode` snapshots and waits on its events.
/// Read-only apart from one thing: it asks the app to build its accessibility tree
/// (`AXEnhancedUserInterface`, `AXManualAccessibility`), which browsers leave empty until asked.
/// It never presses, types into or sets the value of any element.
public final class AXApp {
    public enum Failure: Error, CustomStringConvertible {
        case notTrusted
        case notRunning(String)
        public var description: String {
            switch self {
            case .notTrusted:
                return "Accessibility permission is not granted to this process (System Settings > Privacy & Security > Accessibility)"
            case .notRunning(let b): return "no running app with bundle id \(b)"
            }
        }
    }

    public let pid: pid_t
    let element: AXUIElement
    let windowTitleContains: String?
    private let signal = Signal()
    private var observer: AXObserver?

    static let maxNodes = 40_000
    static let maxDepth = 100

    final class Signal { var fired = false }

    public init(bundleId: String, windowTitleContains: String? = nil) throws {
        guard AXIsProcessTrusted() else { throw Failure.notTrusted }
        // More than one process can share a bundle id (a second browser profile). Take the one with a window.
        let running = NSRunningApplication.runningApplications(withBundleIdentifier: bundleId)
        func hasWindow(_ a: NSRunningApplication) -> Bool {
            var v: CFTypeRef?
            let el = AXUIElementCreateApplication(a.processIdentifier)
            AXUIElementSetMessagingTimeout(el, 2)
            guard AXUIElementCopyAttributeValue(el, kAXWindowsAttribute as CFString, &v) == .success,
                  let ws = v as? [AnyObject] else { return false }
            return !ws.isEmpty
        }
        guard let app = running.first(where: hasWindow) ?? running.first else {
            throw Failure.notRunning(bundleId)
        }
        pid = app.processIdentifier
        element = AXUIElementCreateApplication(pid)
        self.windowTitleContains = windowTitleContains
        AXUIElementSetMessagingTimeout(element, 2)
        wake()
        subscribe()
    }

    /// Browsers build the page's tree lazily. These two writes ask them to. Errors are expected
    /// (a build can report `cannotComplete` and still wake the tree), so they are ignored.
    func wake() {
        _ = AXUIElementSetAttributeValue(element, "AXEnhancedUserInterface" as CFString, kCFBooleanTrue)
        _ = AXUIElementSetAttributeValue(element, "AXManualAccessibility" as CFString, kCFBooleanTrue)
    }

    private static let notifications = [
        kAXCreatedNotification, kAXUIElementDestroyedNotification, kAXValueChangedNotification,
        kAXFocusedUIElementChangedNotification, kAXFocusedWindowChangedNotification,
        kAXTitleChangedNotification, kAXLayoutChangedNotification, kAXSelectedChildrenChangedNotification,
        "AXLoadComplete", "AXLayoutComplete",
    ]

    private func subscribe() {
        var obs: AXObserver?
        let callback: AXObserverCallback = { _, _, _, refcon in
            guard let refcon else { return }
            Unmanaged<Signal>.fromOpaque(refcon).takeUnretainedValue().fired = true
        }
        guard AXObserverCreate(pid, callback, &obs) == .success, let obs else { return }
        let refcon = Unmanaged.passUnretained(signal).toOpaque()
        for n in Self.notifications { _ = AXObserverAddNotification(obs, element, n as CFString, refcon) }
        CFRunLoopAddSource(CFRunLoopGetCurrent(), AXObserverGetRunLoopSource(obs), .defaultMode)
        observer = obs
    }

    deinit {
        if let obs = observer {
            CFRunLoopRemoveSource(CFRunLoopGetCurrent(), AXObserverGetRunLoopSource(obs), .defaultMode)
        }
    }

    /// Blocks until the app sends an accessibility event, or `timeout` seconds pass.
    public func wait(_ timeout: TimeInterval) {
        signal.fired = false
        _ = CFRunLoopRunInMode(.defaultMode, timeout, true)
    }

    /// A snapshot of the chosen window, or nil when the app has no window yet.
    public func snapshot() -> UINode? {
        guard let window = pickWindow() else { wake(); return nil }
        var count = 0
        let root = build(window, depth: 0, count: &count)
        if count < 5 { wake() }
        return root
    }

    private func pickWindow() -> AXUIElement? {
        if let sub = windowTitleContains {
            for w in windows() where (string(w, kAXTitleAttribute) ?? "").localizedCaseInsensitiveContains(sub) { return w }
            return nil
        }
        if let w = elementAttr(element, kAXFocusedWindowAttribute) ?? elementAttr(element, kAXMainWindowAttribute) { return w }
        return windows().first
    }

    private func windows() -> [AXUIElement] {
        var v: CFTypeRef?
        guard AXUIElementCopyAttributeValue(element, kAXWindowsAttribute as CFString, &v) == .success,
              let arr = v as? [AXUIElement] else { return [] }
        return arr
    }

    private func build(_ el: AXUIElement, depth: Int, count: inout Int) -> UINode {
        count += 1
        let attrs = [kAXRoleAttribute, kAXTitleAttribute, kAXDescriptionAttribute, "AXDOMIdentifier",
                     kAXPlaceholderValueAttribute, kAXTitleUIElementAttribute, kAXChildrenAttribute]
        var raw: CFArray?
        var vals: [Any?] = Array(repeating: nil, count: attrs.count)
        if AXUIElementCopyMultipleAttributeValues(el, attrs as CFArray, AXCopyMultipleAttributeOptions(rawValue: 0), &raw) == .success,
           let arr = raw as? [AnyObject] {
            for (i, v) in arr.enumerated() where i < vals.count {
                if CFGetTypeID(v) == AXValueGetTypeID() { continue }  // an error placeholder
                vals[i] = v
            }
        }
        let role = (vals[0] as? String) ?? "AXUnknown"
        func s(_ i: Int) -> String? { (vals[i] as? String).flatMap { $0.isEmpty ? nil : $0 } }
        var label = s(1) ?? s(2)
        if label == nil, let t = vals[5], CFGetTypeID(t as CFTypeRef) == AXUIElementGetTypeID() {
            let te = t as! AXUIElement
            label = string(te, kAXTitleAttribute) ?? string(te, kAXValueAttribute)
        }
        if label == nil { label = s(4) }
        // Only static text is read for its text. A field's value is never read.
        if label == nil, role == kAXStaticTextRole { label = string(el, kAXValueAttribute) }

        var kids: [UINode] = []
        if depth < Self.maxDepth, role != kAXMenuBarRole, let cs = vals[6] as? [AXUIElement] {
            for c in cs {
                if count >= Self.maxNodes { break }
                kids.append(build(c, depth: depth + 1, count: &count))
            }
        }
        return UINode(role: role, label: label, domId: s(3), children: kids, handle: el)
    }

    private func string(_ el: AXUIElement, _ attr: String) -> String? {
        var v: CFTypeRef?
        guard AXUIElementCopyAttributeValue(el, attr as CFString, &v) == .success, let s = v as? String, !s.isEmpty else { return nil }
        return s
    }

    private func elementAttr(_ el: AXUIElement, _ attr: String) -> AXUIElement? {
        var v: CFTypeRef?
        guard AXUIElementCopyAttributeValue(el, attr as CFString, &v) == .success, let v,
              CFGetTypeID(v) == AXUIElementGetTypeID() else { return nil }
        return (v as! AXUIElement)
    }
}
