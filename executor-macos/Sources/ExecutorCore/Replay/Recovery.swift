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
/// Sent: the step's place in the task, action, consequence flag, descriptor (under the tree's rules) and input
/// name (an identifier); why it diverged as a fixed code, never free text; a pruned tree of the window (roles and
/// structure always). The step's `say` line, its id and its checks are task text and are not sent. Text is sent only where it names a control, never where it is the page's content:
/// a label or dom id goes only for control chrome (`chromeRoles`: buttons, tabs, form fields, toggles, the
/// window, a sheet or dialog, a heading inside a dialog). Static text, rows, cells, list items, links and every
/// other role go as role and structure only, so other people's data on screen does not leave. Roles and
/// subroles outside known sets go as `AXUnknown` / are dropped. Field values: masked unless the route is
/// cleared. Password fields: never read. As a second layer, any sent text holding one of this run's input
/// values (folded to letters and digits, word by word, email local part) is masked. No screenshot, no
/// credentials, no list rows: a table, list or outline goes without its rows (`rows_omitted`).
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

    /// The only roles whose label (and dom id) is sent: text that names a control, not the page's content.
    static let chromeRoles: Set<String> = ReplayTask.textRoles.union(ReplayTask.choiceRoles).union([
        "AXButton", "AXTab", "AXDisclosureTriangle", "AXCheckBox", "AXRadioButton", "AXPopUpButton", "AXMenuButton",
        "AXComboBox", "AXSecureTextField", "AXWindow", "AXSheet", "AXDialog"])
    /// Subroles that mark a dialog: its own label (its title) and its headings are chrome too.
    static let dialogSubroles: Set<String> = ["AXApplicationDialog", "AXDialog", "AXSystemDialog"]
    /// The only roles a dialog subrole counts on: a static text or row marked as a dialog is still content.
    static let dialogContainers: Set<String> = ["AXGroup", "AXWindow", "AXSheet", "AXDialog"]
    /// Roles sent as they are. Anything else goes as `AXUnknown`: a role string is the app's, and could carry text.
    static let knownRoles: Set<String> = chromeRoles.union(hoistable).union(rowHolders).union([
        "AXApplication", "AXWebArea", "AXScrollArea", "AXStaticText", "AXHeading", "AXImage", "AXLink", "AXToolbar",
        "AXTabGroup", "AXMenuBar", "AXMenu", "AXMenuItem", "AXSlider", "AXProgressIndicator", "AXSplitter",
        "AXScrollBar", "AXValueIndicator", "AXLandmarkMain", "AXLandmarkNavigation", "AXForm", "AXList", "AXListItem",
        "AXRadioGroup", "AXIncrementor", "AXColorWell", "AXDateField", "AXLevelIndicator", "AXRuler", "AXPopover"])
    static let knownSubroles: Set<String> = dialogSubroles.union([
        "AXStandardWindow", "AXSecureTextField", "AXSearchField", "AXCloseButton", "AXMinimizeButton", "AXZoomButton",
        "AXFullScreenButton", "AXToggle", "AXSwitch", "AXTabButton", "AXContentList", "AXDefinitionList",
        "AXLandmarkMain", "AXLandmarkNavigation", "AXLandmarkBanner", "AXLandmarkRegion", "AXTextAttachment"])

    /// Folded to letters and digits only: case, accents, width, whitespace, punctuation and invisible characters
    /// (zero-width spaces, curly quotes) all ignored, so "O’Neil", "Aiden\u{200B}Tan" or "9123 4567" match.
    static func compact(_ s: String) -> String {
        String(String.UnicodeScalarView(s.folding(options: [.caseInsensitive, .diacriticInsensitive, .widthInsensitive], locale: nil)
            .unicodeScalars.filter { CharacterSet.alphanumerics.contains($0) }))
    }
    static func words(_ s: String) -> [String] {
        s.folding(options: [.caseInsensitive, .diacriticInsensitive, .widthInsensitive], locale: nil)
            .components(separatedBy: CharacterSet.alphanumerics.inverted).filter { !$0.isEmpty }
    }
    /// What a run's input values look like for masking: each whole value, each word of 3+ characters, and an
    /// email's local part, compacted. Values under 3 characters are kept as whole words.
    static func secrets(_ inputs: RunInputs) -> (long: Set<String>, short: Set<String>) {
        var long = Set<String>(), short = Set<String>()
        for v in inputs.values.values.map({ $0.reveal() }) {
            var parts = [v] + words(v)
            if let at = v.firstIndex(of: "@") { parts.append(String(v[..<at])) }
            for p in parts {
                let c = compact(p)
                // Scripts written without spaces (Chinese, Japanese) have no words to match, so a short value in
                // them is matched anywhere: a two-character name is common.
                if c.count >= 3 || (c.count >= 2 && c.unicodeScalars.contains { !$0.isASCII }) { long.insert(c) }
                else if !c.isEmpty { short.insert(c) }
            }
        }
        return (long, short)
    }
    static func holds(_ text: String, _ s: (long: Set<String>, short: Set<String>)) -> Bool {
        let c = compact(text)
        if s.long.contains(where: { c.contains($0) }) { return true }
        return !s.short.isEmpty && (s.short.contains(c) || words(text).contains { s.short.contains($0) })
    }
    /// Folded for comparing labels with each other (dismissal words, gated targets).
    static func fold(_ s: String) -> String {
        s.folding(options: [.caseInsensitive, .diacriticInsensitive, .widthInsensitive], locale: nil)
            .components(separatedBy: .whitespacesAndNewlines).filter { !$0.isEmpty }.joined(separator: " ")
    }

    public static func build(step: ReplayTask.Step, task: ReplayTask, inputs: RunInputs, root: UINode, divergence: String,
                             attempt: Int, policy: RecoveryPolicy, readValue: (UINode) -> String?) -> Swift.Result<Built, Refusal> {
        guard let classification = policy.knownClassification else {
            return .failure(Refusal(reason: "the task has no classification, and an unknown classification counts as above the model route's ceiling; nothing was sent"))
        }
        let secrets = Self.secrets(inputs)
        func clean(_ s: String?) -> (String?, Bool) {
            guard var s, !s.isEmpty else { return (nil, false) }
            if holds(s, secrets) { return (masked, true) }
            if s.count > maxLabel { s = String(s.prefix(maxLabel)) }
            return (s, false)
        }

        var refs: [String: (node: UINode, path: [Int])] = [:]
        var truncated = false
        func isSecure(_ n: UINode) -> Bool { secureRoles.contains(n.role) || n.subrole == "AXSecureTextField" }

        func emit(_ n: UINode, path: [Int], depth: Int, inDialog: Bool = false) -> [[String: Any]] {
            if refs.count >= maxNodes { truncated = true; return [] }
            func kids() -> [[String: Any]] {
                guard depth < maxDepth else { if !n.children.isEmpty { truncated = true }; return [] }
                var out: [[String: Any]] = []
                let dialog = inDialog || n.role == "AXSheet" || n.role == "AXDialog"
                    || (dialogContainers.contains(n.role) && dialogSubroles.contains(n.subrole ?? ""))
                for (i, c) in n.children.enumerated() { out += emit(c, path: path + [i], depth: depth + 1, inDialog: dialog) }
                return out
            }
            if path.count > 0, hoistable.contains(n.role), n.label == nil, n.domId == nil, n.subrole == nil { return kids() }
            let ref = "n\(refs.count + 1)"
            refs[ref] = (n, path)
            let role = knownRoles.contains(n.role) ? n.role : "AXUnknown"
            var o: [String: Any] = ["ref": ref, "role": role]
            let isDialog = dialogContainers.contains(n.role) && dialogSubroles.contains(n.subrole ?? "")
            let chrome = chromeRoles.contains(n.role) || isDialog || (n.role == "AXHeading" && inDialog)
            if chrome {
                let (label, labelMasked) = clean(n.label)
                if let label { o["label"] = label }
                if labelMasked { o["label_masked"] = true }
                if let d = clean(n.domId).0 { o["dom_id"] = d }
            } else if n.label != nil {
                o["text_omitted"] = true
            }
            if let s = n.subrole, knownSubroles.contains(s) { o["subrole"] = s }
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

        var stepJSON: [String: Any] = ["index": task.steps.firstIndex { $0.id == step.id } ?? -1,
                                       "action": step.action.rawValue,
                                       "target": descriptorJSON(step.target, clean: { clean($0) }),
                                       "consequential": step.consequential.rawValue]
        // The input's name, which the answer must echo: only an identifier, and masked like a label.
        if case .input(let name)? = step.value, name.count <= 40,
           name.unicodeScalars.allSatisfy({ $0.isASCII && (CharacterSet.alphanumerics.contains($0) || $0 == "_") }),
           case (let n?, false) = clean(name) { stepJSON["input"] = n }
        var json: [String: Any] = [
            "kind": "recover",
            "classification": classification,
            "app": task.bundle,
            "attempt": attempt,
            "max_attempts": RecoveryPolicy.maxAttempts,
            "step": stepJSON,
            "divergence": divergenceCode(divergence),
            "values": policy.valuesCleared ? "included" : "masked",
            "tree": tree,
        ]
        if truncated { json["truncated"] = true }
        return .success(Built(json: json, refs: refs))
    }

    /// The step's target under the tree's rules: role always; label, glob and dom id only for a chrome role, and
    /// masked if they hold an input; the path as roles only; anchors (page text near it) never, only a count.
    /// Why the step diverged, as one code from a closed set. The reason itself (which can quote a check's target,
    /// a window title pattern or page text) stays on the device.
    static let divergenceCodes: Set<String> = ["refused", "not_found", "dialog", "no_window", "find_failed", "check_failed",
        "check_failed:exists", "check_failed:appears", "check_failed:disappears", "check_failed:value_equals",
        "check_failed:window_title"]
    static func divergenceCode(_ reason: String) -> String {
        if reason.hasPrefix("recovery's answer was refused") { return "refused" }
        if reason.hasPrefix("element not found") { return "not_found" }
        if reason.hasPrefix("no window") || reason.hasPrefix("the window is gone") { return "no_window" }
        if reason.hasPrefix("check not met") {
            let last = reason.split(separator: ":", maxSplits: 1).dropFirst().first.map { $0.trimmingCharacters(in: .whitespaces) } ?? ""
            if last.hasPrefix("window title") || last.hasPrefix("window_title") { return "check_failed:window_title" }
            for k in ["value_equals", "disappears", "appears", "exists"] where last.hasPrefix(k) { return "check_failed:" + k }
            return "check_failed"
        }
        if reason.contains("dialog") { return "dialog" }
        return "find_failed"
    }

    static func descriptorJSON(_ d: Descriptor, clean: (String?) -> (String?, Bool)) -> [String: Any] {
        var o: [String: Any] = [:]
        let role = d.role.map { knownRoles.contains($0) ? $0 : "AXUnknown" }
        if let role { o["role"] = role }
        if let r = d.role, chromeRoles.contains(r) {
            for (key, text) in [("label", d.label), ("label_matches", d.labelMatches), ("dom_id", d.domId)] {
                let (t, masked) = clean(text)
                if let t { o[key] = t }
                if masked { o[key + "_masked"] = true }
            }
        } else if d.label != nil || d.labelMatches != nil || d.domId != nil {
            o["text_omitted"] = true
        }
        if let p = d.path {
            o["path"] = p.map { seg -> String in
                let r = String(seg.split(separator: ":", maxSplits: 1).first ?? "")
                return knownRoles.contains(r) ? r : "AXUnknown"
            }
        }
        if let a = d.anchors, !a.isEmpty { o["anchors_omitted"] = a.count }
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
    /// Verbs that are often the commit itself ("Cancel" in "Cancel this application?", "Skip", "Hide") are left
    /// out; a dismissal whose word the dialog's own text uses ("Close this case?") is refused by `askedAbout`.
    static let dismissals: Set<String> = ["close", "dismiss", "not now", "maybe later", "got it", "no thanks",
                                          "no, thanks", "x", "×", "✕"]

    /// Could `node` be what `d` (a gated step's target) names? Fails closed: the same role and the same label or
    /// dom id; any candidate the resolver finds, ambiguous ones included; and, when `d` has neither an exact
    /// label nor a dom id (a glob, a path, anchors only), anything of its role (any role, if it names none),
    /// since nothing proves the element is not it. No uniqueness is needed: two such buttons are both refused.
    static func couldBe(_ node: UINode, _ d: Descriptor, in root: UINode) -> Bool {
        if let r = d.role, r != node.role { return false }
        if let l = d.label, let nl = node.label, RecoveryRequest.fold(l) == RecoveryRequest.fold(nl) { return true }
        if let id = d.domId, id == node.domId { return true }
        switch Resolver.resolve(d, in: root) {
        case .found(let m): if m.node === node { return true }
        case .ambiguous(let a): if a.candidates.contains(where: { $0 === node }) { return true }
        case .notFound: break
        }
        return d.label == nil && d.domId == nil
    }

    /// True when the text around a dismissal button (its dialog, sheet, or else its parent) uses the button's own
    /// word: "Close" under "Close this case?" is the commit, not a way out.
    static func askedAbout(_ label: String, path: [Int], root: UINode) -> Bool {
        var chain: [UINode] = [root]
        for i in path.dropLast() { guard let n = chain.last, i < n.children.count else { return true }; chain.append(n.children[i]) }
        // Its dialog or sheet. With none (a web modal with no dialog role), the nearest container above it that
        // holds any text besides buttons: the notice or modal it sits in, not the whole page.
        func hasText(_ n: UINode) -> Bool { n.role != "AXButton" && n.label != nil || n.children.contains(where: hasText) }
        let containers = Array(chain.dropFirst().reversed())
        let around = chain.last { $0.role == "AXSheet" || $0.role == "AXDialog" || RecoveryRequest.dialogSubroles.contains($0.subrole ?? "") }
            ?? containers.first { n in n.children.contains { $0.role != "AXButton" && hasText($0) } }
            ?? root
        let word = RecoveryRequest.words(label)
        guard !word.isEmpty else { return false }
        func texts(_ n: UINode) -> [String] {
            let own = n.role == "AXButton" ? [] : [n.label].compactMap { $0 }
            return own + n.children.flatMap(texts)
        }
        // Words match in their forms, so "Closing", "closed", "closure" and "Dismissal" count as the button's word.
        func forms(_ w: String) -> Set<String> {
            let base = w.count > 3 && w.hasSuffix("e") ? String(w.dropLast()) : w
            var f: Set<String> = [w, w + "s", w + "es", w + "ed", w + "d", base + "ing", base + "ed", base + "ure", base + "ures",
                                  w + "al", w + "als", w + "ing"]
            if w == "x" || w.count < 3 { f = [w] }
            return f
        }
        let want = word.map(forms)
        return texts(around).contains { t in
            let w = RecoveryRequest.words(t)
            guard w.count >= want.count else { return false }
            return (0...(w.count - want.count)).contains { i in want.indices.allSatisfy { want[$0].contains(w[i + $0]) } }
        }
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
            if couldBe(node, s.target, in: root) { return refuse("it targets a consequential element (it could be the target of a step that needs approval)") }
        }
        switch kind {
        case .press:
            // The app's own chrome (tab strip, toolbar, menu bar) can leave the recorded page: never pressed.
            var up: UINode = root
            for i in sent.path.dropLast() {
                if ["AXTabGroup", "AXToolbar", "AXMenuBar", "AXMenu"].contains(up.role) { return refuse("it presses the app's own tabs, toolbar or menus, which may leave the recorded page") }
                guard i < up.children.count else { break }
                up = up.children[i]
            }
            if ["AXTabGroup", "AXToolbar", "AXMenuBar", "AXMenu"].contains(up.role) { return refuse("it presses the app's own tabs, toolbar or menus, which may leave the recorded page") }
            switch node.role {
            case "AXDisclosureTriangle": break
            case "AXButton":
                guard dismissals.contains(RecoveryRequest.fold(label)) else {
                    return refuse("it presses a button that is not a way to dismiss something, which may be consequential")
                }
                if askedAbout(label, path: sent.path, root: root) {
                    return refuse("it presses a button whose word the dialog asks about, so it may be the commit")
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
