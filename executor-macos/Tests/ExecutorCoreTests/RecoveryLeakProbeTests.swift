import XCTest
@testable import ExecutorCore

final class RecoveryLeakProbeTests: XCTestCase {
    let policy = RecoveryPolicy(classification: "official-closed")
    func req(_ root: UINode, name: String = "Aiden Tan", email: String = "a@x.example.test") throws -> String {
        let t = try makeTask()
        let r = RecoveryRequest.build(step: t.steps[0], task: t, inputs: try makeInputs(t, name: name, email: email), root: root,
                                      divergence: "element not found within 30 s", attempt: 1, policy: policy) { _ in "FIELDVAL" }
        guard case .success(let b) = r else { XCTFail("build"); return "" }
        return json(b.json)
    }
    func win(_ kids: [UINode]) -> UINode { UINode(role: "AXWindow", label: "W", children: [UINode(role: "AXWebArea", children: kids)]) }

    func testP1_RowOutsideTable() throws {
        let grid = UINode(role: "AXGroup", children: [
            UINode(role: "AXRow", label: "SYNTH-ROW Lim Bee Hoon S9999999Z", children: [UINode(role: "AXCell", label: "SYNTH-CELL Lim")]),
        ])
        let s = try req(win([grid])); print("P1:", s)
        XCTAssertFalse(s.contains("SYNTH-ROW"), "row label sent")
    }
    func testP2_StaticTextListInScrollArea() throws {
        let list = UINode(role: "AXScrollArea", label: "Students", children: (1...3).map {
            UINode(role: "AXStaticText", label: "SYNTH-ROW\($0) Student \($0)") })
        let s = try req(win([list])); print("P2:", s)
        XCTAssertFalse(s.contains("SYNTH-ROW"), "rows sent")
    }
    func testP3_SplitAcrossSiblings() throws {
        let s = try req(win([UINode(role: "AXStaticText", label: "Aiden"), UINode(role: "AXStaticText", label: "Tan")])); print("P3:", s)
        XCTAssertFalse(s.contains("\"Aiden\"") && s.contains("\"Tan\""), "input split across siblings sent")
    }
    func testP4_Subrole() throws {
        let s = try req(win([UINode(role: "AXGroup", label: "x", subrole: "Aiden Tan")])); print("P4:", s)
        XCTAssertFalse(s.contains("Aiden Tan"), "input sent in subrole")
    }
    func testP5_CurlyApostrophe() throws {
        let s = try req(win([UINode(role: "AXStaticText", label: "Welcome, Aiden O\u{2019}Neil")]), name: "Aiden O'Neil"); print("P5:", s)
        XCTAssertFalse(s.contains("Neil"), "input with apostrophe variant sent")
    }
    func testP6_ZeroWidth() throws {
        let s = try req(win([UINode(role: "AXStaticText", label: "Welcome, Aiden\u{200B}Tan")]), name: "AidenTan"); print("P6:", s)
        XCTAssertFalse(s.contains("Aiden"), "input with ZWSP in label sent")
    }
    func testP7_EmailLocalPart() throws {
        let s = try req(win([UINode(role: "AXStaticText", label: "Signed in as aiden.tan")]), email: "aiden.tan@x.example.test"); print("P7:", s)
        XCTAssertFalse(s.contains("aiden.tan"), "email local part sent")
    }
    func testP8_SpacedPhone() throws {
        let s = try req(win([UINode(role: "AXStaticText", label: "Contact 9123 4567")]), name: "91234567"); print("P8:", s)
        XCTAssertFalse(s.contains("9123 4567"), "formatted input sent")
    }
    func testP9_CellLabel() throws {
        let s = try req(win([UINode(role: "AXGroup", children: [UINode(role: "AXCell", label: "SYNTH-CELL Lim Bee Hoon")])])); print("P9:", s)
        XCTAssertFalse(s.contains("SYNTH-CELL"), "cell text sent")
    }
}
