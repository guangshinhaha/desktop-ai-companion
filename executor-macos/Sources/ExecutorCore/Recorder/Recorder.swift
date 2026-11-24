import AppKit
import ApplicationServices
import Foundation

/// `executor-macos record`: tap and indicator on, wait for stop, both off, then write the file.
/// Nothing is written to disk until stop, and what is written holds descriptors and input names only,
/// so a crash mid-recording leaves no file at all and never a file with keystrokes in it.
@MainActor
public final class Recorder {
    public struct Options {
        public var out: String
        public var appBundle: String
        public var name: String
        public var author: String
        public init(out: String, appBundle: String = "com.google.Chrome", name: String = "Recorded task", author: String = "officer") {
            self.out = out; self.appBundle = appBundle; self.name = name; self.author = author
        }
    }

    public enum Failure: Error, CustomStringConvertible {
        case notTrusted, notRunning(String), outExists(String), outDirMissing(String), noSteps
        public var description: String {
            switch self {
            case .notTrusted: return "Accessibility permission is not granted to this terminal (System Settings > Privacy & Security > Accessibility)"
            case .notRunning(let b): return "no running app with bundle id \(b) to record"
            case .outExists(let p): return "\(p) already exists; choose another --out"
            case .outDirMissing(let p): return "the folder for \(p) does not exist"
            case .noSteps: return "no steps were recorded, so no file was written"
            }
        }
    }

    let options: Options
    let tap: EventTap
    let indicator = RecordingIndicator()
    let session: CaptureSession
    private var signalSources: [DispatchSourceSignal] = []
    private var stopped = false

    public init(_ options: Options) {
        self.options = options
        let session = CaptureSession(source: AXSystemSource(), appBundle: options.appBundle,
                                     builder: TaskBuilder(appBundle: options.appBundle, name: options.name, author: options.author))
        self.session = session
        self.tap = EventTap { type, event in session.handle(type, event) }
    }

    /// Checks everything that can fail before the tap or the indicator is turned on.
    func preflight() throws {
        guard AXIsProcessTrusted() else { throw Failure.notTrusted }
        let url = URL(fileURLWithPath: options.out)
        var isDir: ObjCBool = false
        guard FileManager.default.fileExists(atPath: url.deletingLastPathComponent().path, isDirectory: &isDir), isDir.boolValue else {
            throw Failure.outDirMissing(options.out)
        }
        guard !FileManager.default.fileExists(atPath: options.out) else { throw Failure.outExists(options.out) }
        guard let app = NSRunningApplication.runningApplications(withBundleIdentifier: options.appBundle).first else {
            throw Failure.notRunning(options.appBundle)
        }
        // Browsers build the page's accessibility tree only when asked (same two writes as AXApp.wake).
        let el = AXUIElementCreateApplication(app.processIdentifier)
        _ = AXUIElementSetAttributeValue(el, "AXEnhancedUserInterface" as CFString, kCFBooleanTrue)
        _ = AXUIElementSetAttributeValue(el, "AXManualAccessibility" as CFString, kCFBooleanTrue)
    }

    /// Blocks until stop. Returns the file's step count.
    public func run() throws -> Int {
        try preflight()
        NSApplication.shared.setActivationPolicy(.accessory)
        try tap.start()
        indicator.show { [weak self] in self?.stop() }
        for sig in [SIGINT, SIGTERM, SIGHUP] {
            signal(sig, SIG_IGN)
            let s = DispatchSource.makeSignalSource(signal: sig, queue: .main)
            s.setEventHandler { [weak self] in MainActor.assumeIsolated { self?.stop() } }
            s.resume()
            signalSources.append(s)
        }
        log("recording \(options.appBundle): the menu bar shows REC. Ctrl-C or Stop in the menu ends it.")
        NSApplication.shared.run()
        // Both are already off here (stop() ran). Write the file.
        guard session.builder.stepCount > 0 else { throw Failure.noSteps }
        let data = try session.builder.json()
        try data.write(to: URL(fileURLWithPath: options.out), options: .atomic)
        if session.uncapturedSubmitKeys > 0 {
            log("note: Return pressed in a field \(session.uncapturedSubmitKeys) time(s) is not recorded; click the button instead.")
        }
        log("stopped: tap off, indicator off. \(session.builder.stepCount) step(s), \(session.builder.preconditionCount) precondition(s) written to \(options.out)")
        return session.builder.stepCount
    }

    func stop() {
        guard !stopped else { return }
        stopped = true
        tap.stop()
        indicator.hide()
        signalSources.forEach { $0.cancel() }
        signalSources = []
        NSApplication.shared.stop(nil)
        // stop(_:) takes effect after the next event; post one so run() returns now.
        if let e = NSEvent.otherEvent(with: .applicationDefined, location: .zero, modifierFlags: [], timestamp: 0,
                                      windowNumber: 0, context: nil, subtype: 0, data1: 0, data2: 0) {
            NSApplication.shared.postEvent(e, atStart: true)
        }
    }

    func log(_ s: String) { FileHandle.standardError.write(Data((s + "\n").utf8)) }
}
