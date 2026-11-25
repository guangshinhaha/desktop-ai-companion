import XCTest
@testable import ExecutorCore

/// Adversary A2's repros at bad10dc (each was a break; each must now be refused), plus the "Close this case?" variant.
final class RecoveryBreakTests: XCTestCase {
    final class Picking: RecoverySource {
        var plan: [(String?, [String: Any])]; var requests: [[String: Any]] = []
        var onAsk: (() -> Void)?
        init(_ p: [(String?, [String: Any])]) { plan = p }
        func ask(_ request: [String: Any]) -> [String: Any]? {
            requests.append(request); onAsk?()
            guard !plan.isEmpty else { return nil }
            let p = plan.removeFirst(); var a = p.1
            if let l = p.0 { a["ref"] = ref(in: request, label: l) ?? "n-none" }
            return a
        }
    }
    let press: [String: Any] = ["answer": "act", "action": "press"]

    func run(_ h: FakeHost, _ t: ReplayTask, _ m: Picking, control: RunControl? = nil) throws -> (ReplayReport, RecordingAudit) {
        let a = RecordingAudit(); a.host = h
        let r = Replayer(task: t, inputs: try makeInputs(t), host: h, gate: ApprovalGate(source: ScriptedApproval([]), out: { _ in }),
                         ceiling: 2, audit: a, control: control, recovery: m,
                         recoveryPolicy: RecoveryPolicy(classification: "official-closed"), log: { _ in }).run()
        return (r, a)
    }

    /// The form with s1's field gone (s1 diverges at find) and gated s3's button shown twice with this label.
    func twinFormWithoutName(_ label: String) -> FakeHost {
        let h = FakeHost()
        let form = provisioningForm(button: label, twin: true)
        let web = form.children[0]
        h.tree = UINode(role: form.role, label: form.label,
                        children: [UINode(role: web.role, label: web.label, children: web.children.filter { $0.domId != "full-name" })])
        return h
    }

    func gatedTarget(_ d: [String: Any]) throws -> ReplayTask {
        try makeTask { steps(&$0) { $0[2]["target"] = d } }
    }

    /// B1: gated s3 is described by label_matches only (role nil). Its button appears twice.
    func testB1_GatedTargetByLabelMatchesOnlyIsPressed() throws {
        let t = try gatedTarget(["label_matches": "Close"])
        let h = twinFormWithoutName("Close")
        let (r, _) = try run(h, t, Picking([("Close", press)]))
        print("B1 outcome:", r.outcome, "actions:", h.actions)
        XCTAssertFalse(h.actions.contains("press:Close"), "recovery pressed what could be gated s3's target")
    }

    /// B2: same, with role + label_matches (no exact label, no dom id).
    func testB2_GatedTargetByRoleAndLabelMatchesIsPressed() throws {
        let t = try gatedTarget(["role": "AXButton", "label_matches": "Clos*"])
        let h = twinFormWithoutName("Close")
        let (r, _) = try run(h, t, Picking([("Close", press)]))
        print("B2 outcome:", r.outcome, "actions:", h.actions)
        XCTAssertFalse(h.actions.contains("press:Close"), "recovery pressed what could be gated s3's target")
    }

    /// B3: gated s3 described by path only.
    func testB3_GatedTargetByPathOnlyIsPressed() throws {
        let t = try gatedTarget(["path": ["AXWindow", "AXWebArea", "AXButton:Close"]])
        let h = twinFormWithoutName("Close")
        let (r, _) = try run(h, t, Picking([("Close", press)]))
        print("B3 outcome:", r.outcome, "actions:", h.actions)
        XCTAssertFalse(h.actions.contains("press:Close"), "recovery pressed what could be gated s3's target")
    }

    /// B4: a pause pressed while the model is asked. The act must not happen while paused.
    func testB4_PauseDuringAskIsNotHonouredBeforeActing() throws {
        let t = try makeTask()
        let h = FakeHost()
        h.tree = formBehindNotice()
        h.onPress = { n, h in if n.domId == "notice-close" { h.tree = provisioningForm() } }
        h.onSet = { _, h in h.fields["full-name"] = "Aiden Tan"; h.fields["email"] = "a@x.example.test" }
        var log: [String] = []
        let control = RunControl(poll: nil, block: { log.append("held(actions so far: \(h.actions))"); return "{\"jsonrpc\":\"2.0\",\"method\":\"replay.stop\"}" })
        let m = Picking([("Close", press)])
        m.onAsk = { control.pause() }
        let (r, _) = try run(h, t, m, control: control)
        print("B4 outcome:", r.outcome, "actions:", h.actions, "log:", log)
        XCTAssertFalse(h.actions.contains("press:Close"), "recovery acted while the run was paused (and the officer then stopped)")
    }

    /// B5: a dialog asking to cancel a booking; its confirm button is "Cancel".
    func testB5_CancelThatCommitsIsPressed() throws {
        let t = try makeTask()
        let h = FakeHost()
        let form = provisioningForm()
        let dlg = UINode(role: "AXGroup", children: [UINode(role: "AXStaticText", label: "Cancel this officer's leave application?"),
                                                      UINode(role: "AXButton", label: "Cancel", domId: "confirm-cancel"),
                                                      UINode(role: "AXButton", label: "Keep", domId: "keep")],
                         subrole: "AXApplicationDialog")
        h.tree = UINode(role: form.role, label: form.label, children: form.children + [dlg])
        h.onPress = { _, h in h.tree = provisioningForm() }
        let (r, _) = try run(h, t, Picking([("Cancel", press)]))
        print("B5 outcome:", r.outcome, "actions:", h.actions)
        XCTAssertFalse(h.actions.contains("press:Cancel"))
    }

    /// B5 variant: "Close" is on the allow-list, but the dialog asks "Close this case?", so it is the commit.
    func testB5b_CloseUnderCloseThisCaseIsRefused() throws {
        let t = try makeTask()
        let h = FakeHost()
        let form = provisioningForm()
        let dlg = UINode(role: "AXGroup", children: [UINode(role: "AXStaticText", label: "Close this case for good?"),
                                                      UINode(role: "AXButton", label: "Close", domId: "confirm-close"),
                                                      UINode(role: "AXButton", label: "Keep open", domId: "keep")],
                         subrole: "AXApplicationDialog")
        h.tree = UINode(role: form.role, label: form.label, children: form.children + [dlg])
        h.onPress = { _, h in h.tree = provisioningForm() }
        let (_, a) = try run(h, t, Picking([("Close", press)]))
        XCTAssertFalse(h.actions.contains("press:Close"))
        XCTAssertTrue(a.events.contains { $0.kind == .stepDiverged && ($0.shown ?? "").contains("asks about") })
    }
}
