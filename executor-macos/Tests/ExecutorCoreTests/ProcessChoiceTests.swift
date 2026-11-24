import XCTest
@testable import ExecutorCore

/// Which process resolve and replay read when several share one bundle id.
final class ProcessChoiceTests: XCTestCase {
    private let two: [pid_t] = [300, 200]
    private func choose(_ running: [pid_t], wanted: pid_t? = nil, windows: Set<pid_t>) throws -> pid_t {
        try AXApp.choose(bundleId: "com.google.Chrome", running: running, wanted: wanted,
                         requireUnique: true, hasWindow: { windows.contains($0) })
    }

    func testPidResolvesInThatProcessOnly() throws {
        XCTAssertEqual(try choose(two, wanted: 300, windows: [200, 300]), 300)
        XCTAssertEqual(try choose(two, wanted: 200, windows: [200, 300]), 200)
    }

    func testTwoCandidatesWithoutPidAreAmbiguousNotPicked() {
        XCTAssertThrowsError(try choose(two, windows: [200, 300])) { e in
            guard case AXApp.Failure.ambiguous(_, let pids) = e else { return XCTFail("got \(e)") }
            XCTAssertEqual(pids, [200, 300])
        }
    }

    func testOneWindowedProcessNeedsNoPid() throws {
        XCTAssertEqual(try choose(two, windows: [200]), 200)
    }

    func testPidThatIsNotThisAppIsRefused() {
        XCTAssertThrowsError(try choose(two, wanted: 999, windows: [200, 300])) { e in
            guard case AXApp.Failure.pidNotThisApp(999, _) = e else { return XCTFail("got \(e)") }
        }
    }

    func testNoWindowedProcessIsNotRunning() {
        XCTAssertThrowsError(try choose(two, windows: [])) { e in
            guard case AXApp.Failure.notRunning = e else { return XCTFail("got \(e)") }
        }
    }
}
