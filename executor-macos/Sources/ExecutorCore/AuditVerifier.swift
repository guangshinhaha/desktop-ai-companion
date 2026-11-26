import CryptoKit
import Foundation

/// Checks an audit trail export from the shell's store (TD 7.2). The export is JSON lines: first a control
/// line `{anchor_seq, anchor_hash, head_seq, head_hash}`, then one object per record in seq order.
/// The hash is the shell store's (`audit_hash` in shell/src-tauri/src/store.rs): SHA-256 over
/// length-prefixed fields with a presence byte. A fixture made by that store is checked in the tests, so
/// the two implementations cannot drift apart unnoticed.
///
/// What this proves: only that the export is consistent with itself. The hashes use no secret and the export
/// is plain text, so anyone who holds it can edit or drop records, or cut its tail, then recompute the hashes,
/// rewrite the control line and pass. No store key is needed. A pass does not show that the export matches
/// what was written. The live store has limits too (decision 0007): a key holder can rewrite the trail, and
/// someone with access to its files can restore an older copy or delete it. The off-device anchor is the
/// central copy (#29), which is not built; records changed before their first sync are outside what it can show.
public enum AuditVerifier {
    public enum Verdict: Equatable {
        case ok(records: Int)
        /// The chain breaks at this sequence number: the record was edited, or is missing (the number the
        /// chain expected next), or the trail ends before its recorded head.
        case broken(seq: Int64, why: String)
        case unreadable(String)
    }

    static let zeros = String(repeating: "0", count: 64)

    public static func verify(lines: [String]) -> Verdict {
        var it = lines.filter { !$0.trimmingCharacters(in: .whitespaces).isEmpty }.makeIterator()
        guard let first = it.next(), let meta = object(first),
              let anchorSeq = int(meta["anchor_seq"]), let anchorHash = meta["anchor_hash"] as? String,
              let headSeq = int(meta["head_seq"]), let headHash = meta["head_hash"] as? String else {
            return .unreadable("the first line must hold anchor_seq, anchor_hash, head_seq and head_hash")
        }
        var nextSeq = anchorSeq + 1
        var nextPrev = anchorHash
        var count = 0
        while let line = it.next() {
            guard let r = object(line), let seq = int(r["seq"]), let at = int(r["at"]),
                  let officer = r["officer"] as? String, let event = r["event"] as? String,
                  let prev = r["prev_hash"] as? String, let hash = r["hash"] as? String else {
                return .unreadable("record \(count + 1) is not a complete audit record")
            }
            // Nothing may be added, and no key may appear twice (readers disagree on which one wins). The count
            // must match the parsed record's, so a key the scan reads differently from the parser fails too.
            let keys = topLevelKeys(line)
            if !Set(r.keys).isSubset(of: knownKeys) || keys.count != r.count || Set(keys) != Set(r.keys) {
                return .broken(seq: seq, why: "record \(seq) was edited")
            }
            // A field of the wrong type must not read as "absent": that would let an edit hide.
            for k in ["task", "run", "step", "shown"] where r[k] != nil && !(r[k] is NSNull) && !(r[k] is String) {
                return .broken(seq: seq, why: "record \(seq) was edited")
            }
            if r["item"] != nil, !(r["item"] is NSNull), int(r["item"]) == nil {
                return .broken(seq: seq, why: "record \(seq) was edited")
            }
            let calc = hashOf(prev: prev, at: at, officer: officer, event: event, task: str(r["task"]), run: str(r["run"]),
                              item: int(r["item"]), step: str(r["step"]), shown: str(r["shown"]))
            if seq != nextSeq { return .broken(seq: nextSeq, why: "record \(nextSeq) is missing or out of order") }
            if prev != nextPrev { return .broken(seq: seq, why: "record \(seq) does not follow the one before it") }
            if calc != hash { return .broken(seq: seq, why: "record \(seq) was edited") }
            nextSeq += 1
            nextPrev = hash
            count += 1
        }
        if nextSeq - 1 != headSeq || nextPrev != headHash {
            return .broken(seq: nextSeq, why: "the trail ends at \(nextSeq - 1) but its head is \(headSeq)")
        }
        return .ok(records: count)
    }

    static func hashOf(prev: String, at: Int64, officer: String, event: String, task: String?, run: String?,
                       item: Int64?, step: String?, shown: String?) -> String {
        var h = SHA256()
        func put(_ s: String?) {
            guard let s else { h.update(data: Data([0])); return }
            h.update(data: Data([1]))
            let n = UInt64(s.utf8.count).bigEndian
            withUnsafeBytes(of: n) { h.update(bufferPointer: $0) }
            h.update(data: Data(s.utf8))
        }
        put(prev); put(String(at)); put(officer); put(event)
        put(task); put(run); put(step); put(shown)
        put(item.map { String($0) })
        return h.finalize().map { String(format: "%02x", $0) }.joined()
    }

    /// The names of the top-level keys of a line that has parsed as a JSON object, escapes decoded, repeats
    /// kept. A string is a key when a colon follows it, after any JSON whitespace.
    static func topLevelKeys(_ line: String) -> [String] {
        let b = Array(line.utf8)
        var keys: [String] = []
        var depth = 0
        var i = 0
        while i < b.count {
            switch b[i] {
            case UInt8(ascii: "{"), UInt8(ascii: "["): depth += 1
            case UInt8(ascii: "}"), UInt8(ascii: "]"): depth -= 1
            case UInt8(ascii: "\""):
                var end = i + 1
                while end < b.count, b[end] != UInt8(ascii: "\"") { end += b[end] == UInt8(ascii: "\\") ? 2 : 1 }
                guard end < b.count else { return keys }
                if depth == 1 {
                    var next = end + 1
                    while next < b.count, [0x20, 0x09, 0x0A, 0x0D].contains(b[next]) { next += 1 }
                    if next < b.count, b[next] == UInt8(ascii: ":"),
                       let name = (try? JSONSerialization.jsonObject(with: Data(b[i...end]), options: .fragmentsAllowed)) as? String {
                        keys.append(name)
                    }
                }
                i = end
            default: break
            }
            i += 1
        }
        return keys
    }

    private static let knownKeys: Set<String> = ["seq", "at", "officer", "event", "task", "run", "item", "step", "shown", "prev_hash", "hash"]
    private static func object(_ line: String) -> [String: Any]? {
        (try? JSONSerialization.jsonObject(with: Data(line.utf8))) as? [String: Any]
    }
    private static func int(_ v: Any?) -> Int64? {
        guard let n = v as? NSNumber, CFGetTypeID(n) != CFBooleanGetTypeID() else { return nil }
        // A whole number only: 2.7 or 2.0 must not read as 2.
        guard !CFNumberIsFloatType(n), n.doubleValue == Double(n.int64Value) else { return nil }
        return n.int64Value
    }
    private static func str(_ v: Any?) -> String? { v as? String }
}
