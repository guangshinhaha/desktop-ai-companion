import Foundation

/// Builds a task/v1 document (schema/task.v1.json) from the elements the officer acted on.
/// It is given descriptors, never characters: a typed value cannot reach it, so it cannot reach the file.
public final class TaskBuilder {
    struct Step {
        var action: String
        var say: String
        var target: Descriptor
        var inputName: String?
        var consequential: String
        var approval: String?
    }

    let appBundle: String
    let appKey: String
    let name: String
    let author: String
    let id: String
    let created: Date
    var steps: [Step] = []
    var inputs: [(name: String, type: String)] = []
    var inputByField: [(Descriptor, String)] = []
    var secureFields: [Descriptor] = []

    public init(appBundle: String, name: String = "Recorded task", author: String = "officer",
                id: String? = nil, created: Date = Date()) {
        self.appBundle = appBundle
        self.appKey = Self.snake(appBundle.split(separator: ".").last.map(String.init) ?? "app", fallback: "app")
        self.name = name.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty ? "Recorded task" : name
        self.author = author.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty ? "officer" : author
        self.id = id ?? "tsk_" + String((0..<8).map { _ in "0123456789abcdef".randomElement()! })
        self.created = created
    }

    public var stepCount: Int { steps.count }
    public var preconditionCount: Int { secureFields.count }

    /// The officer typed into this element (or chose in it).
    public func typed(into t: Target) {
        switch t.kind {
        case .secure:
            if !secureFields.contains(t.descriptor) { secureFields.append(t.descriptor) }
        case .text, .select:
            // Keystrokes into one field, with nothing else between them, are one step.
            if let last = steps.last, last.action != "press", last.target == t.descriptor { return }
            let n = inputName(for: t)
            let verb = t.kind == .select ? "Choose" : "Type into"
            steps.append(Step(action: t.kind == .select ? "select" : "set_value",
                              say: "\(verb) \(Self.display(t.descriptor))",
                              target: t.descriptor, inputName: n, consequential: "no", approval: nil))
        case .press:
            pressed(t)
        }
    }

    /// A click on a control.
    public func pressed(_ t: Target) {
        guard t.kind == .press || t.kind == .select else { return }
        if t.kind == .select { typed(into: t); return }
        let label = t.descriptor.label ?? ""
        let yes = Self.isConsequentialLabel(label)
        let names = inputs.map { "{\($0.name)}" }
        let summary = "Press \(Self.display(t.descriptor))" + (names.isEmpty ? "" : " with " + names.joined(separator: ", "))
        steps.append(Step(action: "press", say: "Press \(Self.display(t.descriptor))", target: t.descriptor,
                          inputName: nil, consequential: yes ? "yes" : "unknown", approval: summary))
    }

    // MARK: rules

    static let consequentialWords: Set<String> = ["create", "submit", "send", "delete", "approve", "confirm"]

    public static func isConsequentialLabel(_ label: String) -> Bool {
        let words = label.lowercased().split(whereSeparator: { !$0.isLetter }).map(String.init)
        return words.contains(where: consequentialWords.contains)
    }

    static func display(_ d: Descriptor) -> String {
        d.label ?? (d.role.map { $0.hasPrefix("AX") ? String($0.dropFirst(2)).lowercased() : $0 } ?? "element")
    }

    static func snake(_ s: String, fallback: String) -> String {
        var out = ""
        for ch in s.lowercased() {
            if ch.isASCII, ch.isLetter || ch.isNumber { out.append(ch) }
            else if !out.hasSuffix("_") { out.append("_") }
        }
        out = out.trimmingCharacters(in: CharacterSet(charactersIn: "_"))
        if out.isEmpty { return fallback }
        if let f = out.first, !f.isLetter { out = "field_" + out }
        return out
    }

    func inputName(for t: Target) -> String {
        if let (_, n) = inputByField.first(where: { $0.0 == t.descriptor }) { return n }
        let base = Self.snake(t.descriptor.label ?? "", fallback: "field")
        var n = base
        var i = 2
        while inputs.contains(where: { $0.name == n }) { n = "\(base)_\(i)"; i += 1 }
        let type = n.contains("email") ? "email" : "text"
        inputs.append((n, type))
        inputByField.append((t.descriptor, n))
        return n
    }

    // MARK: output

    static func dict(_ d: Descriptor) -> [String: Any] {
        var o: [String: Any] = [:]
        if let v = d.role { o["role"] = v }
        if let v = d.label { o["label"] = v }
        if let v = d.domId { o["dom_id"] = v }
        if let v = d.path { o["path"] = v }
        return o
    }

    public func document() -> [String: Any] {
        let f = ISO8601DateFormatter()
        f.timeZone = .current
        var doc: [String: Any] = [
            "format": "task/v1", "id": id, "version": 1, "name": name, "author": author,
            "created": f.string(from: created), "platform": "macos",
            "apps": [["key": appKey, "bundle": appBundle]],
            "inputs": inputs.map { ["name": $0.name, "type": $0.type, "source": "ask"] },
        ]
        if !secureFields.isEmpty {
            doc["preconditions"] = secureFields.map { d -> [String: Any] in
                var target = Self.dict(d); target["path"] = nil
                return ["app": appKey, "check": ["kind": "exists", "target": target]]
            }
        }
        var out: [[String: Any]] = []
        for (i, s) in steps.enumerated() {
            var o: [String: Any] = [
                "id": "s\(i + 1)", "say": s.say, "app": appKey, "action": s.action,
                "target": Self.dict(s.target), "consequential": s.consequential,
            ]
            if let n = s.inputName {
                o["value"] = ["input": n]
                o["expect"] = [["kind": "value_equals", "input": n]]
            } else {
                // A placeholder until the expect proposer (#14) replaces it: the control is still there.
                o["expect"] = [["kind": "exists", "target": Self.dict(s.target)]]
            }
            if let a = s.approval { o["approval"] = ["summary": a, "undoable": false] }
            out.append(o)
        }
        doc["steps"] = out
        return doc
    }

    public func json() throws -> Data {
        try JSONSerialization.data(withJSONObject: document(), options: [.prettyPrinted, .sortedKeys, .withoutEscapingSlashes])
    }
}
