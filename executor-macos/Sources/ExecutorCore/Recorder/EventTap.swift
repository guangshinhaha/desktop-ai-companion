import AppKit
import CoreGraphics
import Foundation

/// One listen-only event tap for left clicks and key-downs, and nothing else. It cannot change or
/// block an event. It exists only between `start()` and `stop()`.
@MainActor
public final class EventTap {
    public enum Failure: Error, CustomStringConvertible {
        case noPermission
        public var description: String {
            "cannot create the event tap: grant Input Monitoring and Accessibility to this terminal (System Settings > Privacy & Security)"
        }
    }

    public nonisolated static let mask: CGEventMask =
        (1 << CGEventType.leftMouseDown.rawValue) | (1 << CGEventType.keyDown.rawValue)

    private var tap: CFMachPort?
    private var source: CFRunLoopSource?
    private let handler: (CGEventType, CGEvent) -> Void
    public var isActive: Bool { tap != nil }

    public init(handler: @escaping (CGEventType, CGEvent) -> Void) { self.handler = handler }

    public func start() throws {
        guard tap == nil else { return }
        let refcon = Unmanaged.passUnretained(self).toOpaque()
        let callback: CGEventTapCallBack = { _, type, event, refcon in
            guard let refcon else { return Unmanaged.passUnretained(event) }
            let me = Unmanaged<EventTap>.fromOpaque(refcon).takeUnretainedValue()
            // The tap's run loop source is on the main run loop.
            MainActor.assumeIsolated { me.deliver(type, event) }
            return Unmanaged.passUnretained(event)
        }
        guard let t = CGEvent.tapCreate(tap: .cgSessionEventTap, place: .tailAppendEventTap, options: .listenOnly,
                                        eventsOfInterest: Self.mask, callback: callback, userInfo: refcon) else {
            throw Failure.noPermission
        }
        let s = CFMachPortCreateRunLoopSource(kCFAllocatorDefault, t, 0)
        CFRunLoopAddSource(CFRunLoopGetMain(), s, .commonModes)
        CGEvent.tapEnable(tap: t, enable: true)
        tap = t; source = s
    }

    func deliver(_ type: CGEventType, _ event: CGEvent) {
        if type == .tapDisabledByTimeout || type == .tapDisabledByUserInput {
            if let tap { CGEvent.tapEnable(tap: tap, enable: true) }
            return
        }
        handler(type, event)
    }

    public func stop() {
        if let source { CFRunLoopRemoveSource(CFRunLoopGetMain(), source, .commonModes) }
        if let tap {
            CGEvent.tapEnable(tap: tap, enable: false)
            CFMachPortInvalidate(tap)
        }
        tap = nil; source = nil
    }

    deinit { }
}

/// The menu-bar recording indicator (PRD R-1). Shown for the whole recording and removed on stop.
@MainActor
public final class RecordingIndicator {
    private var item: NSStatusItem?
    private var stopTarget: StopTarget?
    public var isVisible: Bool { item != nil }

    final class StopTarget: NSObject {
        let onStop: () -> Void
        init(_ f: @escaping () -> Void) { onStop = f }
        @objc func stop() { onStop() }
    }

    public init() {}

    public func show(onStop: @escaping () -> Void) {
        guard item == nil else { return }
        let it = NSStatusBar.system.statusItem(withLength: NSStatusItem.variableLength)
        let title = NSAttributedString(string: "\u{25CF} REC", attributes: [
            .foregroundColor: NSColor.systemRed, .font: NSFont.boldSystemFont(ofSize: 13),
        ])
        it.button?.attributedTitle = title
        it.button?.toolTip = "Recording a task. Click to stop."
        let target = StopTarget(onStop)
        let menu = NSMenu()
        let stop = NSMenuItem(title: "Stop recording", action: #selector(StopTarget.stop), keyEquivalent: "")
        stop.target = target
        menu.addItem(stop)
        it.menu = menu
        item = it; stopTarget = target
    }

    public func hide() {
        if let item { NSStatusBar.system.removeStatusItem(item) }
        item = nil; stopTarget = nil
    }
}
