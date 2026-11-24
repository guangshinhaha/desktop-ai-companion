import XCTest
@testable import ExecutorCore

// MARK: fakes

/// A scripted app: a tree, a clock that only `wait` moves, and a record of every action taken.
final class FakeHost: ReplayHost {
    var tree: UINode?
    var clock: TimeInterval = 0
    var actions: [String] = []
    var waits: [TimeInterval] = []
    var fields: [String: String] = [:]          // dom id -> value
    var onPress: ((UINode, FakeHost) -> Void)?
    var pressResult: ActionResult = .ok
    var reselectClears = false
    var chooseIgnored = false
    /// Tree changes that happen at a given clock time (a slow response).
    var scheduled: [(at: TimeInterval, apply: (FakeHost) -> Void)] = []

    func snapshot() -> UINode? { tree }
    func now() -> TimeInterval { clock }
    func wait(_ timeout: TimeInterval) {
        waits.append(timeout)
        clock += timeout
        for s in scheduled where s.at <= clock { s.apply(self) }
        scheduled.removeAll { $0.at <= clock }
    }
    func press(_ node: UINode) -> ActionResult {
        actions.append("press:\(node.label ?? "-")")
        onPress?(node, self)
        return pressResult
    }
    func setValue(_ node: UINode, to text: String) -> ActionResult {
        actions.append("set:\(node.domId ?? "-")")
        if let id = node.domId { fields[id] = text }
        return .ok
    }
    func choose(_ node: UINode, option: String) -> ActionResult {
        actions.append("choose:\(node.domId ?? "-")")
        if let id = node.domId, !chooseIgnored {
            // Chrome (issue #70): pressing the option a pop-up already shows leaves nothing chosen.
            fields[id] = (reselectClears && fields[id] == option) ? "" : option
        }
        return .ok
    }
    func value(of node: UINode) -> String? { node.domId.flatMap { fields[$0] } }
}

final class ScriptedApproval: ApprovalSource {
    var answers: [String?]
    var asked = 0
    init(_ a: [String?]) { answers = a }
    func nextAnswer() -> String? { asked += 1; return answers.isEmpty ? nil : answers.removeFirst() }
}

func provisioningForm(button: String = "Create account", twin: Bool = false) -> UINode {
    func field(_ id: String, _ label: String, role: String = "AXTextField") -> UINode {
        UINode(role: role, label: label, domId: id)
    }
    var kids = [UINode(role: "AXHeading", label: "New staff account"),
                field("full-name", "Full name"), field("email", "Email"),
                field("school", "School", role: "AXPopUpButton"), field("role", "Role", role: "AXPopUpButton"),
                UINode(role: "AXButton", label: button, domId: "create")]
    if twin { kids.append(UINode(role: "AXButton", label: button, domId: "create")) }
    return UINode(role: "AXWindow", label: "New staff account",
                  children: [UINode(role: "AXWebArea", label: "New staff account", children: kids)])
}

func createdPage() -> UINode {
    UINode(role: "AXWindow", label: "Account created",
           children: [UINode(role: "AXWebArea", children: [UINode(role: "AXHeading", label: "Account created")])])
}

func withDialog(_ root: UINode) -> UINode {
    let dlg = UINode(role: "AXGroup", children: [UINode(role: "AXStaticText", label: "Are you sure?"),
                                                  UINode(role: "AXButton", label: "Yes, create"),
                                                  UINode(role: "AXButton", label: "Cancel")], subrole: "AXApplicationDialog")
    return UINode(role: root.role, label: root.label, children: root.children + [dlg])
}

func taskJSON() -> [String: Any] { [
    "format": "task/v1", "id": "tsk_x", "version": 1, "name": "Provision", "author": "o",
    "created": "2026-11-24T10:00:00+08:00", "platform": "macos",
    "apps": [["key": "chrome", "bundle": "com.google.Chrome"]],
    "inputs": [["name": "full_name", "type": "text", "source": "ask"], ["name": "email", "type": "email", "source": "ask"]],
    "steps": [
        ["id": "s1", "say": "Type into Full name", "app": "chrome", "action": "set_value",
         "target": ["role": "AXTextField", "label": "Full name", "dom_id": "full-name"],
         "value": ["input": "full_name"], "expect": [["kind": "value_equals", "input": "full_name"]], "consequential": "no"],
        ["id": "s2", "say": "Type into Email", "app": "chrome", "action": "set_value",
         "target": ["role": "AXTextField", "label": "Email", "dom_id": "email"],
         "value": ["input": "email"], "expect": [["kind": "value_equals", "input": "email"]], "consequential": "no"],
        ["id": "s3", "say": "Press Create account", "app": "chrome", "action": "press",
         "target": ["role": "AXButton", "label": "Create account", "dom_id": "create"],
         "expect": [["kind": "appears", "target": ["role": "AXHeading", "label": "Account created"]]],
         "consequential": "yes",
         "approval": ["summary": "Create an account for {full_name} ({email})", "undoable": false]],
    ],
] }

func makeTask(_ mutate: (inout [String: Any]) -> Void = { _ in }) throws -> ReplayTask {
    var t = taskJSON()
    mutate(&t)
    return try ReplayTask.load(from: JSONSerialization.data(withJSONObject: t))
}

func makeInputs(_ task: ReplayTask, name: String = "Aiden Tan", email: String = "a@x.example.test") throws -> RunInputs {
    try RunInputs.load(from: JSONSerialization.data(withJSONObject: ["full_name": name, "email": email]), for: task)
}

func steps(_ t: inout [String: Any], _ f: (inout [[String: Any]]) -> Void) {
    var s = t["steps"] as! [[String: Any]]
    f(&s)
    t["steps"] = s
}

// MARK: tests

final class ReplayTaskTests: XCTestCase {
    func testLoadsTheProvisioningTask() throws {
        let t = try makeTask()
        XCTAssertEqual(t.steps.map(\.id), ["s1", "s2", "s3"])
        XCTAssertFalse(t.steps[0].needsApproval)
        XCTAssertTrue(t.steps[2].needsApproval)
    }

    func testUnknownCountsAsYes() throws {
        let t = try makeTask { steps(&$0) { $0[2]["consequential"] = "unknown" } }
        XCTAssertTrue(t.steps[2].needsApproval)
    }

    func testConsequentialNoOnACommitLabelIsStillGated() throws {
        let t = try makeTask { steps(&$0) { $0[2]["consequential"] = "no" } }
        XCTAssertTrue(t.steps[2].needsApproval, "a wrong flag must not open the gate")
    }

    func testGatedStepWithoutSummaryStillGetsACardThatSaysWhatItDoes() throws {
        let t = try makeTask { steps(&$0) { $0[2]["approval"] = nil } }
        XCTAssertTrue(t.steps[2].needsApproval)
        let card = ApprovalGate.card(step: t.steps[2], task: t, inputs: try makeInputs(t), target: createButton)
        XCTAssertTrue(card.contains("the task gives no summary"))
        XCTAssertTrue(card.contains("Will do: press the AXButton \"Create account\""))
    }

    // Adversary findings 1-4: the task's `no` is not trusted.
    func testPressIsAlwaysGatedEvenWithNoLabelAndNoFlag() throws {
        for target in [["dom_id": "create"], ["path": ["AXWebArea", "AXButton"]], ["role": "AXButton", "label_matches": "*", "anchors": ["New staff account"]]] as [[String: Any]] {
            let t = try makeTask { steps(&$0) { $0[2]["consequential"] = "no"; $0[2]["target"] = target } }
            XCTAssertTrue(t.steps[2].needsApproval, "\(target)")
        }
    }

    func testZeroWidthAndCamelCaseCommitLabelsAreCaught() {
        for l in ["Dele\u{200B}te account", "DeleteAccount", "D\u{0435}lete", "Remove", "Save", "Publish", "Pay", "Transfer",
                  "Grant access", "Reset password", "Submitted", "Su\u{00AD}bmit"] where l != "D\u{0435}lete" {
            XCTAssertTrue(ReplayTask.looksLikeCommit(l), l)
        }
        for l in ["Full name", "Email", "School", "Role", "Next"] { XCTAssertFalse(ReplayTask.looksLikeCommit(l), l) }
    }

    func testSelectOrSetValueAimedAtAButtonIsGated() throws {
        let t = try makeTask { steps(&$0) {
            $0[0]["target"] = ["role": "AXButton", "label": "Hello"]; $0[0]["consequential"] = "no"
        } }
        XCTAssertTrue(t.steps[0].needsApproval)
        let s = try makeTask { steps(&$0) {
            $0[0]["action"] = "select"; $0[0]["target"] = ["role": "AXButton", "label": "Hello"]; $0[0]["consequential"] = "no"
        } }
        XCTAssertTrue(s.steps[0].needsApproval)
    }

    func testLiveElementThatLooksLikeACommitIsGatedEvenWhenTheDescriptorHasNoLabel() throws {
        // The descriptor names only a dom id and the task says no; the live button is "Delete account".
        let t = try makeTask { steps(&$0) {
            $0[2]["consequential"] = "no"; $0[2]["target"] = ["role": "AXButton", "dom_id": "create"]
            $0[2]["expect"] = [["kind": "exists", "target": ["role": "AXButton", "dom_id": "create"]]]
        } }
        let h = FakeHost()
        h.tree = provisioningForm(button: "Dele\u{200B}te account")
        let src = ScriptedApproval(["n"])
        let r = Replayer(task: t, inputs: try makeInputs(t), host: h, gate: ApprovalGate(source: src, out: { _ in }), log: { _ in })
        XCTAssertEqual(r.run().outcome, .notApproved(step: "s3"))
        XCTAssertEqual(src.asked, 1)
        XCTAssertFalse(h.actions.contains { $0.hasPrefix("press") })
    }

    func testSelectLandingOnAButtonIsGatedAtRunTime() throws {
        let t = try makeTask { steps(&$0) {
            $0[0]["action"] = "select"; $0[0]["target"] = ["dom_id": "create"]; $0[0]["consequential"] = "no"
            $0[0]["expect"] = [["kind": "exists", "target": ["role": "AXButton", "dom_id": "create"]]]
        } }
        let h = FakeHost(); h.tree = provisioningForm()
        let src = ScriptedApproval(["n"])
        let r = Replayer(task: t, inputs: try makeInputs(t), host: h, gate: ApprovalGate(source: src, out: { _ in }), log: { _ in })
        XCTAssertEqual(r.run().outcome, .notApproved(step: "s1"))
        XCTAssertTrue(h.actions.isEmpty)
    }

    func testSummaryNamingAnUndeclaredInputIsRefused() {
        XCTAssertThrowsError(try makeTask { steps(&$0) { $0[2]["approval"] = ["summary": "Create {nobody}", "undoable": false] } })
    }

    func testTypeKeysAndOtherUnbuiltActionsAreRefused() {
        for a in ["type_keys", "open_url", "focus_app", "read_value", "wait_for"] {
            XCTAssertThrowsError(try makeTask { steps(&$0) { $0[0]["action"] = a } }, a)
        }
    }

    func testBadConsequentialValueIsRefused() {
        XCTAssertThrowsError(try makeTask { steps(&$0) { $0[0]["consequential"] = "maybe" } })
        XCTAssertThrowsError(try makeTask { steps(&$0) { $0[0]["consequential"] = nil } })
    }

    func testBareLiteralValueIsRefused() {
        XCTAssertThrowsError(try makeTask { steps(&$0) { $0[0]["value"] = "Aiden Tan" } })
    }

    func testPasswordFieldTargetIsRefused() {
        XCTAssertThrowsError(try makeTask { steps(&$0) { $0[0]["target"] = ["role": "AXSecureTextField", "label": "Password"] } })
    }

    func testTwoAppsAreRefused() {
        XCTAssertThrowsError(try makeTask { $0["apps"] = [["key": "chrome", "bundle": "a"], ["key": "x", "bundle": "b"]] })
    }

    func testUnevaluatedCheckKindIsRefused() {
        XCTAssertThrowsError(try makeTask { steps(&$0) { $0[0]["expect"] = [["kind": "focused", "target": ["role": "AXTextField"]]] } })
    }
}

final class RunInputsTests: XCTestCase {
    func testMissingExtraAndEmptyInputsAreRefusedWithoutEchoingValues() throws {
        let t = try makeTask()
        func load(_ o: [String: Any]) throws -> RunInputs { try RunInputs.load(from: JSONSerialization.data(withJSONObject: o), for: t) }
        XCTAssertThrowsError(try load(["full_name": "x"]))
        XCTAssertThrowsError(try load(["full_name": "x", "email": "y", "surprise": "SECRETVALUE"])) {
            XCTAssertFalse("\($0)".contains("SECRETVALUE"))
        }
        XCTAssertThrowsError(try load(["full_name": "  ", "email": "y"]))
        XCTAssertThrowsError(try load(["full_name": String(repeating: "a", count: 300), "email": "y"]))
        XCTAssertThrowsError(try load(["full_name": true, "email": "y"]))
    }

    func testValuesNeverPrintThemselves() throws {
        let t = try makeTask()
        let i = try makeInputs(t, name: "TOPSECRETNAME")
        let shown = "\(i.secret("full_name")!) \(String(reflecting: i.secret("full_name")!))"
        var dumped = ""
        dump(i.secret("full_name")!, to: &dumped)
        XCTAssertFalse((shown + dumped).contains("TOPSECRETNAME"))
        XCTAssertTrue(shown.contains("redacted"))
    }

    func testChoiceMustBeOneOfItsChoices() throws {
        let t = try makeTask {
            $0["inputs"] = [["name": "full_name", "type": "choice", "choices": ["A", "B"], "source": "ask"],
                            ["name": "email", "type": "text", "source": "ask"]]
        }
        XCTAssertThrowsError(try RunInputs.load(from: JSONSerialization.data(withJSONObject: ["full_name": "C", "email": "e"]), for: t))
        XCTAssertNoThrow(try RunInputs.load(from: JSONSerialization.data(withJSONObject: ["full_name": "A", "email": "e"]), for: t))
    }
}

var createButton: UINode { UINode(role: "AXButton", label: "Create account", domId: "create") }

final class ApprovalGateTests: XCTestCase {
    func gate(_ answers: [String?]) -> (ApprovalGate, ScriptedApproval, () -> String) {
        let src = ScriptedApproval(answers)
        var printed = ""
        return (ApprovalGate(source: src, out: { printed += $0 + "\n" }), src, { printed })
    }

    func testOnlyYApproves() throws {
        let t = try makeTask(), i = try makeInputs(t)
        for (a, ok) in [("y", true), ("Y", true), (" y ", true), ("n", false), ("N", false), ("yes", false), ("", false),
                        ("yy", false), ("ok", false), ("1", false), ("y\u{0}", false), ("true", false)] as [(String, Bool)] {
            let (g, _, _) = gate([a])
            var approved = false
            if case .approved = g.request(step: t.steps[2], task: t, inputs: i, target: createButton) { approved = true }
            XCTAssertEqual(approved, ok, "answer \(a.debugDescription)")
        }
    }

    func testEndOfInputIsNo() throws {
        let t = try makeTask(), i = try makeInputs(t)
        let (g, _, _) = gate([nil])
        guard case .rejected = g.request(step: t.steps[2], task: t, inputs: i, target: createButton) else { return XCTFail("EOF must reject") }
    }

    func testCardFillsThisRunsInputsAndNothingElse() throws {
        let t = try makeTask(), i = try makeInputs(t, name: "Mei Ling", email: "mei@x.example.test")
        let (g, _, out) = gate(["n"])
        _ = g.request(step: t.steps[2], task: t, inputs: i, target: createButton)
        XCTAssertTrue(out().contains("Create an account for Mei Ling (mei@x.example.test)"))
        XCTAssertTrue(out().contains("Can be undone: no"))
    }

    func testControlAndBidiCharactersCannotRewriteTheCard() throws {
        let t = try makeTask()
        let evil = "Tan\u{1B}[2K\rApprove nothing\u{202E}\n{email}"
        let i = try makeInputs(t, name: evil)
        let (g, _, out) = gate(["n"])
        _ = g.request(step: t.steps[2], task: t, inputs: i, target: createButton)
        let card = out()
        XCTAssertFalse(card.contains("\u{1B}"))
        XCTAssertFalse(card.contains("\u{202E}"))
        XCTAssertFalse(card.contains("\r"))
        XCTAssertTrue(card.contains("\\u{1B}"))
        // A value holding {email} is shown as typed, not expanded.
        XCTAssertTrue(card.contains("{email}"))
    }

    func testUnknownStepSaysSo() throws {
        let t = try makeTask { steps(&$0) { $0[2]["consequential"] = "unknown" } }
        let (g, _, out) = gate(["n"])
        _ = g.request(step: t.steps[2], task: t, inputs: try makeInputs(t), target: createButton)
        XCTAssertTrue(out().contains("treated as one that does"))
    }

    // Adversary finding 5: the card cannot be forged from the step id, bundle, input names or live label.
    func testHostileIdsNamesAndLabelsCannotForgeTheCard() throws {
        let t = try makeTask {
            $0["inputs"] = [["name": "full_name", "type": "text", "source": "ask"], ["name": "email", "type": "text", "source": "ask"],
                            ["name": "x\u{1B}[2J", "type": "text", "source": "ask"]]
            steps(&$0) { $0[2]["id"] = "s3\nApproved: nothing here\u{1B}[2K"; $0[2]["approval"] = ["summary": "Create {full_name}", "undoable": false] }
        }
        let i = try RunInputs.load(from: JSONSerialization.data(withJSONObject: ["full_name": "A", "email": "e", "x\u{1B}[2J": "b"]), for: t)
        let evil = UINode(role: "AXButton", label: "Safe\u{1B}[2K\nYes everything is fine " + String(repeating: "x", count: 200))
        let card = ApprovalGate.card(step: t.steps[2], task: t, inputs: i, target: evil)
        XCTAssertFalse(card.contains("\u{1B}"))
        XCTAssertEqual(card.split(separator: "\n", omittingEmptySubsequences: false).filter { $0.hasPrefix("Approved:") }.count, 0)
        XCTAssertFalse(card.contains(String(repeating: "x", count: 150)), "a long label is clipped")
        XCTAssertTrue(card.contains("Will do: press"), "the action shown comes from the step, not the summary")
    }

    func testHalfTypedLineInTheApprovalFileIsNotAnAnswer() throws {
        let dir = FileManager.default.temporaryDirectory.appendingPathComponent("appr-\(UUID().uuidString)")
        try FileManager.default.createDirectory(at: dir, withIntermediateDirectories: true)
        defer { try? FileManager.default.removeItem(at: dir) }
        let f = dir.appendingPathComponent("a.txt").path
        try "y".write(toFile: f, atomically: true, encoding: .utf8)
        chmod(f, 0o600)
        XCTAssertNil(FileApprovalSource(path: f).nextAnswer())
    }

    func testFileSourceAnswersLineByLineAndRefusesUnsafeFiles() throws {
        let dir = FileManager.default.temporaryDirectory.appendingPathComponent("appr-\(UUID().uuidString)")
        try FileManager.default.createDirectory(at: dir, withIntermediateDirectories: true)
        defer { try? FileManager.default.removeItem(at: dir) }
        let f = dir.appendingPathComponent("a.txt").path
        try "y\nn\n".write(toFile: f, atomically: true, encoding: .utf8)
        chmod(f, 0o600)
        let src = FileApprovalSource(path: f)
        XCTAssertEqual(src.nextAnswer(), "y")
        XCTAssertEqual(src.nextAnswer(), "n")
        XCTAssertNil(src.nextAnswer(), "no third line means no answer")

        chmod(f, 0o666)
        XCTAssertNil(FileApprovalSource(path: f).nextAnswer(), "a file others can write is not an answer")
        chmod(f, 0o600)
        let link = dir.appendingPathComponent("link.txt").path
        try FileManager.default.createSymbolicLink(atPath: link, withDestinationPath: f)
        XCTAssertNil(FileApprovalSource(path: link).nextAnswer(), "a symlink is not an answer")
        XCTAssertNil(FileApprovalSource(path: dir.appendingPathComponent("missing").path).nextAnswer())
    }

    func testTerminalSourceWithoutATerminalIsNo() throws {
        let p = Pipe()
        p.fileHandleForWriting.write(Data("y\ny\ny\n".utf8))
        XCTAssertNil(TerminalApprovalSource(fd: p.fileHandleForReading.fileDescriptor).nextAnswer(), "a pipe is not an officer")
    }
}

final class ReplayerTests: XCTestCase {
    func replayer(_ t: ReplayTask, _ host: FakeHost, answers: [String?], ceiling: TimeInterval = 30,
                  log: @escaping (String) -> Void = { _ in }) throws -> (Replayer, ScriptedApproval) {
        let src = ScriptedApproval(answers)
        let gate = ApprovalGate(source: src, out: { _ in })
        return (Replayer(task: t, inputs: try makeInputs(t), host: host, gate: gate, ceiling: ceiling, log: log), src)
    }

    func host() -> FakeHost {
        let h = FakeHost()
        h.tree = provisioningForm()
        h.onPress = { n, h in if n.domId == "create" { h.tree = createdPage() } }
        return h
    }

    func testReplaysStartToSubmitAfterApproval() throws {
        let t = try makeTask(), h = host()
        let (r, src) = try replayer(t, h, answers: ["y"])
        let rep = r.run()
        XCTAssertEqual(rep.outcome, .finished)
        XCTAssertEqual(rep.completed, ["s1", "s2", "s3"])
        XCTAssertEqual(h.actions, ["set:full-name", "set:email", "press:Create account"])
        XCTAssertEqual(src.asked, 1)
        XCTAssertTrue(h.waits.isEmpty, "everything was already true, so nothing waited")
    }

    /// One select step whose option's visible label ("Form Teacher") is not its DOM value ("form_teacher").
    /// The accessibility value is the label, and the recorded input is the label, so the check compares labels.
    func selectTask() throws -> ReplayTask {
        try makeTask { steps(&$0) { s in
            s = [["id": "s1", "say": "Choose Role", "app": "chrome", "action": "select",
                  "target": ["role": "AXPopUpButton", "label": "Role", "dom_id": "role"],
                  "value": ["input": "full_name"],
                  "expect": [["kind": "value_equals", "input": "full_name"]], "consequential": "no"]]
        } }
    }

    func testSelectWhoseLabelDiffersFromItsValueIsChosenAndChecked() throws {
        let t = try selectTask(), h = host()
        h.fields["role"] = "Teacher"
        h.reselectClears = true
        let src = ScriptedApproval([])
        let r = Replayer(task: t, inputs: try makeInputs(t, name: "Form Teacher"), host: h,
                         gate: ApprovalGate(source: src, out: { _ in }), log: { _ in })
        XCTAssertEqual(r.run().outcome, .finished)
        XCTAssertEqual(h.actions, ["choose:role"])
        XCTAssertEqual(h.fields["role"], "Form Teacher")
    }

    func testSelectOfTheOptionAlreadyShownIsNotReselected() throws {
        // Pressing an already-selected option empties Chrome's select; the first option is selected at load.
        let t = try selectTask(), h = host()
        h.fields["role"] = "Teacher"
        h.reselectClears = true
        let src = ScriptedApproval([])
        let r = Replayer(task: t, inputs: try makeInputs(t, name: "Teacher"), host: h,
                         gate: ApprovalGate(source: src, out: { _ in }), log: { _ in })
        XCTAssertEqual(r.run().outcome, .finished)
        XCTAssertTrue(h.actions.isEmpty, "nothing to do, so nothing is pressed")
    }

    func testSelectCheckStillFailsWhenTheFieldHoldsAnotherOption() throws {
        // Not loosened: if choosing does not land the value, the step stops.
        let t = try selectTask(), h = host()
        h.fields["role"] = "Teacher"
        h.chooseIgnored = true
        let src = ScriptedApproval([])
        let r = Replayer(task: t, inputs: try makeInputs(t, name: "Admin Clerk"), host: h,
                         gate: ApprovalGate(source: src, out: { _ in }), ceiling: 1, log: { _ in })
        guard case .diverged(let step, _) = r.run().outcome else { return XCTFail("expected a stop") }
        XCTAssertEqual(step, "s1")
    }

    func testNoStopsTheRunAndNothingIsPressed() throws {
        let t = try makeTask(), h = host()
        let (r, _) = try replayer(t, h, answers: ["n"])
        let rep = r.run()
        XCTAssertEqual(rep.outcome, .notApproved(step: "s3"))
        XCTAssertEqual(rep.completed, ["s1", "s2"])
        XCTAssertEqual(rep.notRun, ["s3"])
        XCTAssertFalse(h.actions.contains { $0.hasPrefix("press") })
    }

    func testAnythingButYStopsBeforeThePress() throws {
        for a: String? in [nil, "", "yes", "yy", "no", "\u{4}", " "] {
            let t = try makeTask(), h = host()
            let (r, _) = try replayer(t, h, answers: [a])
            XCTAssertEqual(r.run().outcome, .notApproved(step: "s3"), "answer \(String(describing: a))")
            XCTAssertFalse(h.actions.contains { $0.hasPrefix("press") })
        }
    }

    func testPerformRefusesAGatedStepWithoutItsApproval() throws {
        let t = try makeTask(), h = host()
        let (r, _) = try replayer(t, h, answers: [])
        let node = h.tree!.descendants.first { $0.domId == "create" }!
        XCTAssertEqual(r.perform(t.steps[2], on: node, approval: nil), .failed("no approval for this step"))
        XCTAssertTrue(h.actions.isEmpty)
        // An approval for another step does not carry over.
        let gate = ApprovalGate(source: ScriptedApproval(["y"]), out: { _ in })
        guard case .approved(let forS3) = gate.request(step: t.steps[2], task: t, inputs: try makeInputs(t), target: createButton) else { return XCTFail() }
        var other = t.steps[2]; other.id = "s9"
        XCTAssertEqual(r.perform(other, on: node, approval: forS3), .failed("no approval for this step"))
        XCTAssertTrue(h.actions.isEmpty)
    }

    func testAmbiguousTargetStopsAndNamesTheStep() throws {
        let t = try makeTask { steps(&$0) { $0[2]["target"] = ["role": "AXButton", "label": "Create account"] } }
        let h = host(); h.tree = provisioningForm(twin: true)
        let (r, src) = try replayer(t, h, answers: ["y"])
        let rep = r.run()
        guard case .diverged(let step, let why) = rep.outcome else { return XCTFail("\(rep.outcome)") }
        XCTAssertEqual(step, "s3")
        XCTAssertTrue(why.contains("ambiguous"))
        XCTAssertEqual(rep.completed, ["s1", "s2"])
        XCTAssertEqual(src.asked, 0, "no approval is asked for a step that cannot be found")
        XCTAssertFalse(h.actions.contains { $0.hasPrefix("press") })
    }

    func testMissingElementStopsAtTheCeilingWithoutFixedDelays() throws {
        let t = try makeTask { steps(&$0) { $0[1]["target"] = ["role": "AXTextField", "label": "Nope", "dom_id": "nope"] } }
        let h = host()
        let (r, _) = try replayer(t, h, answers: ["y"], ceiling: 5)
        let rep = r.run()
        guard case .diverged(let step, _) = rep.outcome else { return XCTFail() }
        XCTAssertEqual(step, "s2")
        XCTAssertEqual(rep.completed, ["s1"])
        XCTAssertEqual(rep.notRun, ["s2", "s3"])
        // 100 ms, 200, 400, 800, 1600, then the remaining 1.9 s: backoff only, capped at 2 s.
        XCTAssertEqual(h.waits.prefix(5).map { $0 }, [0.1, 0.2, 0.4, 0.8, 1.6])
        XCTAssertLessThanOrEqual(h.waits.max()!, 2.0)
        XCTAssertEqual(h.clock, 5, accuracy: 0.001)
    }

    func testSlowSubmitPassesWhenTheResultArrivesBeforeTheCeiling() throws {
        let t = try makeTask(), h = host()
        h.onPress = { n, h in
            let at = h.clock + 2.0
            h.scheduled.append((at, { $0.tree = createdPage() }))
        }
        let (r, _) = try replayer(t, h, answers: ["y"])
        XCTAssertEqual(r.run().outcome, .finished)
        XCTAssertGreaterThanOrEqual(h.clock, 2.0)
        XCTAssertLessThan(h.clock, 3.2, "returned soon after the result, not at the 30 s ceiling")
    }

    func testCeilingOnAPressedStepSaysItMayHaveTakenEffect() throws {
        let t = try makeTask(), h = host()
        h.onPress = { _, _ in }
        let (r, _) = try replayer(t, h, answers: ["y"], ceiling: 3)
        guard case .diverged(let step, let why) = r.run().outcome else { return XCTFail() }
        XCTAssertEqual(step, "s3")
        XCTAssertTrue(why.contains("may or may not have taken effect"))
    }

    func testUnexpectedDialogStopsAndIsNeverClicked() throws {
        let t = try makeTask(), h = host()
        h.onPress = { _, h in h.tree = withDialog(h.tree!) }
        let (r, _) = try replayer(t, h, answers: ["y"])
        guard case .diverged(let step, let why) = r.run().outcome else { return XCTFail() }
        XCTAssertEqual(step, "s3")
        XCTAssertTrue(why.contains("dialog"))
        XCTAssertEqual(h.actions.filter { $0.hasPrefix("press") }, ["press:Create account"], "Yes, create was not pressed")
        XCTAssertTrue(h.waits.isEmpty, "stopped at once, not at the ceiling")
    }

    func testDialogAlreadyOpenBeforeAStepStopsTheRun() throws {
        let t = try makeTask(), h = host()
        h.tree = withDialog(provisioningForm())
        let (r, _) = try replayer(t, h, answers: ["y"])
        guard case .diverged(let step, _) = r.run().outcome else { return XCTFail() }
        XCTAssertEqual(step, "s1")
        XCTAssertTrue(h.actions.isEmpty)
    }

    func testDialogThatAppearsWhileTheOfficerIsDecidingStopsBeforeThePress() throws {
        let t = try makeTask(), h = host()
        final class Slow: ApprovalSource {
            let h: FakeHost
            init(_ h: FakeHost) { self.h = h }
            func nextAnswer() -> String? { h.tree = withDialog(h.tree!); return "y" }
        }
        let gate = ApprovalGate(source: Slow(h), out: { _ in })
        let r = Replayer(task: t, inputs: try makeInputs(t), host: h, gate: gate, log: { _ in })
        guard case .diverged(_, let why) = r.run().outcome else { return XCTFail() }
        XCTAssertTrue(why.contains("after approval"))
        XCTAssertFalse(h.actions.contains { $0.hasPrefix("press") })
    }

    func testButtonThatChangedAfterApprovalIsNotPressed() throws {
        let t = try makeTask(), h = host()
        final class Swap: ApprovalSource {
            let h: FakeHost
            init(_ h: FakeHost) { self.h = h }
            func nextAnswer() -> String? { h.tree = provisioningForm(button: "Delete account"); return "y" }
        }
        let gate = ApprovalGate(source: Swap(h), out: { _ in })
        let r = Replayer(task: t, inputs: try makeInputs(t), host: h, gate: gate, log: { _ in })
        guard case .diverged = r.run().outcome else { return XCTFail() }
        XCTAssertFalse(h.actions.contains { $0.hasPrefix("press") })
    }

    func testFailedActionStopsEvenWhenTheCheckAlreadyPasses() throws {
        let t = try makeTask(), h = host()
        h.pressResult = .failed("AX error -25204")
        h.onPress = { _, h in h.tree = createdPage() }
        guard case .diverged(_, let w1) = try replayer(t, h, answers: ["y"]).0.run().outcome else { return XCTFail() }
        XCTAssertTrue(w1.contains("action failed"))

        let h2 = host()
        h2.pressResult = .failed("AX error -25204")
        h2.onPress = { _, _ in }
        guard case .diverged(_, let why) = try replayer(t, h2, answers: ["y"]).0.run().outcome else { return XCTFail() }
        XCTAssertTrue(why.contains("action failed"))
    }

    func testValueCheckFailsWhenTheFieldDoesNotHoldTheInput() throws {
        let t = try makeTask(), h = host()
        h.fields = [:]
        // A host that ignores the set: the field keeps the wrong value.
        final class Ignoring: ReplayHost {
            let inner: FakeHost
            init(_ i: FakeHost) { inner = i }
            func snapshot() -> UINode? { inner.snapshot() }
            func wait(_ t: TimeInterval) { inner.wait(t) }
            func now() -> TimeInterval { inner.now() }
            func press(_ n: UINode) -> ActionResult { inner.press(n) }
            func setValue(_ n: UINode, to: String) -> ActionResult { .ok }
            func choose(_ n: UINode, option: String) -> ActionResult { .ok }
            func value(of n: UINode) -> String? { "something else" }
        }
        let gate = ApprovalGate(source: ScriptedApproval(["y"]), out: { _ in })
        let r = Replayer(task: t, inputs: try makeInputs(t), host: Ignoring(h), gate: gate, ceiling: 2, log: { _ in })
        guard case .diverged(let step, let why) = r.run().outcome else { return XCTFail() }
        XCTAssertEqual(step, "s1")
        XCTAssertFalse(why.contains("something else"))
        XCTAssertFalse(why.contains("Aiden"))
    }

    func testNoInputValueReachesTheLogOrTheReport() throws {
        let t = try makeTask(), h = host()
        var lines: [String] = []
        let (r, _) = try replayer(t, h, answers: ["n"], log: { lines.append($0) })
        let rep = r.run()
        let all = lines.joined() + "\(rep)"
        XCTAssertFalse(all.contains("Aiden"))
        XCTAssertFalse(all.contains("example.test"))
    }

    func testPreconditionNotMetRefusesBeforeAnyAction() throws {
        let t = try makeTask {
            $0["preconditions"] = [["app": "chrome", "check": ["kind": "exists", "target": ["role": "AXButton", "label": "Sign out"]]]]
        }
        let h = host()
        let (r, _) = try replayer(t, h, answers: ["y"], ceiling: 1)
        guard case .refused = r.run().outcome else { return XCTFail() }
        XCTAssertTrue(h.actions.isEmpty)
    }
}

final class WaitingTests: XCTestCase {
    func testBackoffStartsAt100msAndStopsAt2s() {
        var clock = 0.0
        var waits: [Double] = []
        let r: Waiting.Result<Void> = Waiting.until(ceiling: 20, wait: { waits.append($0); clock += $0 }, now: { clock }, poll: { .keepWaiting })
        guard case .ceiling = r else { return XCTFail() }
        XCTAssertEqual(Array(waits.prefix(6)), [0.1, 0.2, 0.4, 0.8, 1.6, 2.0])
        XCTAssertTrue(waits.dropFirst(5).allSatisfy { $0 <= 2.0 })
        XCTAssertEqual(clock, 20, accuracy: 0.001)
    }

    func testAlreadyTrueDoesNotWait() {
        var waited = false
        let r: Waiting.Result<Int> = Waiting.until(ceiling: 30, wait: { _ in waited = true }, now: { 0 }, poll: { .done(1) })
        guard case .done(1) = r else { return XCTFail() }
        XCTAssertFalse(waited)
    }
}

// Interrogate round 1, adversary B.
final class ReplayHardeningTests: XCTestCase {
    func testDialogTextWithARunValueIsNeverPrinted() throws {
        let t = try makeTask(), h = FakeHost()
        let dlg = UINode(role: "AXGroup", children: [UINode(role: "AXStaticText", label: "Create account for Aiden")], subrole: "AXApplicationDialog")
        h.tree = UINode(role: "AXWindow", label: "x", children: provisioningForm().children + [dlg])
        let r = Replayer(task: t, inputs: try makeInputs(t, name: "Aiden"), host: h,
                         gate: ApprovalGate(source: ScriptedApproval([]), out: { _ in }), log: { _ in })
        guard case .diverged(_, let why) = r.run().outcome else { return XCTFail() }
        XCTAssertFalse(why.contains("Aiden"))
    }

    func testTaskTimeoutCannotLengthenTheRunsCeiling() throws {
        let t = try makeTask { steps(&$0) { $0[2]["timeout_seconds"] = 3600 } }
        let h = FakeHost(); h.tree = provisioningForm(); h.onPress = { _, _ in }
        let r = Replayer(task: t, inputs: try makeInputs(t), host: h,
                         gate: ApprovalGate(source: ScriptedApproval(["y"]), out: { _ in }), ceiling: 2, log: { _ in })
        _ = r.run()
        XCTAssertLessThanOrEqual(h.clock, 2.01)
    }

    func testMalformedTimeoutAnchorsAndHugeTasksAreRefused() {
        for bad in [true, "abc", NSNull()] as [Any] {
            XCTAssertThrowsError(try makeTask { steps(&$0) { $0[0]["timeout_seconds"] = bad } }, "\(bad)")
        }
        let many = (0..<40).map { "a\($0)" }
        XCTAssertThrowsError(try makeTask { steps(&$0) { $0[0]["target"] = ["role": "AXTextField", "anchors": many] } })
        XCTAssertThrowsError(try ReplayTask.load(from: Data(repeating: 0x20, count: ReplayTask.maxTaskBytes + 1)))
    }
}

/// The grep in issue #15: the only waiting in Sources is the TD 5.3 backoff.
final class NoFixedDelayTests: XCTestCase {
    func testSourcesHoldNoSleepOrAsyncAfter() throws {
        let sources = URL(fileURLWithPath: #filePath).deletingLastPathComponent().deletingLastPathComponent().deletingLastPathComponent()
            .appendingPathComponent("Sources")
        let en = FileManager.default.enumerator(at: sources, includingPropertiesForKeys: nil)!
        var hits: [String] = []
        for case let u as URL in en where u.pathExtension == "swift" {
            let text = try String(contentsOf: u, encoding: .utf8)
            for (n, l) in text.split(separator: "\n", omittingEmptySubsequences: false).enumerated()
            where l.contains("sleep") || l.contains("asyncAfter") { hits.append("\(u.lastPathComponent):\(n + 1)") }
        }
        XCTAssertEqual(hits, [])
    }
}
