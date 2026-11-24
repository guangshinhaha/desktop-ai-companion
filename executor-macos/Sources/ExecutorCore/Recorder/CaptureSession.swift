import CoreGraphics
import Foundation

/// Turns raw input events into steps. This is the only place events are read.
/// From a keystroke it reads the key code and the command and control flags, to tell a character
/// from Tab or Return, and nothing else: not the characters, not the key code afterwards.
/// A click is read for its position only.
public final class CaptureSession {
    let source: AccessibilitySource
    let appBundle: String
    public let builder: TaskBuilder
    /// Return pressed in a field submits the form, which is not captured as a step. The count lets
    /// the command tell the officer to click the button instead.
    public private(set) var uncapturedSubmitKeys = 0

    public init(source: AccessibilitySource, appBundle: String, builder: TaskBuilder) {
        self.source = source; self.appBundle = appBundle; self.builder = builder
    }

    public func handle(_ type: CGEventType, _ event: CGEvent) {
        switch type {
        case .leftMouseDown:
            // The second click of a double-click is the same press.
            guard event.getIntegerValueField(.mouseEventClickState) <= 1 else { return }
            guard let el = source.element(at: event.location), inTargetApp(el) else { return }
            guard let t = Classifier.classify(el) else { return }
            // A click on a field only focuses it. A step starts when the officer types.
            // The exception is a password field: macOS hides keystrokes from event taps while one is
            // focused, so the click is the only sign it was used. It becomes the precondition.
            if t.kind == .press || t.kind == .select { builder.pressed(t) }
            if t.kind == .secure { builder.typed(into: t) }
        case .keyDown:
            let flags = event.flags
            let code = event.getIntegerValueField(.keyboardEventKeycode)
            let isSubmit = Classifier.isSubmitKey(keyCode: code)
            let cmd = flags.contains(.maskCommand)
            let isPaste = Classifier.isPasteKey(keyCode: code, command: cmd)
            let isText = Classifier.isTextKey(keyCode: code, command: cmd, control: flags.contains(.maskControl))
            guard isText || isSubmit || isPaste else { return }
            guard let el = source.focusedElement(), inTargetApp(el) else { return }
            guard let t = Classifier.classify(el) else { return }
            // Space or Return on a focused button activates it, the same as a click.
            if t.kind == .press { if Classifier.isActivationKey(keyCode: code) { builder.pressed(t) }; return }
            if isSubmit { if t.kind == .text { uncapturedSubmitKeys += 1 }; return }
            // A paste fills a field the same as typing does. Its content is never read.
            if t.kind == .text || t.kind == .secure || t.kind == .select { builder.typed(into: t) }
        default:
            return
        }
    }

    /// Other apps' elements are dropped before any attribute is read.
    func inTargetApp(_ el: ElementReading) -> Bool {
        source.bundleId(forPid: el.pid) == appBundle
    }
}
