import Foundation

/// Minimal JSON-RPC 2.0 over stdio, one JSON message per line (TD 3.2).
/// Phase 1 speaks only `ping`; replay and record join as the shell needs them.
public enum RPC {
    public static let version = "0.1.0"

    /// Handles one request line. Returns the response line, or nil for a notification (no id).
    public static func handle(line: String) -> String? {
        guard let data = line.data(using: .utf8),
              let obj = try? JSONSerialization.jsonObject(with: data) as? [String: Any]
        else { return encode(["jsonrpc": "2.0", "id": NSNull(), "error": err(-32700, "Parse error")]) }
        let id = obj["id"]
        guard obj["jsonrpc"] as? String == "2.0", let method = obj["method"] as? String else {
            return encode(["jsonrpc": "2.0", "id": id ?? NSNull(), "error": err(-32600, "Invalid Request")])
        }
        guard let id else { return nil }
        switch method {
        case "ping":
            return encode(["jsonrpc": "2.0", "id": id, "result": ["pong": true, "version": version]])
        default:
            return encode(["jsonrpc": "2.0", "id": id, "error": err(-32601, "Method not found")])
        }
    }

    /// Reads requests until stdin closes, then returns. Blocks on read, so it is idle between requests.
    public static func serve(output: FileHandle = .standardOutput) {
        while let line = readLine(strippingNewline: true) {
            if line.trimmingCharacters(in: .whitespaces).isEmpty { continue }
            if let reply = handle(line: line) { output.write(Data((reply + "\n").utf8)) }
        }
    }

    private static func err(_ code: Int, _ message: String) -> [String: Any] { ["code": code, "message": message] }

    private static func encode(_ obj: [String: Any]) -> String {
        let data = (try? JSONSerialization.data(withJSONObject: obj, options: [.sortedKeys])) ?? Data("{}".utf8)
        return String(decoding: data, as: UTF8.self)
    }
}
