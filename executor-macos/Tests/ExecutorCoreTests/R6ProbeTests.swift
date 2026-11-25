import XCTest
@testable import ExecutorCore

/// Round 6 (adversary at 0d3243b): four ways a Close got pressed through the dialog-only rule. Decision on #34
/// (consult-shin): recovery never presses anything. Every probe here expects refusal.
final class R6ProbeTests: XCTestCase {
    let p = R5ProbeTests()
    func dlg(_ kids: [UINode]) -> UINode { UINode(role: "AXGroup", children: kids, subrole: "AXApplicationDialog") }
    /// N1: nested dialog. Outer sheet asks; inner role=dialog group holds only the footer.
    func testN1_NestedDialogHidesQuestion() throws {
        let t = p.page([UINode(role: "AXSheet", children: [
            UINode(role: "AXHeading", label: "Close case 1042? This cannot be undone."),
            dlg([UINode(role: "AXButton", label: "Close", domId: "c")])])])
        XCTAssertFalse(try p.pressed(t, id: "c"), "N1 pressed")
    }
    /// N2: whole page rendered as a full-screen role=dialog (MUI fullScreen / record modal); row Close.
    func testN2_FullScreenDialogRowClose() throws {
        let t = p.page([dlg([
            UINode(role: "AXHeading", label: "My cases"),
            UINode(role: "AXGroup", children: [UINode(role: "AXStaticText", label: "Case 1042"), UINode(role: "AXStaticText", label: "Open"),
                                             UINode(role: "AXButton", label: "Close", domId: "close-1042")])])])
        XCTAssertFalse(try p.pressed(t, id: "close-1042"), "N2 pressed")
    }
    /// N3: root window itself has AXDialog subrole (panel-style window); row Close.
    func testN3_WindowIsDialog() throws {
        let form = provisioningForm()
        let t = UINode(role: "AXWindow", label: form.label, children: [UINode(role: "AXWebArea", children: [
            UINode(role: "AXGroup", children: [UINode(role: "AXStaticText", label: "Case 1042"), UINode(role: "AXStaticText", label: "Open"),
                                             UINode(role: "AXButton", label: "Close", domId: "close-1042")])])], subrole: "AXDialog")
        XCTAssertFalse(try p.pressed(t, id: "close-1042"), "N3 pressed")
    }
    /// N4: case drawer (role=dialog), Close is the case action next to Escalate; no question text.
    func testN4_CaseDrawer() throws {
        let t = p.page([dlg([UINode(role: "AXHeading", label: "Case 1042"), UINode(role: "AXStaticText", label: "Status: Open"),
                             UINode(role: "AXButton", label: "Escalate"), UINode(role: "AXButton", label: "Close", domId: "c")])])
        XCTAssertFalse(try p.pressed(t, id: "c"), "N4 pressed")
    }

    /// A disclosure triangle (the one non-button press the old allow-list kept) is refused too: expanding a row
    /// can run app code, and nothing in the tree proves it can't.
    func testDisclosureTriangleIsRefused() throws {
        let h = FakeHost()
        h.tree = p.page([UINode(role: "AXDisclosureTriangle", label: "Show details", domId: "more")])
        h.onPress = { _, h in h.tree = provisioningForm() }
        let press: [String: Any] = ["answer": "act", "action": "press"]
        let (r, audit, model) = try RecoveryReplayTests().runPicking(h, [("Show details", press), ("Show details", press)])
        guard case .diverged(_, let why) = r.outcome else { return XCTFail("\(r.outcome)") }
        XCTAssertFalse(h.actions.contains { $0.hasPrefix("press:") }, "recovery never presses")
        XCTAssertEqual(model.requests.count, 1, "a press answer goes to the officer at once")
        XCTAssertTrue(why.contains("officer"), why)
        XCTAssertTrue(audit.events.contains { $0.kind == .stepDiverged && ($0.shown ?? "").contains("never presses") })
    }

    /// No shape of press answer parses: there is no press path to success.
    func testNoPressAnswerParses() throws {
        let t = try makeTask()
        for a: [String: Any] in [["answer": "act", "action": "press", "ref": "n1"],
                                 ["answer": "act", "action": "press", "ref": "n1", "app": "com.google.Chrome"],
                                 ["answer": "act", "action": "press", "ref": "n1", "value": ["input": "full_name"]]] {
            guard case .failure(let rej) = RecoveryAnswer.parse(a, step: t.steps[0], task: t) else { return XCTFail("\(a) parsed") }
            XCTAssertTrue(rej.malformed, "a press is final: the officer is asked at once")
        }
    }
}
