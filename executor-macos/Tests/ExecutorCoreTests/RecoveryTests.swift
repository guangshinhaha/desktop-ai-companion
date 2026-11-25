import XCTest
@testable import ExecutorCore

/// A stub model (#34). It answers from a script and keeps every request it was sent. No network, no
/// model, no credentials: the requests go nowhere but this array. Synthetic data only.
final class StubModel: RecoverySource {
    var answers: [[String: Any]?]
    var requests: [[String: Any]] = []
    var onAsk: (() -> Void)?
    init(_ answers: [[String: Any]?]) { self.answers = answers }
    func ask(_ request: [String: Any]) -> [String: Any]? {
        requests.append(request)
        onAsk?()
        return answers.isEmpty ? nil : answers.removeFirst()
    }
}

/// The form with a notice dialog in front of it: "Close" dismisses it, "OK, delete" would commit.
func formBehindNotice() -> UINode {
    let form = provisioningForm()
    let dlg = UINode(role: "AXGroup", children: [UINode(role: "AXStaticText", label: "Scheduled maintenance tonight"),
                                                  UINode(role: "AXButton", label: "Close", domId: "notice-close"),
                                                  UINode(role: "AXButton", label: "OK, delete drafts", domId: "notice-ok"),
                                                  UINode(role: "AXLink", label: "Read more", domId: "notice-link")],
                     subrole: "AXApplicationDialog")
    return UINode(role: form.role, label: form.label, children: form.children + [dlg])
}

/// The ref the stub model would use for the element with this label, read from the request it was sent.
func ref(in request: [String: Any], label: String) -> String? {
    func walk(_ n: [String: Any]) -> String? {
        if n["label"] as? String == label { return n["ref"] as? String }
        for c in (n["children"] as? [[String: Any]]) ?? [] { if let r = walk(c) { return r } }
        return nil
    }
    return (request["tree"] as? [String: Any]).flatMap(walk)
}

func json(_ o: Any) -> String {
    String(decoding: try! JSONSerialization.data(withJSONObject: o, options: [.sortedKeys]), as: UTF8.self)
}

final class RecoveryRequestTests: XCTestCase {
    let policy = RecoveryPolicy(classification: "official-closed")

    /// A window holding a seeded field value, a password field and a label that shows this run's input.
    func seededWindow() -> (UINode, [String: String]) {
        let root = UINode(role: "AXWindow", label: "Sign in", children: [
            UINode(role: "AXWebArea", children: [
                UINode(role: "AXTextField", label: "Full name", domId: "full-name"),
                UINode(role: "AXSecureTextField", label: "Password", domId: "pw"),
                UINode(role: "AXStaticText", label: "Welcome back, Aiden Tan"),
                UINode(role: "AXButton", label: "Close"),
            ]),
        ])
        return (root, ["full-name": "SEEDED-VALUE-7731", "pw": "SEEDED-PASSWORD-9"])
    }

    func build(_ policy: RecoveryPolicy, readCount: UnsafeMutablePointer<[String]>? = nil) throws -> [String: Any] {
        let t = try makeTask()
        let (root, values) = seededWindow()
        let r = RecoveryRequest.build(step: t.steps[0], task: t, inputs: try makeInputs(t), root: root,
                                      divergence: "element not found within 30 s", attempt: 1, policy: policy) { n in
            readCount?.pointee.append(n.role)
            return n.domId.flatMap { values[$0] }
        }
        guard case .success(let b) = r else { XCTFail("must build"); return [:] }
        return b.json
    }

    func testCarriesTheStepsActionAndDescriptorAndAPrunedTreeButNotItsSayLine() throws {
        let j = try build(policy)
        let step = j["step"] as! [String: Any]
        XCTAssertNil(step["say"], "say is task text: not sent")
        XCTAssertNil(step["id"], "the step id is task text: its index goes instead")
        XCTAssertEqual(step["index"] as? Int, 0)
        XCTAssertEqual(step["action"] as? String, "set_value")
        XCTAssertEqual((step["target"] as? [String: Any])?["dom_id"] as? String, "full-name")
        XCTAssertEqual(step["input"] as? String, "full_name", "the input's name, never its value")
        XCTAssertEqual(j["classification"] as? String, "official-closed")
        XCTAssertEqual(j["app"] as? String, "com.google.Chrome")
        XCTAssertNotNil(ref(in: j, label: "Close"))
        XCTAssertNil(j["screenshot"], "no screenshot: vision fallback is #39")
    }

    /// Decision 0003: the one test of the table. A seeded value and a password field go out in neither case
    /// unless the route is cleared for the value; the password never, and never read.
    func testASeededValueIsMaskedAndAPasswordNeverReadOrSent() throws {
        var reads: [String] = []
        let j = try build(policy, readCount: &reads)
        let text = json(j)
        XCTAssertFalse(text.contains("SEEDED-VALUE-7731"), "a field value went out with the route not cleared")
        XCTAssertFalse(text.contains("SEEDED-PASSWORD-9"))
        XCTAssertTrue(reads.isEmpty, "no value is read when the route is not cleared")
        XCTAssertTrue(text.contains("\"value_masked\":true"))
        XCTAssertTrue(text.contains("\"secure\":true"))
    }

    func testWhenTheRouteIsClearedTheValueGoesAndThePasswordStillNever() throws {
        var reads: [String] = []
        let j = try build(RecoveryPolicy(classification: "official-closed", valuesCleared: true), readCount: &reads)
        let text = json(j)
        XCTAssertTrue(text.contains("SEEDED-VALUE-7731"))
        XCTAssertFalse(text.contains("SEEDED-PASSWORD-9"))
        XCTAssertFalse(reads.contains("AXSecureTextField"), "a password field is never read")
    }

    func testThisRunsInputValuesNeverGoEvenInALabelOrWhenCleared() throws {
        for p in [policy, RecoveryPolicy(classification: "official-closed", valuesCleared: true)] {
            let text = json(try build(p))
            XCTAssertFalse(text.contains("Aiden Tan"), "a recorded input value is not part of a recovery request")
        }
    }

    func testUnknownClassificationIsRefusedNotDowngraded() throws {
        let t = try makeTask()
        let (root, _) = seededWindow()
        for c in [nil, "", "  "] as [String?] {
            var read = false
            let r = RecoveryRequest.build(step: t.steps[0], task: t, inputs: try makeInputs(t), root: root, divergence: "x",
                                          attempt: 1, policy: RecoveryPolicy(classification: c, valuesCleared: true)) { _ in read = true; return nil }
            guard case .failure = r else { return XCTFail("an unclassified request must be refused") }
            XCTAssertFalse(read)
        }
    }

    func testTheTreeIsCappedSoAHugePageCannotBeSentWhole() throws {
        let t = try makeTask()
        let many = (0..<5000).map { UINode(role: "AXStaticText", label: "row \($0)") }
        let root = UINode(role: "AXWindow", children: [UINode(role: "AXWebArea", children: many)])
        guard case .success(let b) = RecoveryRequest.build(step: t.steps[0], task: t, inputs: try makeInputs(t), root: root,
                                                           divergence: "x", attempt: 1, policy: policy, readValue: { _ in nil })
        else { return XCTFail() }
        XCTAssertLessThanOrEqual(b.refs.count, RecoveryRequest.maxNodes)
        XCTAssertEqual(b.json["truncated"] as? Bool, true)
    }
}

final class RecoveryReplayTests: XCTestCase {
    struct Run { var report: ReplayReport; var audit: RecordingAudit; var host: FakeHost; var model: StubModel; var approvals: ScriptedApproval }

    /// A host whose form sits behind a notice. Pressing Close takes the notice away; pressing anything in the
    /// notice that commits would be a disaster, so it is recorded and the test fails on it.
    func noticeHost() -> FakeHost {
        let h = FakeHost()
        h.tree = formBehindNotice()
        h.onPress = { n, h in
            if n.domId == "notice-close" { h.tree = provisioningForm() }
            if n.domId == "create" { h.tree = createdPage() }
        }
        return h
    }

    func run(_ h: FakeHost, _ answers: [[String: Any]?], task: ReplayTask? = nil, approvals: [String?] = ["y"],
             policy: RecoveryPolicy = RecoveryPolicy(classification: "official-closed"),
             control: RunControl? = nil, answer: ((StubModel) -> [[String: Any]?])? = nil) throws -> Run {
        let t = try task ?? makeTask()
        let model = StubModel(answers)
        let src = ScriptedApproval(approvals)
        let audit = RecordingAudit()
        audit.host = h
        let r = Replayer(task: t, inputs: try makeInputs(t), host: h, gate: ApprovalGate(source: src, out: { _ in }), ceiling: 2,
                         audit: audit, control: control, recovery: model, recoveryPolicy: policy, log: { _ in })
        return Run(report: r.run(), audit: audit, host: h, model: model, approvals: src)
    }

    /// The stub picks the element by label from the request it was actually sent.
    final class PickingModel: RecoverySource {
        var plan: [(label: String?, extra: [String: Any])]
        var requests: [[String: Any]] = []
        init(_ plan: [(String?, [String: Any])]) { self.plan = plan.map { (label: $0.0, extra: $0.1) } }
        func ask(_ request: [String: Any]) -> [String: Any]? {
            requests.append(request)
            guard !plan.isEmpty else { return nil }
            let p = plan.removeFirst()
            var a = p.extra
            if let l = p.label { a["ref"] = ref(in: request, label: l) ?? "n-none" }
            return a
        }
    }

    func runPicking(_ h: FakeHost, _ plan: [(String?, [String: Any])], task: ReplayTask? = nil,
                    approvals: [String?] = ["y"]) throws -> (ReplayReport, RecordingAudit, PickingModel) {
        let t = try task ?? makeTask()
        let model = PickingModel(plan)
        let audit = RecordingAudit()
        audit.host = h
        let r = Replayer(task: t, inputs: try makeInputs(t), host: h, gate: ApprovalGate(source: ScriptedApproval(approvals), out: { _ in }),
                         ceiling: 2, audit: audit, recovery: model, recoveryPolicy: RecoveryPolicy(classification: "official-closed"), log: { _ in })
        return (r.run(), audit, model)
    }

    // MARK: the request goes out on divergence

    func testADialogBeforeAStepIsDismissedAndTheRunFinishes() throws {
        let (r, audit, model) = try runPicking(noticeHost(), [("Close", ["answer": "act", "action": "press"])])
        XCTAssertEqual(r.outcome, .finished)
        XCTAssertEqual(model.requests.count, 1)
        let req = model.requests[0]
        XCTAssertEqual((req["step"] as? [String: Any])?["index"] as? Int, 0)
        XCTAssertEqual(audit.kinds.filter { $0 == .recovered }.count, 1)
        // The record of the recovery is written before the press, so the trail knows first.
        let i = audit.kinds.firstIndex(of: .recovered)!
        XCTAssertEqual(audit.actionsSeen[i], 0)
        XCTAssertTrue(audit.events[i].shown?.contains("Close") ?? false)
    }

    func testNoRecoverySourceKeepsTheOldBehaviour() throws {
        let h = noticeHost()
        let t = try makeTask()
        let r = Replayer(task: t, inputs: try makeInputs(t), host: h, gate: ApprovalGate(source: ScriptedApproval([]), out: { _ in }),
                         ceiling: 2, log: { _ in }).run()
        guard case .diverged = r.outcome else { return XCTFail("\(r.outcome)") }
        XCTAssertTrue(h.actions.isEmpty)
    }

    // MARK: answers the executor refuses

    func testAnAnswerThatTargetsAConsequentialElementIsRefused() throws {
        let h = noticeHost()
        let (r, audit, _) = try runPicking(h, [("OK, delete drafts", ["answer": "act", "action": "press"])])
        guard case .diverged(_, let why) = r.outcome else { return XCTFail("\(r.outcome)") }
        XCTAssertFalse(h.actions.contains("press:OK, delete drafts"), "a consequential element was pressed")
        XCTAssertTrue(h.actions.isEmpty)
        XCTAssertTrue(why.contains("officer"), why)
        XCTAssertTrue(audit.events.contains { $0.kind == .stepDiverged && ($0.shown ?? "").contains("consequential") })
        XCTAssertFalse(audit.kinds.contains(.recovered))
    }

    func testTheStepsOwnGatedTargetIsConsequentialToo() throws {
        // The dialog is in front of step s3's Create button. Pressing Create through recovery would skip the gate.
        let t = try makeTask()
        let h = FakeHost()
        h.tree = provisioningForm()
        h.fields = ["full-name": "Aiden Tan", "email": "a@x.example.test"]
        h.onPress = { n, h in if n.domId == "create" { h.tree = createdPage() } }
        h.onSet = { _, h in h.tree = formBehindNotice() }
        let (r, _, _) = try runPicking(h, [("Create account", ["answer": "act", "action": "press"])], task: t, approvals: [])
        guard case .diverged = r.outcome else { return XCTFail("\(r.outcome)") }
        XCTAssertFalse(h.actions.contains("press:Create account"))
    }

    func testTheTargetOfAGatedStepIsRefusedEvenWhenItsLabelReadsHarmless() throws {
        // s3 is gated (consequential: yes) and its button reads "Continue", which is not a commit word.
        let t = try makeTask { steps(&$0) { $0[2]["target"] = ["role": "AXButton", "label": "Continue", "dom_id": "create"] } }
        let h = FakeHost()
        let form = provisioningForm(button: "Continue")
        let dlg = UINode(role: "AXGroup", children: [UINode(role: "AXButton", label: "Close", domId: "notice-close")], subrole: "AXApplicationDialog")
        h.tree = UINode(role: form.role, label: form.label, children: form.children + [dlg])
        let (r, audit, _) = try runPicking(h, [("Continue", ["answer": "act", "action": "press"])], task: t, approvals: [])
        guard case .diverged = r.outcome else { return XCTFail("\(r.outcome)") }
        XCTAssertTrue(h.actions.isEmpty, "recovery pressed a gated step's target")
        XCTAssertTrue(audit.events.contains { ($0.shown ?? "").contains("needs approval") })
    }

    func testAnUnlabelledButtonOrALinkIsRefused() throws {
        for (label, extra) in [("Read more", [String: Any]())] {
            let h = noticeHost()
            var e = extra; e["answer"] = "act"; e["action"] = "press"
            let (r, _, _) = try runPicking(h, [(label, e)])
            guard case .diverged = r.outcome else { return XCTFail("\(r.outcome)") }
            XCTAssertTrue(h.actions.isEmpty, "a link may leave the recorded app")
        }
    }

    func testAnAnswerThatTypesAValueOtherThanTheStepsOwnInputIsRefused() throws {
        let h = noticeHost()
        for value in ["Mallory", ["input": "email"], ["constant": "x"]] as [Any] {
            let (r, _, _) = try runPicking(h, [("Full name", ["answer": "act", "action": "set_value", "value": value])])
            guard case .diverged = r.outcome else { return XCTFail("\(value): \(r.outcome)") }
            XCTAssertTrue(h.actions.isEmpty, "\(value)")
        }
    }

    func testAnAnswerThatLeavesTheRecordedAppIsRefused() throws {
        let h = noticeHost()
        for app in ["com.apple.Terminal", "com.apple.mail", ""] {
            let (r, audit, _) = try runPicking(h, [("Close", ["answer": "act", "action": "press", "app": app])])
            guard case .diverged = r.outcome else { return XCTFail("\(app): \(r.outcome)") }
            XCTAssertTrue(h.actions.isEmpty)
            XCTAssertTrue(audit.events.contains { ($0.shown ?? "").contains("recorded app") }, app)
        }
        for extra in [["url": "https://example.test"], ["open": "com.apple.Terminal"], ["bundle": "com.apple.Terminal"]] {
            var a: [String: Any] = ["answer": "act", "action": "press"]
            a.merge(extra) { $1 }
            let (r, _, _) = try runPicking(h, [("Close", a)])
            guard case .diverged = r.outcome else { return XCTFail("\(extra): \(r.outcome)") }
            XCTAssertTrue(h.actions.isEmpty)
        }
    }

    func testMalformedAnswersFailClosed() throws {
        let bad: [[String: Any]?] = [nil, [:], ["answer": "ACT"], ["answer": "act"], ["answer": "act", "action": "press"],
                                     ["answer": "act", "action": "click", "ref": "n1"], ["answer": 1],
                                     ["answer": "act", "action": "press", "ref": 3],
                                     ["answer": "retry", "then": "press Yes"], ["answer": "ask_officer", "reason": 5]]
        for a in bad {
            let h = noticeHost()
            let r = try run(h, [a, a, a, a])
            guard case .diverged(_, let why) = r.report.outcome else { return XCTFail("\(String(describing: a)): \(r.report.outcome)") }
            XCTAssertTrue(h.actions.isEmpty, "\(String(describing: a))")
            XCTAssertEqual(r.model.requests.count, 1, "a malformed or missing answer stops asking: \(String(describing: a))")
            XCTAssertTrue(why.contains("officer"), why)
        }
    }

    // MARK: three attempts, then the officer

    func testAfterThreeAttemptsOnOneStepTheOfficerIsAsked() throws {
        let h = noticeHost()
        let r = try run(h, [["answer": "retry"], ["answer": "retry"], ["answer": "retry"], ["answer": "retry"]])
        guard case .diverged(let step, let why) = r.report.outcome else { return XCTFail("\(r.report.outcome)") }
        XCTAssertEqual(step, "s1")
        XCTAssertEqual(r.model.requests.count, 3)
        XCTAssertEqual(r.model.requests.map { $0["attempt"] as? Int }, [1, 2, 3])
        XCTAssertTrue(why.contains("3 times"), why)
        XCTAssertEqual(r.audit.kinds.last, .runFinished)
        XCTAssertTrue(r.audit.kinds.contains(.stepDiverged))
    }

    func testRefusedAnswersCountAsAttempts() throws {
        let h = noticeHost()
        let (r, _, model) = try runPicking(h, Array(repeating: ("OK, delete drafts", ["answer": "act", "action": "press"]), count: 5))
        guard case .diverged = r.outcome else { return XCTFail() }
        XCTAssertEqual(model.requests.count, 3)
        XCTAssertTrue(h.actions.isEmpty)
    }

    func testAskOfficerStopsAtOnceWithTheReason() throws {
        let r = try run(noticeHost(), [["answer": "ask_officer", "reason": "A notice I cannot read is open."]])
        guard case .diverged(_, let why) = r.report.outcome else { return XCTFail() }
        XCTAssertTrue(why.contains("A notice I cannot read is open."), why)
        XCTAssertEqual(r.model.requests.count, 1)
    }

    func testAnUnclassifiedTaskIsNeverSentForHelp() throws {
        let r = try run(noticeHost(), [["answer": "retry"]], policy: RecoveryPolicy(classification: nil))
        XCTAssertTrue(r.model.requests.isEmpty)
        guard case .diverged(_, let why) = r.report.outcome else { return XCTFail() }
        XCTAssertTrue(why.contains("classification"), why)
    }

    // MARK: recovery never touches the gate

    func testRecoveryNeverApprovesAndTheGatedStepStillAsks() throws {
        // Recovery clears the notice before s1; s3 is still asked about, and a no means nothing is created.
        let h = noticeHost()
        let (r, audit, _) = try runPicking(h, [("Close", ["answer": "act", "action": "press"])], approvals: ["n"])
        XCTAssertEqual(r.outcome, .notApproved(step: "s3"))
        XCTAssertFalse(h.actions.contains("press:Create account"))
        XCTAssertFalse(audit.kinds.contains(.approved))
    }

    func testAGatedStepThatWasActedOnIsNeverRecoveredOrRetried() throws {
        // After Create is pressed, a confirm dialog appears (stand-in confirm=1). The press may have taken effect:
        // the model is not asked, Create is not pressed again, and "Yes, create" is not pressed.
        let h = FakeHost()
        h.tree = provisioningForm()
        h.onPress = { n, h in if n.domId == "create" { h.tree = withDialog(provisioningForm()) } }
        let r = try run(h, [["answer": "retry"]])
        guard case .diverged(let step, _) = r.report.outcome else { return XCTFail("\(r.report.outcome)") }
        XCTAssertEqual(step, "s3")
        XCTAssertTrue(r.model.requests.isEmpty)
        XCTAssertEqual(h.actions.filter { $0 == "press:Create account" }.count, 1)
        XCTAssertFalse(h.actions.contains("press:Yes, create"))
    }

    func testTheConfirmDialogsYesIsRefusedEvenIfAskedAboutBeforeTheStep() throws {
        let h = FakeHost()
        h.tree = withDialog(provisioningForm())
        h.onPress = { n, h in if n.label == "Cancel" { h.tree = provisioningForm() } }
        let (r, _, _) = try runPicking(h, [("Yes, create", ["answer": "act", "action": "press"])], approvals: [])
        guard case .diverged = r.outcome else { return XCTFail("\(r.outcome)") }
        XCTAssertTrue(h.actions.isEmpty)
    }

    // MARK: stop and pause

    func testAStopWhileTheModelIsAskedActsOnNothing() throws {
        let h = noticeHost()
        let control = RunControl()
        let model = StubModel([["answer": "retry"]])
        model.onAsk = { control.stop() }
        let t = try makeTask()
        let audit = RecordingAudit()
        let r = Replayer(task: t, inputs: try makeInputs(t), host: h, gate: ApprovalGate(source: ScriptedApproval([]), out: { _ in }),
                         ceiling: 2, audit: audit, control: control, recovery: model,
                         recoveryPolicy: RecoveryPolicy(classification: "official-closed"), log: { _ in }).run()
        XCTAssertEqual(r.outcome, .stopped(step: "s1", actionDone: false))
        XCTAssertTrue(h.actions.isEmpty)
        XCTAssertFalse(audit.kinds.contains(.recovered))
        XCTAssertTrue(audit.kinds.contains(.runStopped))
    }

    func testAStopAfterAValidAnswerStillPressesNothing() throws {
        let h = noticeHost()
        let control = RunControl()
        let audit = RecordingAudit()
        final class StopThenClose: RecoverySource {
            let control: RunControl
            init(_ c: RunControl) { control = c }
            func ask(_ request: [String: Any]) -> [String: Any]? {
                control.stop()
                return ["answer": "act", "action": "press", "ref": ref(in: request, label: "Close")!]
            }
        }
        let t = try makeTask()
        let r = Replayer(task: t, inputs: try makeInputs(t), host: h, gate: ApprovalGate(source: ScriptedApproval([]), out: { _ in }),
                         ceiling: 2, audit: audit, control: control, recovery: StopThenClose(control),
                         recoveryPolicy: RecoveryPolicy(classification: "official-closed"), log: { _ in }).run()
        XCTAssertEqual(r.outcome, .stopped(step: "s1", actionDone: false))
        XCTAssertTrue(h.actions.isEmpty)
    }

    func testAFailedRecoveryRecordStopsBeforeTheAction() throws {
        let h = noticeHost()
        let t = try makeTask()
        let audit = RecordingAudit()
        audit.failOn = [.recovered]
        let model = PickingModel([("Close", ["answer": "act", "action": "press"])])
        let r = Replayer(task: t, inputs: try makeInputs(t), host: h, gate: ApprovalGate(source: ScriptedApproval([]), out: { _ in }),
                         ceiling: 2, audit: audit, recovery: model, recoveryPolicy: RecoveryPolicy(classification: "official-closed"),
                         log: { _ in }).run()
        guard case .diverged(_, let why) = r.outcome else { return XCTFail() }
        XCTAssertTrue(h.actions.isEmpty)
        XCTAssertTrue(why.contains("audit"), why)
    }

    func testANonGatedStepWhoseCheckFailsBehindADialogIsRecoveredWithoutRedoingTheAction() throws {
        // s1 types, then a notice covers the page and the check cannot pass until it goes.
        let h = FakeHost()
        h.tree = provisioningForm()
        var typed = 0
        h.onSet = { n, h in
            if n.domId == "full-name" { typed += 1; if typed == 1 { h.tree = formBehindNotice() } }
        }
        h.onPress = { n, h in
            if n.domId == "notice-close" { h.tree = provisioningForm() }
            if n.domId == "create" { h.tree = createdPage() }
        }
        let (r, audit, _) = try runPicking(h, [("Close", ["answer": "act", "action": "press"])])
        XCTAssertEqual(r.outcome, .finished)
        XCTAssertEqual(typed, 1, "recovery re-checks; it does not type again")
        XCTAssertEqual(audit.kinds.filter { $0 == .recovered }.count, 1)
    }

    func testOverRPCTheRequestGoesToTheShellAndAnErrorIsANo() throws {
        let t = taskJSON()
        let start = json(["jsonrpc": "2.0", "id": 1, "method": "replay.start",
                          "params": ["task": t, "inputs": ["full_name": "Aiden Tan", "email": "a@x.example.test"],
                                     "ceiling": 1, "classification": "official-closed"]])
        var lines = [start]
        var out: [String] = []
        let h = noticeHost()
        RPC.serve(input: { lines.isEmpty ? nil : lines.removeFirst() }, output: { line in
            out.append(line)
            guard let o = RPC.parse(line), let id = o["id"] as? String, let m = o["method"] as? String else { return }
            switch m {
            case "audit.append": lines.append(json(["jsonrpc": "2.0", "id": id, "result": ["seq": out.count]]))
            case "recover.request": lines.append(json(["jsonrpc": "2.0", "id": id, "error": ["code": -32000, "message": "no service"]]))
            default: lines.append(json(["jsonrpc": "2.0", "id": id, "result": ["approved": false]]))
            }
        }, hostFactory: { _, _ in h })
        let asked = out.compactMap(RPC.parse).filter { $0["method"] as? String == "recover.request" }
        XCTAssertEqual(asked.count, 1)
        let req = (asked[0]["params"] as? [String: Any])?["request"] as? [String: Any]
        XCTAssertEqual(req?["classification"] as? String, "official-closed")
        XCTAssertFalse(json(asked[0]).contains("Aiden Tan"))
        XCTAssertTrue(h.actions.isEmpty)
        let result = out.compactMap(RPC.parse).first { $0["id"] as? Int == 1 }?["result"] as? [String: Any]
        XCTAssertEqual(result?["outcome"] as? String, "diverged")
    }

    func testOverRPCWithoutAClassificationNothingIsAsked() throws {
        let host = noticeHost()
        let start = json(["jsonrpc": "2.0", "id": 1, "method": "replay.start",
                          "params": ["task": taskJSON(), "inputs": ["full_name": "Aiden Tan", "email": "a@x.example.test"], "ceiling": 1]])
        var lines = [start]
        var out: [String] = []
        RPC.serve(input: { lines.isEmpty ? nil : lines.removeFirst() }, output: { line in
            out.append(line)
            guard let o = RPC.parse(line), let id = o["id"] as? String else { return }
            lines.append(json(["jsonrpc": "2.0", "id": id, "result": ["seq": out.count]]))
        }, hostFactory: { _, _ in host })
        XCTAssertFalse(out.contains { $0.contains("recover.request") })
        // Same as before #34: one step_diverged, no recovered, nothing pressed, the run ends diverged.
        let events = out.compactMap { RPC.parse($0) }.filter { $0["method"] as? String == "audit.append" }
            .compactMap { ($0["params"] as? [String: Any])?["event"] as? String }
        XCTAssertEqual(events, ["run_started", "step_diverged", "run_finished"])
        XCTAssertTrue(host.actions.isEmpty)
        let result = out.compactMap { RPC.parse($0) }.first { ($0["id"] as? Int) == 1 }?["result"] as? [String: Any]
        XCTAssertEqual(result?["outcome"] as? String, "diverged")
    }
}
