import XCTest
@testable import ExecutorCore

final class RecoveryAttackTests: XCTestCase {
    final class Picking: RecoverySource {
        var plan: [(String?, [String: Any])]; var requests: [[String: Any]] = []
        var onAsk: (() -> Void)?
        init(_ p: [(String?, [String: Any])]) { plan = p }
        func ask(_ request: [String: Any]) -> [String: Any]? {
            requests.append(request)
            onAsk?()
            guard !plan.isEmpty else { return nil }
            let p = plan.removeFirst(); var a = p.1
            if let l = p.0 { a["ref"] = ref(in: request, label: l) ?? "n-none" }
            return a
        }
    }
    func go(_ h: FakeHost, _ t: ReplayTask, _ plan: [(String?, [String: Any])], approvals: [String?] = [], onAsk: (() -> Void)? = nil) throws -> (ReplayReport, RecordingAudit, Picking, ScriptedApproval) {
        let m = Picking(plan); m.onAsk = onAsk; let a = RecordingAudit(); a.host = h; let src = ScriptedApproval(approvals)
        let r = Replayer(task: t, inputs: try makeInputs(t), host: h, gate: ApprovalGate(source: src, out: { _ in }), ceiling: 2,
                         audit: a, recovery: m, recoveryPolicy: RecoveryPolicy(classification: "official-closed"), log: { _ in }).run()
        return (r, a, m, src)
    }
    let press: [String: Any] = ["answer": "act", "action": "press"]

    /// A1: gated s3's button appears twice (top and bottom of the form). Recovery for s1 presses one.
    func testA1_DuplicateGatedTargetIsPressedWithoutApproval() throws {
        let t = try makeTask { steps(&$0) { $0[2]["target"] = ["role": "AXButton", "label": "Continue", "dom_id": "create"] } }
        let h = FakeHost()
        let form = provisioningForm(button: "Continue", twin: true)
        // s1's field is missing (renamed), so s1 diverges at find.
        let web = form.children[0]
        let kids = web.children.filter { $0.domId != "full-name" }
        h.tree = UINode(role: form.role, label: form.label, children: [UINode(role: web.role, label: web.label, children: kids)])
        let (r, _, _, src) = try go(h, t, [("Continue", press)])
        print("A1 outcome:", r.outcome, "actions:", h.actions, "approvals asked:", src.asked)
        XCTAssertFalse(h.actions.contains("press:Continue"), "recovery pressed the target of a gated step with no approval")
    }

    /// A2: gated s3's button was renamed ("Continue" -> "Go", new dom id). s3 diverges at find; recovery presses it.
    func testA2_RenamedGatedTargetIsPressedWithoutApproval() throws {
        let t = try makeTask { steps(&$0) { $0[2]["target"] = ["role": "AXButton", "label": "Continue", "dom_id": "create"] } }
        let h = FakeHost()
        h.fields = ["full-name": "Aiden Tan", "email": "a@x.example.test"]
        let form = provisioningForm(button: "Go")
        let web = form.children[0]
        let kids = web.children.map { $0.domId == "create" ? UINode(role: "AXButton", label: "Go", domId: "go") : $0 }
        h.tree = UINode(role: form.role, label: form.label, children: [UINode(role: web.role, label: web.label, children: kids)])
        h.onPress = { n, h in if n.domId == "go" { h.tree = createdPage() } }
        let (r, _, _, src) = try go(h, t, [("Go", press)])
        print("A2 outcome:", r.outcome, "actions:", h.actions, "approvals asked:", src.asked)
        XCTAssertFalse(h.actions.contains("press:Go"), "recovery did gated s3's action with no approval")
    }

    /// A3: one step gets 3 recovery requests at find, then 3 more after its action.
    func testA3_MoreThanThreeRequestsOnOneStep() throws {
        let t = try makeTask()
        let h = FakeHost()
        h.tree = formBehindNotice()
        h.onPress = { n, h in if n.domId == "notice-close" { h.tree = provisioningForm() } }
        h.onSet = { n, h in if n.domId == "full-name" { h.fields["full-name"] = "" } } // check never passes
        let retry: [String: Any] = ["answer": "retry"]
        let (r, _, m, _) = try go(h, t, [(nil, retry), (nil, retry), ("Close", press), (nil, retry), (nil, retry), (nil, retry)])
        let s1 = m.requests.filter { ($0["step"] as? [String: Any])?["index"] as? Int == 0 }
        print("A3 outcome:", r.outcome, "requests for s1:", s1.count, "attempt numbers:", s1.map { $0["attempt"]! })
        XCTAssertLessThanOrEqual(s1.count, 3, "more than 3 recovery attempts on one step")
    }

    /// A4: an extra dialog's destructive button that is not in the commit word list.
    func testA4_DestructiveDialogButtonIsPressed() throws {
        let t = try makeTask()
        let h = FakeHost()
        let form = provisioningForm()
        let dlg = UINode(role: "AXGroup", children: [UINode(role: "AXStaticText", label: "You have an unsent draft"),
                                                      UINode(role: "AXButton", label: "Discard draft", domId: "discard"),
                                                      UINode(role: "AXButton", label: "Close", domId: "notice-close")],
                         subrole: "AXApplicationDialog")
        h.tree = UINode(role: form.role, label: form.label, children: form.children + [dlg])
        h.onPress = { _, h in h.tree = provisioningForm() }
        let (r, a, _, _) = try go(h, t, [("Discard draft", press)])
        print("A4 outcome:", r.outcome, "actions:", h.actions, "recovered records:", a.kinds.filter { $0 == .recovered }.count)
        XCTAssertFalse(h.actions.contains("press:Discard draft"), "a destructive dialog button was pressed by recovery")
    }

    /// A gated step's button reads like a dismissal ("Close") and appears twice. Only the could-be-gated rule stops it.
    func testAGatedTargetThatReadsLikeADismissalIsStillRefusedEvenWhenDuplicated() throws {
        let t = try makeTask { steps(&$0) { $0[2]["target"] = ["role": "AXButton", "label": "Close", "dom_id": "create"] } }
        let h = FakeHost()
        let form = provisioningForm(button: "Close", twin: true)
        let web = form.children[0]
        let kids = web.children.filter { $0.domId != "full-name" }
        h.tree = UINode(role: form.role, label: form.label, children: [UINode(role: web.role, label: web.label, children: kids)])
        let (r, a, _, _) = try go(h, t, [("Close", press)])
        guard case .diverged = r.outcome else { return XCTFail("\(r.outcome)") }
        XCTAssertTrue(h.actions.isEmpty, "recovery pressed what could be a gated step's target")
        XCTAssertTrue(a.events.contains { ($0.shown ?? "").contains("never presses") })
    }

    /// Two stages of one step share one budget, and the officer message says so truly.
    func testTheBudgetIsSharedAcrossAStepsStages() throws {
        let t = try makeTask()
        let h = FakeHost()
        h.tree = formBehindNotice()
        // The notice goes by itself while the model is asked (no press); then the typing is wiped, so the check
        // stage diverges too.
        h.onSet = { n, h in if n.domId == "full-name" { h.fields["full-name"] = "" } }
        let retry: [String: Any] = ["answer": "retry"]
        let (r, _, m, _) = try go(h, t, [(nil, retry), (nil, retry), (nil, retry), (nil, retry)], onAsk: { h.tree = provisioningForm() })
        XCTAssertEqual(m.requests.map { $0["attempt"] as? Int }, [1, 2, 3])
        XCTAssertTrue(h.actions.contains("set:full-name"), "the second stage (the check) was reached")
        guard case .diverged(_, let why) = r.outcome else { return XCTFail("\(r.outcome)") }
        XCTAssertTrue(why.contains("tried 3 times"), why)
    }
}
