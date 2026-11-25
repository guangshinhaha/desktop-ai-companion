import XCTest
@testable import ExecutorCore

/// Round 3 adversarial probes on `RecoveryRequest.build` (#34). Synthetic data only.
final class RecoveryRound3LeakTests: XCTestCase {
    let policy = RecoveryPolicy(classification: "official-closed")
    func req(_ root: UINode, name: String = "Aiden Tan", email: String = "a@x.example.test",
             mutate: (inout [String: Any]) -> Void = { _ in }) throws -> String {
        let t = try makeTask(mutate)
        let r = RecoveryRequest.build(step: t.steps[0], task: t, inputs: try makeInputs(t, name: name, email: email), root: root,
                                      divergence: "element not found within 30 s", attempt: 1, policy: policy) { _ in "FIELDVAL" }
        guard case .success(let b) = r else { XCTFail("build"); return "" }
        return json(b.json)
    }
    func win(_ kids: [UINode], title: String = "W") -> UINode {
        UINode(role: "AXWindow", label: title, children: [UINode(role: "AXWebArea", children: kids)])
    }

    /// A two-character CJK name is a "short" secret, matched only as a whole word; CJK has no spaces between words.
    func testR1_CJKNameInWindowTitle() throws {
        let s = try req(win([], title: "陈伟的档案 - Records"), name: "陈伟"); print("R1:", s)
        XCTAssertFalse(s.contains("陈伟"), "CJK run input sent in window title")
    }
    func testR1b_CJKNameInButton() throws {
        let s = try req(win([UINode(role: "AXButton", label: "编辑陈伟")]), name: "陈伟"); print("R1b:", s)
        XCTAssertFalse(s.contains("陈伟"), "CJK run input sent in button label")
    }

    /// The step's descriptor goes out as recorded, without the run-input mask or the chrome-only rule.
    func testR2_DescriptorLabelHoldsRunInput() throws {
        let s = try req(win([])) { t in steps(&t) { $0[0]["target"] = ["role": "AXLink", "label": "Aiden Tan", "path": ["AXWebArea", "AXLink:Aiden Tan"]] } }
        print("R2:", s)
        XCTAssertFalse(s.contains("Aiden Tan"), "run input sent in step target descriptor")
    }
    func testR2b_DescriptorAnchorsCarryPageText() throws {
        let s = try req(win([])) { t in steps(&t) { $0[0]["target"] = ["role": "AXTextField", "label": "Full name", "anchors": ["SYNTH-ANCHOR Lim Bee Hoon S9999999Z"]] } }
        print("R2b:", s)
        XCTAssertFalse(s.contains("SYNTH-ANCHOR"), "non-chrome page text sent via descriptor anchors")
    }

    /// `say` is sent as is.
    func testR3_SayHoldsRunInput() throws {
        let s = try req(win([])) { t in steps(&t) { $0[0]["say"] = "Type Aiden Tan into Full name" } }
        print("R3:", s)
        XCTAssertFalse(s.contains("Aiden"), "run input sent in say")
    }

    /// A dialog subrole makes any role chrome, including static text, cells and rows.
    func testR4_StaticTextWithDialogSubrole() throws {
        let s = try req(win([UINode(role: "AXStaticText", label: "SYNTH-TEXT Lim Bee Hoon", subrole: "AXApplicationDialog")])); print("R4:", s)
        XCTAssertFalse(s.contains("SYNTH-TEXT"), "static text label sent because of a dialog subrole")
    }
    func testR4b_RowWithDialogSubrole() throws {
        let s = try req(win([UINode(role: "AXRow", label: "SYNTH-ROW Lim Bee Hoon", subrole: "AXDialog")])); print("R4b:", s)
        XCTAssertFalse(s.contains("SYNTH-ROW"), "row label sent because of a dialog subrole")
    }

    /// German sharp s: case folding of "ß".
    func testR6_SharpS() throws {
        let s = try req(win([], title: "Record: STRAUSS"), name: "Strauß"); print("R6:", s)
        XCTAssertFalse(s.contains("STRAUSS"), "run input with ß vs ss sent")
    }
}
