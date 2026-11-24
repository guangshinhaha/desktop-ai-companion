import Foundation

/// The `target` of a step in task/v1 (schema/task.v1.json, `$defs/descriptor`).
/// It names an element by what it is, never by where it sits, and never holds a value.
public struct Descriptor: Codable, Equatable, Sendable {
    public var role: String?
    public var label: String?
    /// A glob (`*` and `?`), matched case-insensitively. Used by locator 3.
    public var labelMatches: String?
    public var domId: String?
    /// Locator 4. Each segment is `Role`, `Role:label` or `Role:N` (N counts from 1).
    public var path: [String]?
    /// Nearby label text that must also be on screen. Used by locator 3.
    public var anchors: [String]?

    enum CodingKeys: String, CodingKey {
        case role, label, path, anchors
        case labelMatches = "label_matches"
        case domId = "dom_id"
    }

    public init(role: String? = nil, label: String? = nil, labelMatches: String? = nil,
                domId: String? = nil, path: [String]? = nil, anchors: [String]? = nil) {
        self.role = role; self.label = label; self.labelMatches = labelMatches
        self.domId = domId; self.path = path; self.anchors = anchors
    }

    public enum LoadError: Error, CustomStringConvertible {
        case notADescriptor(String)
        public var description: String {
            switch self { case .notADescriptor(let m): return m }
        }
    }

    /// Reads a bare descriptor, or an object that holds one under `target` (a step).
    public static func load(from data: Data) throws -> Descriptor {
        let obj = try JSONSerialization.jsonObject(with: data)
        guard var dict = obj as? [String: Any] else {
            throw LoadError.notADescriptor("descriptor file must hold a JSON object")
        }
        // A step holds its descriptor under `target`. A step also has other keys, so only
        // the descriptor's own keys beside `target` are an error (which one would win?).
        if let t = dict["target"] {
            if let clash = dict.keys.first(where: { Self.keys.contains($0) }) {
                throw LoadError.notADescriptor("both target and \(clash) given; pass one descriptor")
            }
            guard let inner = t as? [String: Any] else { throw LoadError.notADescriptor("target must be an object") }
            dict = inner
        }
        // The same rules as the schema: known keys only, non-empty strings, path has an element.
        for k in dict.keys where !Self.keys.contains(k) {
            throw LoadError.notADescriptor("unknown key \(k)")
        }
        for k in ["role", "label", "label_matches", "dom_id"] {
            if let v = dict[k], (v as? String).map({ $0.isEmpty }) ?? true {
                throw LoadError.notADescriptor("\(k) must be a non-empty string")
            }
        }
        for k in ["path", "anchors"] {
            guard let v = dict[k] else { continue }
            guard let arr = v as? [Any], arr.allSatisfy({ ($0 as? String).map { !$0.isEmpty } ?? false }) else {
                throw LoadError.notADescriptor("\(k) must be a list of non-empty strings")
            }
            if k == "path" && arr.isEmpty { throw LoadError.notADescriptor("path must have at least one element") }
        }
        let d = try JSONDecoder().decode(Descriptor.self, from: JSONSerialization.data(withJSONObject: dict))
        if d.role == nil && d.label == nil && d.labelMatches == nil && d.domId == nil && d.path == nil {
            throw LoadError.notADescriptor("descriptor needs at least one of role, label, label_matches, dom_id, path")
        }
        return d
    }

    static let keys: Set<String> = ["role", "label", "label_matches", "dom_id", "path", "anchors"]
}
