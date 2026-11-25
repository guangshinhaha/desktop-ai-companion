import XCTest
@testable import ExecutorCore

final class R4ProbeTests: XCTestCase {
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

    /// A check target's static-text label (page content, here holding the run's input) goes out in `divergence`.
    func testP1_CheckTargetLabelLeaksViaDivergence() throws {
        let t = try makeTask { t in steps(&t) { s in
            s[0]["expect"] = [["kind": "appears", "target": ["role": "AXStaticText", "label": "Saved for Aiden Tan, NRIC S9999999Z"]]]
        } }
        let h = FakeHost(); h.tree = provisioningForm()
        let m = Picking([])
        _ = try run(h, t, m)
        let s = m.requests.map { json($0) }.joined()
        print("P1:", s.prefix(400))
        XCTAssertFalse(s.contains("Aiden Tan"), "run input sent in divergence")
        XCTAssertFalse(s.contains("S9999999Z"), "static text sent in divergence")
    }

    /// window_title pattern goes out verbatim.
    func testP2_WindowTitlePatternLeaksViaDivergence() throws {
        let t = try makeTask { t in steps(&t) { s in
            s[0]["expect"] = [["kind": "window_title", "matches": "Case of Aiden Tan*"]]
        } }
        let h = FakeHost(); h.tree = provisioningForm()
        let m = Picking([])
        _ = try run(h, t, m)
        let s = m.requests.map { json($0) }.joined()
        print("P2:", s.prefix(300))
        XCTAssertFalse(s.contains("Aiden Tan"), "run input sent in divergence (window title pattern)")
    }

    /// A pause that lands between the pre-request hold and the stop check: the request still goes out while paused.
    func testP3_PauseBeforeRequestIsSwallowed() throws {
        var hits: [Int] = []
        for k in 1...8 {
            let t = try makeTask()
            let h = FakeHost(); h.tree = formBehindNotice()
            var polls = 0, armed = false
            let control = RunControl(poll: {
                guard armed else { return nil }
                polls += 1
                return polls == k ? "{\"jsonrpc\":\"2.0\",\"method\":\"replay.pause\"}" : nil
            }, block: { "{\"jsonrpc\":\"2.0\",\"method\":\"replay.stop\"}" })
            var stateAtAsk: [RunControl.State] = []
            let m = Picking([(nil, ["answer": "retry"]), (nil, ["answer": "ask_officer", "reason": "x"])])
            m.onAsk = { stateAtAsk.append(control.state); armed = true }
            _ = try run(h, t, m, control: control)
            print("P3 k=\(k) state at ask:", stateAtAsk)
            if stateAtAsk.contains(.paused) { hits.append(k) }
        }
        XCTAssertTrue(hits.isEmpty, "request sent while paused for k=\(hits)")
    }

    /// Over-correction: a notice with no dialog role, its "Close" button, and the page elsewhere says "Closing date".
    func testP4_NoticeWithoutDialogRoleOnPageWithClosingDate() throws {
        let t = try makeTask()
        let form = provisioningForm()
        let notice = UINode(role: "AXGroup", label: "Notice", children: [
            UINode(role: "AXStaticText", label: "Scheduled maintenance tonight"),
            UINode(role: "AXButton", label: "Close", domId: "notice-close")])
        let page = UINode(role: form.role, label: form.label, children: [UINode(role: "AXWebArea", children:
            form.children[0].children + [UINode(role: "AXStaticText", label: "Closing date for applications: 1 Dec"), notice])])
        let h = FakeHost(); h.tree = page
        // The notice covers the form: the target is missing until it is closed.
        let hidden = UINode(role: page.role, label: page.label, children: [UINode(role: "AXWebArea", children:
            [UINode(role: "AXStaticText", label: "Closing date for applications: 1 Dec"), notice])])
        h.tree = hidden
        h.onPress = { n, h in if n.domId == "notice-close" { h.tree = provisioningForm() } }
        let m = Picking([("Close", press), ("Close", press), ("Close", press)])
        let (r, a) = try run(h, t, m)
        print("P4:", r.outcome, h.actions, a.events.compactMap { $0.shown }.filter { $0.contains("refused") }.first ?? "")
        // Decision on #34 (consult-shin): a dismissal is pressed only inside a real dialog or sheet. This notice has
        // no dialog role, so it is refused and the officer is asked; the page's "Closing date" plays no part.
        XCTAssertFalse(h.actions.contains("press:Close"), "a Close outside a dialog is refused")
        XCTAssertTrue(a.events.contains { ($0.shown ?? "").contains("outside a dialog") })
    }

    /// The same notice with a dialog role: page text outside the dialog ("Closing date") no longer blocks its Close.
    func testP4b_DialogNoticeOnPageWithClosingDateIsDismissed() throws {
        let t = try makeTask()
        let notice = UINode(role: "AXGroup", label: "Notice", children: [
            UINode(role: "AXStaticText", label: "Scheduled maintenance tonight"),
            UINode(role: "AXButton", label: "Close", domId: "notice-close")], subrole: "AXApplicationDialog")
        let form = provisioningForm()
        let h = FakeHost()
        h.tree = UINode(role: form.role, label: form.label, children: [UINode(role: "AXWebArea", children:
            [UINode(role: "AXStaticText", label: "Closing date for applications: 1 Dec"), notice])])
        h.onPress = { n, h in if n.domId == "notice-close" { h.tree = provisioningForm() } }
        h.onSet = { _, h in h.fields["full-name"] = "Aiden Tan"; h.fields["email"] = "a@x.example.test" }
        let (_, _) = try run(h, t, Picking([("Close", press)]))
        XCTAssertTrue(h.actions.contains("press:Close"), "page text outside the dialog must not block its Close")
    }
}
