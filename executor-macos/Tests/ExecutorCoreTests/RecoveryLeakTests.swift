import XCTest
@testable import ExecutorCore

final class RecoveryLeakTests: XCTestCase {
    let policy = RecoveryPolicy(classification: "official-closed")
    func req(_ root: UINode, name: String = "Aiden Tan") throws -> String {
        let t = try makeTask()
        let r = RecoveryRequest.build(step: t.steps[0], task: t, inputs: try makeInputs(t, name: name), root: root,
                                      divergence: "element not found within 30 s", attempt: 1, policy: policy) { _ in "FIELDVAL" }
        guard case .success(let b) = r else { XCTFail("build"); return "" }
        return json(b.json)
    }
    func win(_ kids: [UINode]) -> UINode { UINode(role: "AXWindow", label: "W", children: [UINode(role: "AXWebArea", children: kids)]) }

    func testListRows() throws {
        let table = UINode(role: "AXTable", label: "Students", children: [
            UINode(role: "AXRow", children: [UINode(role: "AXCell", children: [UINode(role: "AXStaticText", label: "SYNTH-ROW Lim Bee Hoon S9999999Z")])]),
        ])
        let s = try req(win([table]))
        print("LISTROWS:", s)
        XCTAssertFalse(s.contains("SYNTH-ROW"), "list row text was sent")
    }
    func testTrailingSpaceInput() throws {
        let s = try req(win([UINode(role: "AXStaticText", label: "Welcome back, Aiden Tan")]), name: "Aiden Tan ")
        print("TRAILING:", s)
        XCTAssertFalse(s.contains("Aiden Tan"), "input value sent inside a label")
    }
    func testNbspLabel() throws {
        let s = try req(win([UINode(role: "AXStaticText", label: "Welcome back, Aiden\u{00A0}Tan")]))
        print("NBSP:", s)
        XCTAssertFalse(s.contains("Aiden"), "input value sent inside a label")
    }
    func testOneCharInput() throws {
        let t = try makeTask()
        let inputs = try RunInputs.load(from: JSONSerialization.data(withJSONObject: ["full_name": "Q", "email": "a@x.example.test"]), for: t)
        let r = RecoveryRequest.build(step: t.steps[0], task: t, inputs: inputs, root: win([UINode(role: "AXStaticText", label: "Q")]),
                                      divergence: "x", attempt: 1, policy: policy) { _ in nil }
        guard case .success(let b) = r else { return XCTFail() }
        let s = json(b.json); print("ONECHAR:", s)
        XCTAssertFalse(s.contains("\"label\":\"Q\""), "1-char input sent as label")
    }
}
