import Foundation

/// A value from this run's inputs file. It never prints itself: string interpolation, `print` and
/// `dump` all show `<redacted>`. The only ways out are `reveal()`, used to type into the app and to
/// compare with what the app holds, and the approval gate's own display.
public struct SecretValue: CustomStringConvertible, CustomDebugStringConvertible, CustomReflectable, Equatable {
    private let text: String
    public init(_ text: String) { self.text = text }
    public var description: String { "<redacted>" }
    public var debugDescription: String { "<redacted>" }
    public var customMirror: Mirror { Mirror(self, children: []) }
    func reveal() -> String { text }
}

/// This run's inputs, read once from the `--inputs` file, held in memory only.
/// Nothing here writes a value anywhere: no file, no log, and no error message carries one.
public struct RunInputs {
    public static let maxValueLength = 256
    public static let maxFileBytes = 64 * 1024

    let values: [String: SecretValue]

    public struct Invalid: Error, CustomStringConvertible {
        public let description: String
        init(_ m: String) { description = m }
    }

    public init(values: [String: SecretValue]) { self.values = values }

    /// Reads `{ "name": "value", ... }` and checks it against the task's declared inputs.
    /// Errors name an input, never its value.
    public static func load(from data: Data, for task: ReplayTask) throws -> RunInputs {
        guard data.count <= maxFileBytes else { throw Invalid("inputs file is larger than \(maxFileBytes) bytes") }
        guard let obj = try? JSONSerialization.jsonObject(with: data) as? [String: Any] else {
            throw Invalid("inputs file must hold one JSON object of name: value")
        }
        var out: [String: SecretValue] = [:]
        for spec in task.inputs {
            guard let raw = obj[spec.name] else { throw Invalid("inputs file has no value for input \(spec.name)") }
            let text: String
            if let s = raw as? String { text = s }
            else if let n = raw as? NSNumber, CFGetTypeID(n) != CFBooleanGetTypeID() { text = n.stringValue }
            else { throw Invalid("input \(spec.name) must be a string or a number") }
            guard !text.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty else { throw Invalid("input \(spec.name) is empty") }
            guard text.count <= maxValueLength else { throw Invalid("input \(spec.name) is longer than \(maxValueLength) characters") }
            if spec.type == "choice", !(spec.choices ?? []).contains(text) {
                throw Invalid("input \(spec.name) is not one of its choices")
            }
            out[spec.name] = SecretValue(text)
        }
        if let extra = obj.keys.sorted().first(where: { out[$0] == nil }) {
            throw Invalid("inputs file has \(extra), which the task does not declare")
        }
        return RunInputs(values: out)
    }

    func secret(_ name: String) -> SecretValue? { values[name] }

    func resolve(_ ref: ReplayTask.ValueRef) -> SecretValue? {
        switch ref {
        case .input(let n): return values[n]
        case .constant(let c): return SecretValue(c)
        }
    }
}
