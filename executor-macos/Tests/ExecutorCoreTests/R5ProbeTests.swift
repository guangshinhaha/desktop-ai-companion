import XCTest
@testable import ExecutorCore

final class R5ProbeTests: XCTestCase {
    typealias Picking = R4ProbeTests.Picking
    let press: [String: Any] = ["answer": "act", "action": "press"]
    func run(_ h: FakeHost, _ t: ReplayTask, _ m: Picking) throws -> ReplayReport {
        let a = RecordingAudit(); a.host = h; defer { print("AUD:", a.events.compactMap { $0.shown }.filter { $0.contains("refused") }, m.requests.map { json($0) }.first?.prefix(900) ?? "") }
        return Replayer(task: t, inputs: try makeInputs(t), host: h, gate: ApprovalGate(source: ScriptedApproval([]), out: { _ in }),
                        ceiling: 2, audit: a, recovery: m,
                        recoveryPolicy: RecoveryPolicy(classification: "official-closed"), log: { _ in }).run()
    }
    func page(_ extra: [UINode]) -> UINode {
        let form = provisioningForm()
        return UINode(role: form.role, label: form.label, children: [UINode(role: "AXWebArea", children: extra)])
    }
    func pressed(_ tree: UINode, id: String) throws -> Bool {
        let h = FakeHost(); h.tree = tree
        var hit = false
        h.onPress = { n, h in if n.domId == id { hit = true; h.tree = provisioningForm() } }
        _ = try run(h, try makeTask(), Picking([("Close", press), ("Close", press), ("Close", press)]))
        return hit
    }
    /// B1: a case table. Row 2's "Close" closes an open case. Row 1 says "Closed"; the old window-wide check refused.
    func testB1_RowCloseOnCaseList() throws {
        let t = page([
            UINode(role: "AXHeading", label: "My cases"),
            UINode(role: "AXGroup", children: [UINode(role: "AXStaticText", label: "Case 1041"), UINode(role: "AXStaticText", label: "Closed")]),
            UINode(role: "AXGroup", children: [UINode(role: "AXStaticText", label: "Case 1042"), UINode(role: "AXStaticText", label: "Open"),
                                             UINode(role: "AXButton", label: "Close", domId: "close-1042")])])
        XCTAssertFalse(try pressed(t, id: "close-1042"), "B1: row Close (closes a case) pressed")
    }
    /// B2: the question is in the same container, worded with a form not listed.
    func testB2_UnlistedForm() throws {
        for q in ["Closeout case 1042 and notify the applicant?", "Reclose case 1042?", "CloseCase 1042?"] {
            let t = page([UINode(role: "AXGroup", children: [UINode(role: "AXStaticText", label: q),
                                                            UINode(role: "AXButton", label: "Close", domId: "c")])])
            XCTAssertFalse(try pressed(t, id: "c"), "B2 pressed under: \(q)")
        }
    }
    /// B3: no dialog role, question in a header group, button in a footer with any text (declared residual).
    func testB3_FooterHint() throws {
        let t = page([UINode(role: "AXGroup", children: [
            UINode(role: "AXGroup", children: [UINode(role: "AXHeading", label: "Close case 1042?")]),
            UINode(role: "AXGroup", children: [UINode(role: "AXStaticText", label: "This cannot be undone."),
                                               UINode(role: "AXButton", label: "Close", domId: "c")])])])
        XCTAssertFalse(try pressed(t, id: "c"), "B3: footer Close pressed")
    }
}
