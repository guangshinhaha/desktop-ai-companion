import Foundation
import XCTest
@testable import ExecutorCore

final class RPCTests: XCTestCase {
    private func parse(_ s: String?) throws -> [String: Any] {
        let s = try XCTUnwrap(s)
        return try XCTUnwrap(JSONSerialization.jsonObject(with: Data(s.utf8)) as? [String: Any])
    }

    func testPingReturnsPongWithSameId() throws {
        let r = try parse(RPC.handle(line: #"{"jsonrpc":"2.0","id":7,"method":"ping"}"#))
        XCTAssertEqual(r["id"] as? Int, 7)
        XCTAssertEqual((r["result"] as? [String: Any])?["pong"] as? Bool, true)
    }

    func testUnknownMethodIsMinus32601() throws {
        let r = try parse(RPC.handle(line: #"{"jsonrpc":"2.0","id":"a","method":"nope"}"#))
        XCTAssertEqual((r["error"] as? [String: Any])?["code"] as? Int, -32601)
        XCTAssertEqual(r["id"] as? String, "a")
    }

    func testGarbageIsParseError() throws {
        let r = try parse(RPC.handle(line: "not json"))
        XCTAssertEqual((r["error"] as? [String: Any])?["code"] as? Int, -32700)
    }

    func testMissingMethodIsInvalidRequest() throws {
        let r = try parse(RPC.handle(line: #"{"jsonrpc":"2.0","id":1}"#))
        XCTAssertEqual((r["error"] as? [String: Any])?["code"] as? Int, -32600)
    }

    func testNotificationGetsNoReply() {
        XCTAssertNil(RPC.handle(line: #"{"jsonrpc":"2.0","method":"ping"}"#))
    }
}
