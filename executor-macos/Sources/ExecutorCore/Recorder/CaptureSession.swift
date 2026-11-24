import CoreGraphics
import Foundation

/// Turns raw input events into steps. This is the only place events are read.
/// From a keystroke it reads the key code and the command and control flags, to tell a character
/// from Tab or Return, and nothing else: not the characters, not the key code afterwards.
/// A click is read for its position only.
/// Reads the focused window of the recorded app as a tree of roles and labels (never field values).
public protocol WindowSnapshotting {
    func snapshot() -> UINode?
}

/// What proposing one step's check cost, in milliseconds. `snapshotMs` is reading the window after the
/// action; `compareMs` is comparing it with the window before. No values, only names and numbers.
public struct StepTiming: Equatable {
    public var step: String
    public var snapshotMs: Double
    public var compareMs: Double
    public var kind: String
}

public final class CaptureSession {
    let source: AccessibilitySource
    let appBundle: String
    public let builder: TaskBuilder
    /// Return pressed in a field submits the form, which is not captured as a step. The count lets
    /// the command tell the officer to click the button instead.
    public private(set) var uncapturedSubmitKeys = 0

    var snapshotter: WindowSnapshotting?
    /// The window as it was when the last step began. The next step's "before".
    var baseline: UINode?
    /// A press whose screen change is read when the next step begins, or at stop.
    var pending: (index: Int, before: UINode?)?
    public private(set) var timings: [StepTiming] = []

    public init(source: AccessibilitySource, appBundle: String, builder: TaskBuilder, snapshotter: WindowSnapshotting? = nil) {
        self.source = source; self.appBundle = appBundle; self.builder = builder; self.snapshotter = snapshotter
    }

    /// Takes the first "before". Call once, when the tap goes on.
    public func start() { baseline = snapshotter?.snapshot() }

    /// Reads the window once the last press has had its effect, proposes that press's check, and makes
    /// that reading the next "before". No timer: the screen is read when the officer's next action begins
    /// (before it can change anything) or when they stop, by which time the last action has settled.
    public func settle() {
        guard let snapshotter else { return }
        let t0 = Self.now()
        let after = snapshotter.snapshot()
        let t1 = Self.now()
        if let p = pending {
            pending = nil
            if let b = p.before, let a = after {
                let proposal = ExpectProposer.propose(before: b, after: a)
                let t2 = Self.now()
                if let proposal { builder.setExpect(proposal.json(), at: p.index) }
                timings.append(StepTiming(step: "s\(p.index + 1)", snapshotMs: (t1 - t0) * 1000, compareMs: (t2 - t1) * 1000,
                                          kind: proposal?.kind.rawValue ?? "exists"))
            }
        }
        // A failed read leaves no "before" for the next press, which then keeps its default check.
        // Keeping the old window would give it a change that happened before it.
        baseline = after
    }

    static func now() -> Double { ProcessInfo.processInfo.systemUptime }

    /// Hands a target to the builder. A new step first settles the one before it.
    func act(_ t: Target) {
        if builder.startsStep(t) { settle() }
        let before = builder.stepCount
        builder.typed(into: t)   // a press goes to pressed(_:) from there
        if t.kind == .press, builder.stepCount > before, snapshotter != nil { pending = (before, baseline) }
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
            if t.kind == .press || t.kind == .select { act(t) }
            if t.kind == .secure { act(t) }
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
            if t.kind == .press { if Classifier.isActivationKey(keyCode: code) { act(t) }; return }
            if isSubmit { if t.kind == .text { uncapturedSubmitKeys += 1 }; return }
            // A paste fills a field the same as typing does. Its content is never read.
            if t.kind == .text || t.kind == .secure || t.kind == .select { act(t) }
        default:
            return
        }
    }

    /// Other apps' elements are dropped before any attribute is read.
    func inTargetApp(_ el: ElementReading) -> Bool {
        source.bundleId(forPid: el.pid) == appBundle
    }
}
