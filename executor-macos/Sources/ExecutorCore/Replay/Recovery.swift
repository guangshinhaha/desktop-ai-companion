import Foundation

/// Recovery from expected variation (#34, TD 5.4, decision 0003). When a step diverges, the executor asks
/// for help within strict bounds, and asks the officer when that is not enough.
///
/// Where a request goes. Over RPC it is the shell (`recover.request`), which passes it to the companion
/// service's `POST /recover`; the service checks the classification against the route's ceiling and calls the
/// model through its one adapter. The executor holds no model credentials and calls no model.
public protocol RecoverySource: AnyObject {
    /// The answer object, or nil when there is none (no service, a refusal, an error, a stop). Nil is never a yes.
    func ask(_ request: [String: Any]) -> [String: Any]?
}

/// What a run's recovery may send.
public struct RecoveryPolicy {
    public static let maxAttempts = 3
    /// The classification of the data this run handles, set by the task's owner through the shell.
    /// Nil or blank is unknown, and unknown counts as above the route's ceiling: nothing is sent.
    public var classification: String?
    /// True only when the route is cleared for that classification. Then field values may go. Nothing sets
    /// it before #19 names a route, so values are always masked for now.
    public var valuesCleared: Bool
    public init(classification: String? = nil, valuesCleared: Bool = false) {
        self.classification = classification; self.valuesCleared = valuesCleared
    }
    var knownClassification: String? {
        guard let c = classification?.trimmingCharacters(in: .whitespacesAndNewlines), !c.isEmpty else { return nil }
        return c
    }
}

/// The one function that builds a recovery request (decision 0003, "What a recovery request may carry").
/// Sent: the step's `say` line, action, descriptor and input name; a pruned tree of the window (roles, labels,
/// structure). Field values: masked unless the route is cleared. Password fields: never read, never sent.
/// This run's input values: never, not even inside a label. No screenshot, no credentials, no list rows: a
/// table, list or outline goes as its role and label only, with `rows_omitted`.
public enum RecoveryRequest {
    public static let maxNodes = 400
    public static let maxDepth = 40
    public static let maxLabel = 200
    static let masked = "[masked]"

    public struct Built {
        public let json: [String: Any]
        /// Each `ref` in the tree, with the node it stands for and its path of child indexes from the root.
        let refs: [String: (node: UINode, path: [Int])]
    }

    public struct Refusal: Error { public let reason: String }

    static let secureRoles: Set<String> = ["AXSecureTextField"]
    static let fieldRoles: Set<String> = ReplayTask.textRoles.union(ReplayTask.choiceRoles)
    /// Containers with nothing to say: their children are lifted into their parent.
    static let hoistable: Set<String> = ["AXGroup", "AXUnknown", "AXGenericElement", "AXSection", "AXLayoutArea", "AXSplitGroup"]
    /// Holders of list rows (decision 0003: no list rows). Their contents are never sent.
    static let rowHolders: Set<String> = ["AXTable", "AXOutline", "AXList", "AXGrid", "AXBrowser", "AXRow", "AXCell", "AXColumn"]

    /// Folded for comparison: case and diacritics ignored, every run of whitespace (NBSP included) one space.
    static func fold(_ s: String) -> String {
        s.folding(options: [.caseInsensitive, .diacriticInsensitive, .widthInsensitive], locale: nil)
            .components(separatedBy: .whitespacesAndNewlines).filter { !$0.isEmpty }.joined(separator: " ")
    }

    /// True when `text` holds one of `secrets` (already folded). Short values (under 3 characters) match a whole
    /// word, so a one-letter grade still masks a label that is that letter.
    static func holds(_ text: String, _ secrets: [String]) -> Bool {
        let t = fold(text)
        let words = Set(t.components(separatedBy: CharacterSet.alphanumerics.inverted).filter { !$0.isEmpty })
        return secrets.contains { $0.count >= 3 ? t.contains($0) : (t == $0 || words.contains($0)) }
    }

    public static func build(step: ReplayTask.Step, task: ReplayTask, inputs: RunInputs, root: UINode, divergence: String,
                             attempt: Int, policy: RecoveryPolicy, readValue: (UINode) -> String?) -> Swift.Result<Built, Refusal> {
        guard let classification = policy.knownClassification else {
            return .failure(Refusal(reason: "the task has no classification, and an unknown classification counts as above the model route's ceiling; nothing was sent"))
        }
        let secrets = inputs.values.values.map { fold($0.reveal()) }.filter { !$0.isEmpty }
        func clean(_ s: String?) -> (String?, Bool) {
            guard var s, !s.isEmpty else { return (nil, false) }
            if holds(s, secrets) { return (masked, true) }
            if s.count > maxLabel { s = String(s.prefix(maxLabel)) }
            return (s, false)
        }

        var refs: [String: (node: UINode, path: [Int])] = [:]
        var truncated = false
        func isSecure(_ n: UINode) -> Bool { secureRoles.contains(n.role) || n.subrole == "AXSecureTextField" }

        func emit(_ n: UINode, path: [Int], depth: Int) -> [[String: Any]] {
            if refs.count >= maxNodes { truncated = true; return [] }
            func kids() -> [[String: Any]] {
                guard depth < maxDepth else { if !n.children.isEmpty { truncated = true }; return [] }
                var out: [[String: Any]] = []
                for (i, c) in n.children.enumerated() { out += emit(c, path: path + [i], depth: depth + 1) }
                return out
            }
            if path.count > 0, hoistable.contains(n.role), n.label == nil, n.domId == nil, n.subrole == nil { return kids() }
            let ref = "n\(refs.count + 1)"
            refs[ref] = (n, path)
            var o: [String: Any] = ["ref": ref, "role": n.role]
            let (label, labelMasked) = clean(n.label)
            if let label { o["label"] = label }
            if labelMasked { o["label_masked"] = true }
            if let d = clean(n.domId).0 { o["dom_id"] = d }
            if let s = n.subrole { o["subrole"] = s }
            if isSecure(n) {
                o["secure"] = true // never read: no call to readValue
            } else if fieldRoles.contains(n.role) {
                if policy.valuesCleared, let v = readValue(n), !v.isEmpty {
                    let (value, valueMasked) = clean(v)
                    if valueMasked { o["value_masked"] = true } else if let value { o["value"] = value }
                } else {
                    o["value_masked"] = true
                }
            }
            if rowHolders.contains(n.role) {
                if !n.children.isEmpty { o["rows_omitted"] = true }
                return [o]
            }
            let c = kids()
            if !c.isEmpty { o["children"] = c }
            return [o]
        }
        let tree = emit(root, path: [], depth: 0).first ?? [:]

        var stepJSON: [String: Any] = ["id": step.id, "say": step.say, "action": step.action.rawValue,
                                       "target": descriptorJSON(step.target), "consequential": step.consequential.rawValue]
        if case .input(let name)? = step.value { stepJSON["input"] = name }
        var json: [String: Any] = [
            "kind": "recover",
            "classification": classification,
            "app": task.bundle,
            "attempt": attempt,
            "max_attempts": RecoveryPolicy.maxAttempts,
            "step": stepJSON,
            "divergence": divergence,
            "values": policy.valuesCleared ? "included" : "masked",
            "tree": tree,
        ]
        if truncated { json["truncated"] = true }
        return .success(Built(json: json, refs: refs))
    }

    static func descriptorJSON(_ d: Descriptor) -> [String: Any] {
        guard let data = try? JSONEncoder().encode(d), let o = try? JSONSerialization.jsonObject(with: data) as? [String: Any] else { return [:] }
        return o
    }
}

/// The model's answer (TD 5.4): act on a named element, wait again, or ask the officer.
public enum RecoveryAnswer: Equatable {
    public enum Kind: String { case press, setValue = "set_value" }
    case act(kind: Kind, ref: String)
    case retry
    case askOfficer(String)

    public struct Rejection: Error, Equatable {
        public let reason: String
        /// Not an answer at all. Asking again is pointless: the officer is asked at once.
        public let malformed: Bool
    }

    static let keys: Set<String> = ["answer", "action", "ref", "value", "app", "reason", "role", "label"]

    /// Strict. Anything not plainly one of the three answers is malformed and fails closed.
    static func parse(_ o: [String: Any], step: ReplayTask.Step, task: ReplayTask) -> Swift.Result<RecoveryAnswer, Rejection> {
        func bad(_ why: String) -> Swift.Result<RecoveryAnswer, Rejection> { .failure(Rejection(reason: why, malformed: true)) }
        func refuse(_ why: String) -> Swift.Result<RecoveryAnswer, Rejection> { .failure(Rejection(reason: why, malformed: false)) }
        if let k = o.keys.sorted().first(where: { !keys.contains($0) }) { return bad("the answer has a field this executor does not take (\(ApprovalGate.safe(String(k.prefix(40)))))") }
        guard let a = o["answer"] as? String else { return bad("the answer names no kind") }
        if let app = o["app"] {
            guard let s = app as? String, s == task.bundle || s == task.appKey else { return refuse("it would leave the recorded app") }
        }
        switch a {
        case "retry":
            guard o.count == 1 || (o.count == 2 && o["reason"] is String) else { return bad("a retry answer carries extra fields") }
            return .success(.retry)
        case "ask_officer":
            guard let r = o["reason"] as? String else { return bad("an ask_officer answer has no reason") }
            return .success(.askOfficer(String(ApprovalGate.safe(r).prefix(200))))
        case "act":
            guard let name = o["action"] as? String, let kind = Kind(rawValue: name) else { return bad("an act answer names no action this executor allows") }
            guard let ref = o["ref"] as? String, !ref.isEmpty else { return bad("an act answer names no element") }
            switch kind {
            case .press:
                if o["value"] != nil { return refuse("it types a value other than the step's own input") }
            case .setValue:
                guard let v = o["value"] as? [String: Any], v.count == 1, let n = v["input"] as? String,
                      step.action == .setValue, case .input(let own)? = step.value, n == own
                else { return refuse("it types a value other than the step's own input") }
            }
            return .success(.act(kind: kind, ref: ref))
        default:
            return bad("the answer is not act, retry or ask_officer")
        }
    }

    /// Labels of the only buttons recovery may press: ways to dismiss something, compared whole after folding.
    /// An allow-list, not a list of dangerous words: anything else ("Discard draft", "Go", "OK") is refused.
    static let dismissals: Set<String> = ["close", "dismiss", "cancel", "not now", "later", "maybe later", "skip",
                                          "got it", "no thanks", "no, thanks", "hide", "x", "×", "✕"]

    /// Could `node` be what `d` (a gated step's target) names? Same role, and the same label or dom id. No
    /// uniqueness is needed: two such buttons are both refused, not neither.
    static func couldBe(_ node: UINode, _ d: Descriptor) -> Bool {
        if let r = d.role, r != node.role { return false }
        if let l = d.label, let nl = node.label, RecoveryRequest.fold(l) == RecoveryRequest.fold(nl) { return true }
        if let id = d.domId, id == node.domId { return true }
        return false
    }

    /// Checks an act against the window as it is now. Returns the live node to act on, or why not.
    /// Refused: an element not in the tree that was sent, one that moved or changed, a link (it may leave the
    /// app), anything unlabelled, anything that reads like a commit, anything that could be the target of a step
    /// that needs approval, a password field, a button whose label is not a dismissal, and typing into anything
    /// but this step's own non-gated text field.
    static func check(_ kind: Kind, ref: String, built: RecoveryRequest.Built, step: ReplayTask.Step, task: ReplayTask,
                      now root: UINode) -> Swift.Result<UINode, Rejection> {
        func refuse(_ why: String) -> Swift.Result<UINode, Rejection> { .failure(Rejection(reason: why, malformed: false)) }
        guard let sent = built.refs[ref] else { return refuse("it names no element in the tree that was sent") }
        var live: UINode? = root
        for i in sent.path { live = live.flatMap { i < $0.children.count ? $0.children[i] : nil } }
        guard let node = live, node.role == sent.node.role, node.label == sent.node.label, node.domId == sent.node.domId else {
            return refuse("the element it names is not there any more")
        }
        if RecoveryRequest.secureRoles.contains(node.role) || node.subrole == "AXSecureTextField" { return refuse("it targets a password field") }
        if node.role == "AXLink" { return refuse("it targets a link, which may leave the recorded app") }
        guard let label = node.label, !label.trimmingCharacters(in: .whitespaces).isEmpty else {
            return refuse("it targets an unlabelled element, which may be consequential")
        }
        if ReplayTask.looksLikeCommit(label) { return refuse("it targets a consequential element") }
        for s in task.steps where s.needsApproval {
            var hit = couldBe(node, s.target)
            if case .found(let m) = Resolver.resolve(s.target, in: root), m.node === node { hit = true }
            if hit { return refuse("it targets a consequential element (it could be the target of a step that needs approval)") }
        }
        switch kind {
        case .press:
            switch node.role {
            case "AXDisclosureTriangle", "AXTab": break
            case "AXButton":
                guard dismissals.contains(RecoveryRequest.fold(label)) else {
                    return refuse("it presses a button that is not a way to dismiss something, which may be consequential")
                }
            default:
                return refuse("it presses an element of a kind recovery does not press (\(ApprovalGate.safe(node.role)))")
            }
        case .setValue:
            guard !step.needsApproval, ReplayTask.textRoles.contains(node.role) else { return refuse("it types into something other than this step's own field") }
            guard case .found(let m) = Resolver.resolve(step.target, in: root), m.node === node else {
                return refuse("it types into something other than this step's own field")
            }
        }
        return .success(node)
    }
}
