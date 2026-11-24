import ExecutorCore
import Foundation

// executor-macos verify-audit [--file <audit export>]
// Exit codes: 0 the trail is intact, 5 the chain is broken (the output names the sequence number),
// 1 usage or unreadable input.
// The export is what the shell's store writes (TD 7.2). It holds the approved values, so it is read from
// standard input by default and a file must be yours and not writable by others.

func runVerifyAudit(_ args: [String]) -> Never {
    var path: String?
    var i = 0
    while i < args.count {
        guard args[i] == "--file", i + 1 < args.count, !args[i + 1].hasPrefix("--") else {
            replayFail("usage: executor-macos verify-audit [--file <audit export>]")
        }
        path = args[i + 1]
        i += 2
    }
    let text: String
    if let path {
        var st = stat()
        guard lstat(path, &st) == 0, (st.st_mode & S_IFMT) == S_IFREG, st.st_uid == getuid(), st.st_mode & (S_IRWXG | S_IRWXO) == 0,
              let d = FileManager.default.contents(atPath: path) else {
            replayFail("the audit export must be a plain file that only you can read or write")
        }
        text = String(decoding: d, as: UTF8.self)
    } else {
        text = String(decoding: FileHandle.standardInput.readDataToEndOfFile(), as: UTF8.self)
    }
    switch AuditVerifier.verify(lines: text.components(separatedBy: "\n")) {
    case .ok(let n):
        print("audit trail intact: \(n) records")
        exit(0)
    case .broken(let seq, let why):
        print("audit trail broken at seq \(seq): \(why)")
        exit(5)
    case .unreadable(let why):
        replayFail("audit export not readable: \(why)")
    }
}
