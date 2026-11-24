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
        let decoder = JSONDecoder()
        struct Wrapper: Decodable { var target: Descriptor? }
        let obj = try JSONSerialization.jsonObject(with: data)
        guard let dict = obj as? [String: Any] else {
            throw LoadError.notADescriptor("descriptor file must hold a JSON object")
        }
        let d: Descriptor
        if dict["target"] != nil, let t = try decoder.decode(Wrapper.self, from: data).target {
            d = t
        } else {
            d = try decoder.decode(Descriptor.self, from: data)
        }
        if d.role == nil && d.label == nil && d.labelMatches == nil && d.domId == nil && d.path == nil {
            throw LoadError.notADescriptor("descriptor needs at least one of role, label, label_matches, dom_id, path")
        }
        return d
    }
}
