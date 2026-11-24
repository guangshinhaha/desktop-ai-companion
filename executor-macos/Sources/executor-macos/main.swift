import ExecutorCore
import Foundation

// executor-macos resolve --descriptor <file> [--app <bundle id>] [--window <title text>] [--ceiling <seconds>]
// Exit codes: 0 found, 2 ambiguous, 3 not found, 1 usage or error.

func fail(_ msg: String, code: Int32 = 1) -> Never {
    FileHandle.standardError.write(Data("error: \(msg)\n".utf8))
    exit(code)
}

let usage = """
usage: executor-macos resolve --descriptor <file> [--app <bundle id>] [--window <title text>] [--ceiling <seconds>]
  --app      bundle id of the running app (default com.google.Chrome)
  --window   pick the window whose title contains this text (default: the focused window)
  --ceiling  step ceiling in seconds (default 30)
"""

var args = Array(CommandLine.arguments.dropFirst())
guard args.first == "resolve" else { fail(usage) }
args.removeFirst()

var file: String?
var bundle = "com.google.Chrome"
var window: String?
var ceiling = 30.0
var i = 0
@MainActor func value(_ flag: String) -> String {
    i += 1
    guard i < args.count else { fail("\(flag) needs a value\n\(usage)") }
    return args[i]
}
while i < args.count {
    switch args[i] {
    case "--descriptor": file = value("--descriptor")
    case "--app": bundle = value("--app")
    case "--window": window = value("--window")
    case "--ceiling":
        guard let c = Double(value("--ceiling")), c > 0 else { fail("--ceiling must be a positive number of seconds") }
        ceiling = c
    default: fail("unknown argument \(args[i])\n\(usage)")
    }
    i += 1
}
guard let file else { fail("--descriptor is required\n\(usage)") }

let descriptor: Descriptor
do {
    guard let data = FileManager.default.contents(atPath: file) else { fail("cannot read \(file)") }
    descriptor = try Descriptor.load(from: data)
} catch {
    fail("bad descriptor: \(error)")
}

let app: AXApp
do { app = try AXApp(bundleId: bundle, windowTitleContains: window) } catch { fail("\(error)") }

func describe(_ n: UINode) -> String {
    "role=\(n.role) label=\(n.label.map { "\"\($0)\"" } ?? "-") dom_id=\(n.domId ?? "-")"
}

let result = Resolver.resolve(
    descriptor,
    ceiling: ceiling,
    snapshot: { app.snapshot() },
    wait: { app.wait($0) },
    now: { ProcessInfo.processInfo.systemUptime }
)

switch result {
case .found(let m):
    print("match: found")
    print("locator: \(m.locator.rawValue) (\(m.locator.summary))")
    print("element: \(describe(m.node))")
case .ambiguous(let a):
    print("match: ambiguous")
    print("locator: \(a.locator.rawValue) (\(a.locator.summary))")
    print("candidates: \(a.candidates.count)")
    for c in a.candidates { print("  \(describe(c))") }
    exit(2)
case .notFound:
    print("match: not found")
    print("ceiling: \(ceiling)s")
    exit(3)
}
