import XCTest
@testable import ExecutorCore

final class PlaceholderTests: XCTestCase {
    func testPackageIsWired() {
        XCTAssertEqual(ExecutorCore.name, "executor-macos")
    }
}
