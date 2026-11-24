import Foundation

/// TD section 5.2: which locator found the element.
public enum Locator: Int, Sendable {
    case domId = 1
    case roleExactLabel = 2
    case roleFuzzyLabelAnchors = 3
    case path = 4

    public var summary: String {
        switch self {
        case .domId: return "dom_id"
        case .roleExactLabel: return "role + exact label"
        case .roleFuzzyLabelAnchors: return "role + fuzzy label + anchors"
        case .path: return "recorded path"
        }
    }
}

public struct Match {
    public let node: UINode
    public let locator: Locator
}

public struct Ambiguity {
    public let locator: Locator
    public let candidates: [UINode]
}

public enum Resolution {
    case found(Match)
    /// Two or more equally good matches. A divergence: the resolver never picks one.
    case ambiguous(Ambiguity)
    case notFound
}

/// Finds a recorded element by what it is. Locators 1 to 4 of TD section 5.2, in order.
/// The first locator that finds anything decides: one match is found, two or more is
/// ambiguous. Locator 5 (vision) is Phase 1.
public enum Resolver {
    /// Below this a fuzzy label is not a match at all. Set high on purpose: Approve/Disapprove
    /// and Enable/Disable must never count as the same control.
    static let fuzzyThreshold = 0.85
    /// Labels longer than this are never fuzzy-matched (bounds time and memory on hostile pages).
    static let maxLabelLength = 2000
    /// Fuzzy candidates within this of the best score count as equally good.
    static let fuzzyMargin = 0.1

    public static func resolve(_ d: Descriptor, in root: UINode) -> Resolution {
        let all = root.descendants
        let steps: [(Locator, () -> Outcome)] = [
            (.domId, { byDomId(d, all) }),
            (.roleExactLabel, { byRoleExactLabel(d, all) }),
            (.roleFuzzyLabelAnchors, { byFuzzyLabel(d, all) }),
            (.path, { byPath(d, root) }),
        ]
        for (locator, run) in steps {
            switch run() {
            case .none: continue
            case .one(let n): return .found(Match(node: n, locator: locator))
            case .many(let ns): return .ambiguous(Ambiguity(locator: locator, candidates: ns))
            }
        }
        return .notFound
    }

    /// Re-resolves until the element is found or ambiguous, or the step ceiling passes.
    /// `wait(timeout)` blocks until an accessibility event arrives or `timeout` seconds pass
    /// (TD 5.3: event first, then a bounded backoff from 100 ms to 2 s for apps that miss events).
    /// No fixed delay: a found element returns at once.
    public static func resolve(
        _ d: Descriptor,
        ceiling: TimeInterval,
        snapshot: () -> UINode?,
        wait: (TimeInterval) -> Void,
        now: () -> TimeInterval
    ) -> Resolution {
        let deadline = now() + ceiling
        var backoff = 0.1
        while true {
            if let root = snapshot() {
                let r = resolve(d, in: root)
                if case .notFound = r {} else { return r }
            }
            let remaining = deadline - now()
            if remaining <= 0 { return .notFound }
            wait(min(backoff, remaining))
            backoff = min(backoff * 2, 2.0)
        }
    }

    // MARK: locators

    enum Outcome { case none, one(UINode), many([UINode]) }

    static func outcome(_ ns: [UINode]) -> Outcome {
        ns.isEmpty ? .none : (ns.count == 1 ? .one(ns[0]) : .many(ns))
    }

    static func byDomId(_ d: Descriptor, _ all: [UINode]) -> Outcome {
        guard let id = d.domId else { return .none }
        return outcome(all.filter { $0.domId == id })
    }

    static func byRoleExactLabel(_ d: Descriptor, _ all: [UINode]) -> Outcome {
        guard let role = d.role, let label = d.label else { return .none }
        let want = squash(label)
        return outcome(all.filter { $0.role == role && $0.label.map(squash) == want })
    }

    static func byFuzzyLabel(_ d: Descriptor, _ all: [UINode]) -> Outcome {
        guard d.label != nil || d.labelMatches != nil else { return .none }
        let anchors = (d.anchors ?? []).filter { !squash($0).isEmpty }
        // A fuzzy label alone is a guess. TD 5.2 scopes it by anchors; an explicit
        // label_matches pattern is the other way to say what "close enough" means.
        if anchors.isEmpty && d.labelMatches == nil { return .none }

        func score(_ n: UINode) -> Double? {
            if let role = d.role, n.role != role { return nil }
            guard let l = n.label, !squash(l).isEmpty, l.count <= maxLabelLength else { return nil }
            var s = 1.0
            if let g = d.labelMatches {
                if !glob(g, l) { return nil }
            }
            if let want = d.label {
                s = similarity(want, l)
                if s < fuzzyThreshold { return nil }
            }
            return s
        }
        let scored = all.compactMap { n in score(n).map { (n, $0) } }
        guard let best = scored.map(\.1).max() else { return .none }
        var contenders = scored.filter { best - $0.1 <= fuzzyMargin }.map(\.0)

        if !anchors.isEmpty {
            // Every anchor must be on screen, or this locator has no evidence to go on.
            let anchorNodes = anchors.map { a in all.filter { n in n.label.map { $0.count <= maxLabelLength && similarity(a, $0) >= 0.9 } ?? false } }
            if anchorNodes.contains(where: { $0.isEmpty }) { return .none }
            if contenders.count > 1 {
                // Anchors break a tie only when one candidate sits clearly nearest to them.
                let dist = contenders.map { c in
                    anchorNodes.reduce(0) { sum, group in
                        sum + (group.filter { $0 !== c }.map { treeDistance(c, $0) }.min() ?? unreachable)
                    }
                }
                let nearest = dist.min()!
                contenders = zip(contenders, dist).filter { $0.1 == nearest }.map(\.0)
            }
        }
        return outcome(contenders)
    }

    static func byPath(_ d: Descriptor, _ root: UINode) -> Outcome {
        guard let path = d.path, !path.isEmpty else { return .none }
        var current = root
        for seg in path {
            let (role, qualifier) = splitSegment(seg)
            let matches = depthTagged(current).filter { $0.node.role == role }
            var chosen: [UINode]
            if let q = qualifier, let idx = Int(q) {
                chosen = idx >= 1 && idx <= matches.count ? [matches[idx - 1].node] : []
            } else if let q = qualifier {
                let ms = matches.filter { $0.node.label.map(squash) == squash(q) }
                chosen = nearestLevel(ms)
            } else {
                chosen = nearestLevel(matches)
            }
            if chosen.isEmpty { return .none }
            if chosen.count > 1 { return .many(chosen) }
            current = chosen[0]
        }
        if let role = d.role, current.role != role { return .none }
        // A path says where; the label still has to agree, or this is a different control.
        if let want = d.label {
            guard let l = current.label, l.count <= maxLabelLength, similarity(want, l) >= fuzzyThreshold else { return .none }
        }
        if let g = d.labelMatches {
            guard let l = current.label, l.count <= maxLabelLength, glob(g, l) else { return .none }
        }
        return .one(current)
    }

    // MARK: helpers

    static func splitSegment(_ s: String) -> (String, String?) {
        guard let i = s.firstIndex(of: ":") else { return (s, nil) }
        return (String(s[..<i]), String(s[s.index(after: i)...]))
    }

    static func depthTagged(_ root: UINode) -> [(node: UINode, depth: Int)] {
        var out: [(UINode, Int)] = []
        var stack = root.children.reversed().map { ($0, 1) }
        while let (n, d) = stack.popLast() {
            out.append((n, d))
            stack.append(contentsOf: n.children.reversed().map { ($0, d + 1) })
        }
        return out
    }

    static func nearestLevel(_ ms: [(node: UINode, depth: Int)]) -> [UINode] {
        guard let d = ms.map(\.depth).min() else { return [] }
        return ms.filter { $0.depth == d }.map(\.node)
    }

    /// Distance when two nodes share no ancestor, or an anchor has nothing else to measure to.
    /// Small enough that sums over many anchors cannot overflow.
    static let unreachable = 1_000_000

    static func treeDistance(_ a: UINode, _ b: UINode) -> Int {
        let chainA = [a] + a.ancestors()
        let chainB = [b] + b.ancestors()
        for (i, x) in chainA.enumerated() {
            if let j = chainB.firstIndex(where: { $0 === x }) { return i + j }
        }
        return unreachable
    }

    /// Trim and collapse whitespace. Case stays: "exact" means exact.
    static func squash(_ s: String) -> String {
        s.split(whereSeparator: { $0.isWhitespace }).joined(separator: " ")
    }

    static func normalize(_ s: String) -> String {
        let folded = s.folding(options: [.caseInsensitive, .diacriticInsensitive], locale: nil)
        let mapped = folded.map { $0.isLetter || $0.isNumber ? $0 : " " }
        return squash(String(mapped))
    }

    /// 1.0 for equal text (ignoring case and punctuation). 0.9 when one label's words are all in
    /// the other and exactly one word differs ("Create" and "Create account"), unless that word
    /// negates ("Don't approve" is not "Approve"). Otherwise the bigram overlap (Dice).
    static func similarity(_ a: String, _ b: String) -> Double {
        let x = normalize(a), y = normalize(b)
        if x.isEmpty || y.isEmpty { return 0 }
        if x == y { return 1 }
        let tx = Set(x.split(separator: " ")), ty = Set(y.split(separator: " "))
        if tx.isSubset(of: ty) || ty.isSubset(of: tx) {
            let extra = tx.symmetricDifference(ty)
            return extra.count == 1 && extra.isDisjoint(with: negations) ? 0.9 : 0
        }
        if tx.isDisjoint(with: negations) != ty.isDisjoint(with: negations) { return 0 }
        func bigrams(_ s: String) -> [String] {
            let c = Array(s)
            return c.count < 2 ? [s] : (0..<(c.count - 1)).map { String(c[$0...$0 + 1]) }
        }
        let bx = bigrams(x), by = bigrams(y)
        var pool: [String: Int] = [:]
        for g in by { pool[g, default: 0] += 1 }
        var common = 0
        for g in bx where (pool[g] ?? 0) > 0 { pool[g]! -= 1; common += 1 }
        return 2.0 * Double(common) / Double(bx.count + by.count)
    }

    static let negations: Set<Substring> = ["not", "no", "don", "dont", "cannot", "cant", "never", "cancel"]

    /// `*` any run, `?` any one character, case-insensitive, whole label.
    /// Iterative with one backtrack point, so a long label cannot exhaust the stack.
    static func glob(_ pattern: String, _ text: String) -> Bool {
        let p = Array(squash(pattern).lowercased()), t = Array(squash(text).lowercased())
        var i = 0, j = 0, star = -1, mark = 0
        while j < t.count {
            if i < p.count, p[i] == "*" { star = i; mark = j; i += 1 }
            else if i < p.count, p[i] == "?" || p[i] == t[j] { i += 1; j += 1 }
            else if star >= 0 { i = star + 1; mark += 1; j = mark }
            else { return false }
        }
        while i < p.count, p[i] == "*" { i += 1 }
        return i == p.count
    }
}
