import Foundation

/// A task/v1 document read for replay (schema/task.v1.json, TD section 4).
/// Parsing is strict on purpose: anything this executor cannot do safely stops the run before
/// the first action, so a task never half-runs because of a step it does not understand.
public struct ReplayTask {
    public enum ValueRef: Equatable {
        case input(String)
        /// A constant the recorder confirmed holds no personal data.
        case constant(String)
    }

    public struct Check {
        public enum Kind: String { case exists, appears, disappears, valueEquals = "value_equals", windowTitle = "window_title" }
        public var kind: Kind
        public var target: Descriptor?
        public var matches: String?
        public var value: ValueRef?
    }

    public enum Consequential: String { case yes, no, unknown }

    public enum Action: String { case press, setValue = "set_value", select }

    public struct Step {
        public var id: String
        public var say: String
        public var action: Action
        public var target: Descriptor
        public var value: ValueRef?
        public var expect: [Check]
        public var consequential: Consequential
        public var approvalSummary: String?
        public var undoable: Bool
        public var timeoutSeconds: Double?

        /// A step needs approval unless the task says `no` AND it is plainly typing into a text field or
        /// choosing in a pop-up. `unknown` counts as `yes`. The task's flag is not trusted beyond that: a
        /// press never runs without approval in Phase 0, a set_value or select aimed at anything but a
        /// field is gated, and a label that reads like a commit is gated. The replayer also checks the
        /// live element, because a descriptor may carry no label at all.
        public var needsApproval: Bool {
            if consequential != .no { return true }
            switch action {
            case .press: return true
            case .setValue: if !(target.role.map(ReplayTask.textRoles.contains) ?? false) { return true }
            case .select: if !(target.role.map(ReplayTask.choiceRoles.contains) ?? false) { return true }
            }
            return target.label.map(ReplayTask.looksLikeCommit) ?? false
        }
    }

    static let textRoles: Set<String> = ["AXTextField", "AXTextArea", "AXSearchField", "AXComboBox"]
    static let choiceRoles: Set<String> = ["AXPopUpButton", "AXComboBox", "AXMenuButton"]

    /// Words that commit. Matched with invisible characters removed and camelCase split, so
    /// "Dele\u{200B}te" and "DeleteAccount" count. Long stems also match inside a longer word.
    /// Wrongly gating is the safe error; wrongly not gating is not.
    static let commitWords: Set<String> = [
        "create", "submit", "send", "delete", "remove", "approve", "confirm", "save", "publish", "pay", "transfer",
        "grant", "reset", "revoke", "post", "apply", "finish", "complete", "accept", "sign", "register", "enrol",
        "enroll", "buy", "purchase", "order", "merge", "deploy", "release", "share", "invite", "yes", "ok", "done",
        "commit", "update", "change", "disable", "enable", "activate", "deactivate", "assign", "provision", "submitted",
    ]
    static let commitStems = ["create", "submit", "delete", "remove", "approve", "confirm", "publish", "transfer",
                              "revoke", "purchase", "register", "provision", "activate"]

    public static func looksLikeCommit(_ label: String) -> Bool {
        // Invisible and combining characters are dropped, not turned into spaces, so they cannot split a word.
        var kept = ""
        var prevLower = false
        for u in label.precomposedStringWithCompatibilityMapping.unicodeScalars {
            switch u.properties.generalCategory {
            case .format, .control, .nonspacingMark, .enclosingMark: continue
            default: break
            }
            let ch = Character(u)
            if ch.isLetter {
                if ch.isUppercase, prevLower { kept.append(" ") }
                prevLower = ch.isLowercase
                kept.append(ch)
            } else {
                prevLower = false
                kept.append(" ")
            }
        }
        let words = kept.lowercased().split(separator: " ").map(String.init)
        if words.contains(where: commitWords.contains) { return true }
        let squashed = words.joined()
        return commitStems.contains { squashed.contains($0) }
    }

    public struct InputSpec: Equatable {
        public var name: String
        public var type: String
        public var choices: [String]?
    }

    public var id: String
    public var name: String
    public var appKey: String
    public var bundle: String
    public var inputs: [InputSpec]
    public var preconditions: [Check]
    public var steps: [Step]

    public struct Invalid: Error, CustomStringConvertible {
        public let description: String
        init(_ m: String) { description = m }
    }

    // MARK: parsing

    public static func load(from data: Data) throws -> ReplayTask {
        guard data.count <= maxTaskBytes else { throw Invalid("task file is larger than \(maxTaskBytes) bytes") }
        guard let root = try? JSONSerialization.jsonObject(with: data) as? [String: Any] else {
            throw Invalid("task file must hold a JSON object")
        }
        guard root["format"] as? String == "task/v1" else { throw Invalid("format must be task/v1") }
        guard root["platform"] as? String == "macos" else { throw Invalid("this executor replays macos tasks only") }
        let id = (root["id"] as? String) ?? "task"
        let name = (root["name"] as? String) ?? id

        guard let apps = root["apps"] as? [[String: Any]], !apps.isEmpty else { throw Invalid("task has no apps") }
        guard apps.count == 1, let key = apps[0]["key"] as? String, let bundle = apps[0]["bundle"] as? String else {
            throw Invalid("Phase 0 replays a task in exactly one app; this task names \(apps.count)")
        }

        var inputs: [InputSpec] = []
        for i in (root["inputs"] as? [[String: Any]]) ?? [] {
            guard let n = i["name"] as? String, let t = i["type"] as? String else { throw Invalid("an input needs a name and a type") }
            if inputs.contains(where: { $0.name == n }) { throw Invalid("input \(n) is declared twice") }
            inputs.append(InputSpec(name: n, type: t, choices: i["choices"] as? [String]))
        }
        let declared = Set(inputs.map(\.name))

        var pre: [Check] = []
        for p in (root["preconditions"] as? [[String: Any]]) ?? [] {
            guard p["app"] as? String == key else { throw Invalid("a precondition names an app this task does not use") }
            guard let c = p["check"] as? [String: Any] else { throw Invalid("a precondition has no check") }
            pre.append(try parseCheck(c, declared: declared, where: "a precondition"))
        }

        guard let rawSteps = root["steps"] as? [[String: Any]], !rawSteps.isEmpty else { throw Invalid("task has no steps") }
        var steps: [Step] = []
        var seen = Set<String>()
        for s in rawSteps {
            let step = try parseStep(s, appKey: key, declared: declared)
            guard seen.insert(step.id).inserted else { throw Invalid("step id \(step.id) appears twice") }
            steps.append(step)
        }
        return ReplayTask(id: id, name: name, appKey: key, bundle: bundle, inputs: inputs, preconditions: pre, steps: steps)
    }

    static func parseStep(_ s: [String: Any], appKey: String, declared: Set<String>) throws -> Step {
        guard let id = s["id"] as? String else { throw Invalid("a step has no id") }
        let w = "step \(id)"
        guard s["app"] as? String == appKey else { throw Invalid("\(w) uses another app; Phase 0 replays one app") }
        guard let say = s["say"] as? String, !say.isEmpty else { throw Invalid("\(w) has no say line") }
        guard let actionName = s["action"] as? String else { throw Invalid("\(w) has no action") }
        // type_keys can press Return, which submits a form without any button the gate could see.
        // open_url, focus_app, read_value and wait_for are not built yet. All of them refuse.
        guard let action = Action(rawValue: actionName) else {
            throw Invalid("\(w) uses action \(actionName), which this executor does not run yet")
        }
        guard let cons = (s["consequential"] as? String).flatMap(Consequential.init(rawValue:)) else {
            throw Invalid("\(w) needs consequential to be yes, no or unknown")
        }
        guard let t = s["target"],
              let target = try? Descriptor.load(from: JSONSerialization.data(withJSONObject: ["target": t])) else {
            throw Invalid("\(w) needs a usable target")
        }
        if (target.anchors?.count ?? 0) > maxAnchors { throw Invalid("\(w) target has more than \(maxAnchors) anchors") }
        // Passwords are never an input (PRD R-2): the recorder turns them into a precondition.
        if target.role == "AXSecureTextField" { throw Invalid("\(w) targets a password field, which replay never fills") }
        var value: ValueRef?
        if let v = s["value"] { value = try parseValue(v, declared: declared, where: w) }
        switch action {
        case .press: if value != nil { throw Invalid("\(w) presses a control and cannot carry a value") }
        case .setValue, .select: if value == nil { throw Invalid("\(w) needs a value") }
        }
        guard let ex = s["expect"] as? [[String: Any]], !ex.isEmpty else { throw Invalid("\(w) has no expect check") }
        let checks = try ex.map { try parseCheck($0, declared: declared, where: w) }

        var summary: String?
        var undoable = false
        if let a = s["approval"] as? [String: Any] {
            summary = a["summary"] as? String
            undoable = (a["undoable"] as? Bool) ?? false
        }
        let step = Step(id: id, say: say, action: action, target: target, value: value, expect: checks, consequential: cons,
                        approvalSummary: summary, undoable: undoable, timeoutSeconds: try parseTimeout(s["timeout_seconds"], where: w))
        // A gated step with no summary is allowed: the card says so and still shows what will be done.
        if let sum = summary {
            for name in placeholders(in: sum) where !declared.contains(name) {
                throw Invalid("\(w) approval summary names input \(name), which the task does not declare")
            }
        }
        if let t = step.timeoutSeconds, !(t > 0 && t <= 3600) { throw Invalid("\(w) timeout_seconds must be above 0 and at most 3600") }
        return step
    }

    /// Absent is fine. Present must be a real number, not a bool, string or null.
    static func parseTimeout(_ v: Any?, where w: String) throws -> Double? {
        guard let v else { return nil }
        guard let n = v as? NSNumber, CFGetTypeID(n) != CFBooleanGetTypeID() else {
            throw Invalid("\(w) timeout_seconds must be a number")
        }
        return n.doubleValue
    }

    static let maxAnchors = 16
    static let maxTaskBytes = 1_000_000

    static func parseValue(_ v: Any, declared: Set<String>, where w: String) throws -> ValueRef {
        guard let o = v as? [String: Any] else { throw Invalid("\(w) has a value that is not {input} or {constant}") }
        if let n = o["input"] as? String, o.count == 1 {
            guard declared.contains(n) else { throw Invalid("\(w) uses input \(n), which the task does not declare") }
            return .input(n)
        }
        if o["confirmed_no_personal_data"] as? Bool == true, o.count == 2, let c = o["constant"] {
            if let s = c as? String { return .constant(s) }
            if let n = c as? NSNumber { return .constant(n.stringValue) }
        }
        throw Invalid("\(w) has a value that is not {input} or a confirmed {constant}")
    }

    static func parseCheck(_ c: [String: Any], declared: Set<String>, where w: String) throws -> Check {
        guard let kindName = c["kind"] as? String else { throw Invalid("\(w) has a check with no kind") }
        guard let kind = Check.Kind(rawValue: kindName) else {
            throw Invalid("\(w) uses check kind \(kindName), which this executor does not evaluate yet")
        }
        var target: Descriptor?
        if let t = c["target"] {
            guard let d = try? Descriptor.load(from: JSONSerialization.data(withJSONObject: ["target": t])) else {
                throw Invalid("\(w) has a check with an unusable target")
            }
            if (d.anchors?.count ?? 0) > maxAnchors { throw Invalid("\(w) has a check target with more than \(maxAnchors) anchors") }
            target = d
        }
        var value: ValueRef?
        if kind == .valueEquals {
            if let n = c["input"] as? String {
                guard declared.contains(n) else { throw Invalid("\(w) checks input \(n), which the task does not declare") }
                value = .input(n)
            } else if c["confirmed_no_personal_data"] as? Bool == true, let k = c["constant"] {
                value = .constant((k as? String) ?? ((k as? NSNumber)?.stringValue ?? ""))
            } else {
                throw Invalid("\(w) has a value_equals check with no input or confirmed constant")
            }
        }
        switch kind {
        case .exists, .appears, .disappears: if target == nil { throw Invalid("\(w) has a \(kindName) check with no target") }
        case .windowTitle: if (c["matches"] as? String)?.isEmpty ?? true { throw Invalid("\(w) has a window_title check with no matches") }
        case .valueEquals: break
        }
        return Check(kind: kind, target: target, matches: c["matches"] as? String, value: value)
    }

    /// `{name}` tokens in a summary, in order.
    public static func placeholders(in s: String) -> [String] {
        var out: [String] = []
        var rest = Substring(s)
        while let open = rest.firstIndex(of: "{") {
            guard let close = rest[open...].firstIndex(of: "}") else { break }
            out.append(String(rest[rest.index(after: open)..<close]))
            rest = rest[rest.index(after: close)...]
        }
        return out
    }
}
