import Foundation

/// Proposes a step's `expect` from what changed in the focused window between two snapshots (TD 5.1 step 4).
///
/// Privacy, by construction:
/// - A snapshot (`UINode`) holds roles and labels, never a field's value.
/// - Only text in a heading or a dialog is considered (not plain paragraphs, where a page echoes a
///   record), and never anything inside a text field, pop-up or other input, so a field's content
///   cannot become a check.
/// - A label that looks like data (digits, an email, a list of values, a long sentence) is not stable
///   enough to check and is dropped. A page that echoes a person's name in a short plain label is
///   the case this cannot see; the officer sees every check on the review screen (#26).
public enum ExpectProposer {
    static let inputRoles: Set<String> = [
        "AXTextField", "AXTextArea", "AXSearchField", "AXSecureTextField", "AXComboBox",
        "AXPopUpButton", "AXMenu", "AXMenuButton", "AXList", "AXTable", "AXOutline", "AXSlider",
        "AXGrid", "AXRow", "AXCell", "AXColumn",
    ]
    static let textRoles: Set<String> = ["AXStaticText", "AXHeading"]
    static let maxWords = 6
    static let maxChars = 60

    /// One of the kinds this proposer writes, as a task/v1 check object.
    public struct Proposal: Equatable {
        public enum Kind: String { case appears, disappears, windowTitle = "window_title" }
        public var kind: Kind
        public var role: String?
        public var label: String
        public func json() -> [String: Any] {
            switch kind {
            case .windowTitle: return ["kind": kind.rawValue, "matches": label]
            default: return ["kind": kind.rawValue, "target": ["role": role ?? "AXStaticText", "label": label]]
            }
        }
    }

    /// The most specific stable change from `before` to `after`, or nil when there is none.
    /// Order: something appeared, else something disappeared, else the window title changed.
    public static func propose(before: UINode, after: UINode) -> Proposal? {
        let b = candidates(before), a = candidates(after)
        let appeared = added(from: b, to: a)
        if let best = best(appeared) { return Proposal(kind: .appears, role: best.role, label: best.label) }
        let gone = added(from: a, to: b)
        if let best = best(gone) { return Proposal(kind: .disappears, role: best.role, label: best.label) }
        if let t = stable(after.label), t != stable(before.label), !t.contains("*"), !t.contains("?") {
            return Proposal(kind: .windowTitle, role: nil, label: t)
        }
        return nil
    }

    // MARK: candidates

    struct Candidate {
        var role: String
        var label: String
        var score: Int
        var key: String { role + "\u{1}" + label }
    }

    /// Candidates in `new` whose text is nowhere in `old`. A set difference, not a count: a label that
    /// is on screen both times (twice, then once) gives a check that is already true, or never will be.
    static func added(from old: [Candidate], to new: [Candidate]) -> [Candidate] {
        let seen = Set(old.map { $0.key })
        return new.filter { !seen.contains($0.key) }
    }

    /// Highest score, then the earlier in document order.
    static func best(_ cs: [Candidate]) -> Candidate? {
        var pick: Candidate?
        for c in cs where pick == nil || c.score > pick!.score { pick = c }
        return pick
    }

    static func candidates(_ root: UINode) -> [Candidate] {
        // A browser window holds toolbar and tabs around the page. When there is a page, only it counts.
        var starts = [root]
        let pages = ([root] + root.descendants).filter { $0.role == "AXWebArea" }
        if !pages.isEmpty { starts = pages }
        var out: [Candidate] = []
        for s in starts { walk(s, heading: false, dialog: false, into: &out) }
        return out
    }

    static func walk(_ n: UINode, heading: Bool, dialog: Bool, into out: inout [Candidate]) {
        if inputRoles.contains(n.role) { return }
        let inHeading = heading || n.role == "AXHeading"
        let inDialog = dialog || n.role == "AXDialog" || n.role == "AXSheet"
        // Only what a page says as a heading or in a dialog. Plain paragraphs are where a page echoes
        // a person's record back, so they are never a check.
        if textRoles.contains(n.role), inHeading || inDialog, let l = stable(n.label) {
            // A heading that only wraps a static text of the same words is that text; take the text.
            let wrapped = n.role == "AXHeading" && n.children.contains { $0.role == "AXStaticText" && stable($0.label) == l }
            if !wrapped {
                out.append(Candidate(role: n.role, label: l, score: (inHeading ? 4 : 0) + (inDialog ? 2 : 0)))
            }
        }
        for c in n.children { walk(c, heading: inHeading, dialog: inDialog, into: &out) }
    }

    /// The label if it is plain, short, fixed text: no digits (counters, times, ids), no email,
    /// no brackets or commas (a record echoed back), nothing `Classifier.scrub` drops, and no two
    /// capitalised words in a row (a person's name: "Welcome Aiden Tan"). Labels are folded to
    /// compatibility form first, so look-alike digits and a fullwidth @ count as what they imitate.
    /// Fixed text in sentence case ("Account created") passes; Title Case does not, and falls back
    /// to the control-is-still-there check.
    static func stable(_ s: String?) -> String? {
        guard let s, let t = Classifier.scrub(s.precomposedStringWithCompatibilityMapping), t.count <= maxChars else { return nil }
        if t.rangeOfCharacter(from: .decimalDigits) != nil { return nil }
        if t.rangeOfCharacter(from: CharacterSet(charactersIn: "@()[],;|<>")) != nil { return nil }
        let words = t.split(whereSeparator: { $0.isWhitespace })
        if words.count > maxWords { return nil }
        var run = 0
        for w in words {
            if let f = w.first, f.isUppercase { run += 1; if run >= 2 { return nil } } else { run = 0 }
        }
        return t
    }
}
