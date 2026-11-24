import ExecutorCore
import Foundation

// executor-macos record --out <file> [--app <bundle id>] [--name <task name>] [--author <id>]
// executor-macos resolve --descriptor <file> [--app <bundle id>] [--window <title text>] [--ceiling <seconds>]
// Exit codes: 0 found, 2 ambiguous, 3 not found, 1 usage or error.

func fail(_ msg: String, code: Int32 = 1) -> Never {
    FileHandle.standardError.write(Data("error: \(msg)\n".utf8))
    exit(code)
}

let usage = """
usage: executor-macos record --out <file> [--app <bundle id>] [--name <task name>] [--author <id>]
       executor-macos resolve --descriptor <file> [--app <bundle id>] [--window <title text>] [--ceiling <seconds>]
       executor-macos replay <task.json> --inputs <file> [--pid <n>] [--approval-file <file>] [--ceiling <seconds>]
  --app      bundle id of the running app (default com.google.Chrome)
  --window   pick the window whose title contains this text (default: the focused window)
  --ceiling  step ceiling in seconds (default 30)
"""

var args = Array(CommandLine.arguments.dropFirst())
let command = args.first
if command == "replay" { runReplay(Array(args.dropFirst())) }
guard command == "resolve" || command == "record" else { fail(usage) }
args.removeFirst()

var file: String?
var out: String?
var taskName = "Recorded task"
var author = "officer"
var bundle = "com.google.Chrome"
var window: String?
var ceiling = 30.0
var i = 0
@MainActor func value(_ flag: String) -> String {
    i += 1
    guard i < args.count, !args[i].hasPrefix("--") else { fail("\(flag) needs a value\n\(usage)") }
    return args[i]
}
while i < args.count {
    switch args[i] {
    case "--out": out = value("--out")
    case "--name": taskName = value("--name")
    case "--author": author = value("--author")
    case "--descriptor": file = value("--descriptor")
    case "--app": bundle = value("--app")
    case "--window": window = value("--window")
    case "--ceiling":
        guard let c = Double(value("--ceiling")), c > 0, c <= 3600 else {
            fail("--ceiling must be a number of seconds above 0 and at most 3600")
        }
        ceiling = c
    default: fail("unknown argument \(args[i])\n\(usage)")
    }
    i += 1
}
if command == "record" {
    guard let out, !out.isEmpty else { fail("--out is required\n\(usage)") }
    guard !taskName.trimmingCharacters(in: .whitespaces).isEmpty, !author.trimmingCharacters(in: .whitespaces).isEmpty else {
        fail("--name and --author must not be empty")
    }
    guard !args.contains("--ceiling"), file == nil, window == nil else { fail("record takes --out, --app, --name and --author\n\(usage)") }
    do {
        _ = try Recorder(Recorder.Options(out: out, appBundle: bundle, name: taskName, author: author)).run()
        exit(0)
    } catch {
        fail("\(error)")
    }
}
guard command == "resolve" else { fail(usage) }
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
