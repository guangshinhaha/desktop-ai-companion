import XCTest
@testable import ExecutorCore

final class DescriptorTests: XCTestCase {
    func testDecodesEverySchemaField() throws {
        let json = #"{"role":"AXTextField","label":"Full name","label_matches":"Full*","dom_id":"full-name","path":["AXWebArea","AXTextField:1"],"anchors":["Role"]}"#
        let d = try Descriptor.load(from: Data(json.utf8))
        XCTAssertEqual(d, Descriptor(role: "AXTextField", label: "Full name", labelMatches: "Full*", domId: "full-name",
                                     path: ["AXWebArea", "AXTextField:1"], anchors: ["Role"]))
    }

    func testReadsTheTargetOfAStep() throws {
        let json = #"{"id":"s9","target":{"role":"AXButton","label":"Create account","dom_id":"create"}}"#
        let d = try Descriptor.load(from: Data(json.utf8))
        XCTAssertEqual(d.domId, "create")
    }

    func testRejectsAnEmptyDescriptor() {
        XCTAssertThrowsError(try Descriptor.load(from: Data("{}".utf8)))
        XCTAssertThrowsError(try Descriptor.load(from: Data(#"{"anchors":["x"]}"#.utf8)))
        XCTAssertThrowsError(try Descriptor.load(from: Data("[]".utf8)))
    }

    /// Every target in the schema package's example task decodes, so the two cannot drift apart.
    func testEveryTargetInTheSchemaFixtureDecodes() throws {
        let url = URL(fileURLWithPath: #filePath)
            .deletingLastPathComponent().deletingLastPathComponent().deletingLastPathComponent().deletingLastPathComponent()
            .appendingPathComponent("schema/fixtures/provision-account.json")
        let task = try JSONSerialization.jsonObject(with: Data(contentsOf: url)) as! [String: Any]
        let steps = task["steps"] as! [[String: Any]]
        var n = 0
        for step in steps {
            guard let target = step["target"] else { continue }
            _ = try Descriptor.load(from: JSONSerialization.data(withJSONObject: target))
            n += 1
        }
        XCTAssertGreaterThan(n, 0)
    }
}
