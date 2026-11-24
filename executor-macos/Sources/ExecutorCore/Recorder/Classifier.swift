import Foundation

public enum TargetKind: Equatable, Sendable {
    case text      // a field the officer types into
    case secure    // a password field: never read, never typed into the output
    case select    // a pop-up or combo box
    case press     // a button, link, checkbox or radio button
}

/// An element the officer acted on, reduced to what the task file may hold: a descriptor.
public struct Target: Equatable, Sendable {
    public var kind: TargetKind
    public var descriptor: Descriptor
    public init(kind: TargetKind, descriptor: Descriptor) { self.kind = kind; self.descriptor = descriptor }
}

/// Turns a live element into a `Target`. Reads role, subrole, labels, DOM id and position in the
/// tree. Never a field's value (see `ElementReading`).
public enum Classifier {
    static let secureRole = "AXSecureTextField"
    static let textRoles: Set<String> = ["AXTextField", "AXTextArea", "AXSearchField"]
    static let selectRoles: Set<String> = ["AXPopUpButton", "AXComboBox"]
    static let pressRoles: Set<String> = ["AXButton", "AXLink", "AXCheckBox", "AXRadioButton", "AXMenuButton", "AXTab"]
    static let maxLabel = 120
    static let maxClimb = 4
    static let maxPathNodes = 2000

    public static func isSecure(role: String?, subrole: String?) -> Bool {
        role == secureRole || subrole == secureRole
    }

    /// A text field that names itself a password but is not exposed as secure is treated as one.
    /// Wrong in the safe direction: the field is skipped and becomes a precondition.
    public static func looksLikePassword(label: String?, domId: String?) -> Bool {
        let hay = [label, domId].compactMap { $0 }.joined(separator: " ").lowercased()
        return ["password", "passcode", "passwd", "pwd", "kata laluan"].contains(where: hay.contains)
    }

    static let personalPatterns = [
        "[STFGMstfgm][0-9]{7}[A-Za-z]",                 // NRIC / FIN
        "[0-9]{6,}",                                    // long digit runs (phone, ids)
        "[A-Za-z0-9._%+-]+@[A-Za-z0-9.-]+",             // email address
    ]

    /// Labels and ids come from the page, so one can carry a person's data (a link named after a
    /// citizen). A label that contains an NRIC, an email or a long number is dropped. A name alone
    /// cannot be detected; the review screen (#26) is where the officer sees every label.
    public static func scrub(_ s: String?) -> String? {
        guard var t = s?.trimmingCharacters(in: .whitespacesAndNewlines), !t.isEmpty else { return nil }
        for p in personalPatterns where t.range(of: p, options: .regularExpression) != nil { return nil }
        // Braces mean "input reference" in a summary; a label never gets to be one.
        t = t.replacingOccurrences(of: "{", with: "(").replacingOccurrences(of: "}", with: ")")
        return t.count > maxLabel ? String(t.prefix(maxLabel)) : t
    }

    public static func kind(role: String?, subrole: String?) -> TargetKind? {
        if isSecure(role: role, subrole: subrole) { return .secure }
        guard let role else { return nil }
        if textRoles.contains(role) { return .text }
        if selectRoles.contains(role) { return .select }
        if pressRoles.contains(role) { return .press }
        return nil
    }

    /// The element itself, or the nearest ancestor that is an actionable control (a click often lands
    /// on the static text inside a button). Nil when none is.
    public static func classify(_ start: ElementReading) -> Target? {
        var el: ElementReading? = start
        for _ in 0...maxClimb {
            guard let cur = el else { return nil }
            let role = cur.string("AXRole")
            let subrole = cur.string("AXSubrole")
            if let kind = kind(role: role, subrole: subrole), let role {
                let domId = scrub(cur.string("AXDOMIdentifier"))
                if kind == .secure {
                    // Name and role only: title or linked label, never a placeholder. No path walk.
                    let d = Descriptor(role: role, label: label(of: cur, role: role, secure: true), domId: domId)
                    return Target(kind: .secure, descriptor: d)
                }
                let lbl = label(of: cur, role: role, secure: false)
                if kind == .text, looksLikePassword(label: lbl, domId: domId) {
                    return Target(kind: .secure, descriptor: Descriptor(role: role, label: lbl, domId: domId))
                }
                let d = Descriptor(role: role, label: lbl, domId: domId, path: path(to: cur, role: role))
                return Target(kind: kind, descriptor: d)
            }
            if role == "AXWindow" || role == "AXWebArea" { return nil }
            el = cur.element("AXParent")
        }
        return nil
    }

    /// Title, description, the linked label element, then placeholder. Same order as the resolver's snapshot.
    static func label(of el: ElementReading, role: String, secure: Bool) -> String? {
        var l = el.string("AXTitle") ?? el.string("AXDescription")
        if l == nil, let t = el.element("AXTitleUIElement") {
            l = t.string("AXTitle") ?? t.staticText()
        }
        if l == nil, !secure { l = el.string("AXPlaceholderValue") }
        if l == nil, !secure, ["AXButton", "AXLink", "AXMenuButton", "AXTab"].contains(role) {
            for c in el.children().prefix(5) {
                if c.string("AXRole") == "AXStaticText", let t = c.staticText() { l = t; break }
            }
        }
        return scrub(l)
    }

    /// `["AXWebArea", "AXTextField:2"]`: the web area (if any), then the Nth element of this role below
    /// it, document order. The resolver's locator 4 reads exactly this shape.
    static func path(to el: ElementReading, role: String) -> [String]? {
        var container: ElementReading?
        var p = el.element("AXParent")
        var hops = 0
        while let cur = p, hops < 60 {
            let r = cur.string("AXRole")
            if r == "AXWebArea" { container = cur; break }
            if r == "AXWindow" { container = cur; break }
            p = cur.element("AXParent"); hops += 1
        }
        guard let container else { return nil }
        var visited = 0
        var index = 0
        var stack: [ElementReading] = container.children().reversed()
        while let n = stack.popLast() {
            visited += 1
            if visited > maxPathNodes { return nil }
            if n.string("AXRole") == role {
                index += 1
                if n.same(as: el) {
                    return container.string("AXRole") == "AXWebArea" ? ["AXWebArea", "\(role):\(index)"] : ["\(role):\(index)"]
                }
            }
            stack.append(contentsOf: n.children().reversed())
        }
        return nil
    }

    /// Keys that do not put a character into a field. The key code is used to classify and is never kept.
    static let nonTextKeyCodes: Set<Int64> = [
        36, 76,                 // return, enter
        48,                     // tab
        53,                     // escape
        114, 115, 116, 117, 119, 121, // help, home, page up, forward delete, end, page down
        123, 124, 125, 126,     // arrows
        122, 120, 99, 118, 96, 97, 98, 100, 101, 109, 103, 111, 105, 107, 113, 106, 64, 79, 80, 90, // F1-F20
    ]

    public static func isActivationKey(keyCode: Int64) -> Bool { keyCode == 49 || isSubmitKey(keyCode: keyCode) }
    public static func isPasteKey(keyCode: Int64, command: Bool) -> Bool { command && keyCode == 9 }

    public static func isTextKey(keyCode: Int64, command: Bool, control: Bool) -> Bool {
        !command && !control && !nonTextKeyCodes.contains(keyCode)
    }

    public static func isSubmitKey(keyCode: Int64) -> Bool { keyCode == 36 || keyCode == 76 }
}
