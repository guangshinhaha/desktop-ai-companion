import XCTest
@testable import ExecutorCore

/// Live progress, pause and stop (PRD C-3, C-4). Synthetic data only.
final class StopTests: XCTestCase {
    static let stopLine = #"{"jsonrpc":"2.0","method":"replay.stop"}"#
    static let pauseLine = #"{"jsonrpc":"2.0","method":"replay.pause"}"#
    static let resumeLine = #"{"jsonrpc":"2.0","method":"replay.resume"}"#

    func formHost() -> FakeHost {
        let h = FakeHost()
        h.tree = provisioningForm()
        h.onPress = { n, h in if n.domId == "create" { h.tree = createdPage() } }
        return h
    }

    /// Hands out `line` once, the first time `when` is true.
    func once(_ line: String, when: @escaping () -> Bool) -> () -> String? {
        var sent = false
        return { if !sent && when() { sent = true; return line }; return nil }
    }

    struct Run {
        var report: ReplayReport
        var audit: RecordingAudit
        var progress: [ReplayProgress]
    }

    func run(_ h: FakeHost, task: ReplayTask? = nil, answers: [String?] = ["y"], poll: @escaping () -> String?,
             block: (() -> String?)? = nil, ceiling: TimeInterval = 30,
             asked: ((ScriptedApproval) -> Void)? = nil) throws -> Run {
        let t = try task ?? makeTask()
        let src = ScriptedApproval(answers)
        asked?(src)
        let audit = RecordingAudit()
        audit.host = h
        var seen: [ReplayProgress] = []
        let r = Replayer(task: t, inputs: try makeInputs(t), host: h, gate: ApprovalGate(source: src, out: { _ in }), ceiling: ceiling,
                         audit: audit, control: RunControl(poll: poll, block: block), progress: { seen.append($0) }, log: { _ in })
        return Run(report: r.run(), audit: audit, progress: seen)
    }

    // MARK: progress

    func testProgressNamesEachStepWhenItStartsAndWhenItIsDone() throws {
        let h = formHost()
        let r = try run(h, poll: { nil })
        XCTAssertEqual(r.report.outcome, .finished)
        XCTAssertEqual(r.progress.map { "\($0.kind.rawValue):\($0.step):\($0.index)/\($0.total)" },
                       ["started:s1:1/3", "done:s1:1/3", "started:s2:2/3", "done:s2:2/3", "started:s3:3/3", "done:s3:3/3"])
        XCTAssertEqual(r.progress[0].say, "Type into Full name")
    }

    // MARK: stop

    func testStopWhileAStepWaitsEndsWithinHalfASecondAndDoesNothing() throws {
        let h = FakeHost()
        h.tree = createdPage() // the Full name field never appears, so step s1 waits
        var stopAt: TimeInterval?
        let poll = once(Self.stopLine) { if h.clock >= 1.0 { stopAt = h.clock; return true }; return false }
        let r = try run(h, poll: poll)
        XCTAssertEqual(r.report.outcome, .stopped(step: "s1", actionDone: false))
        XCTAssertLessThanOrEqual(h.clock - (stopAt ?? 99), 0.5, "the wait is cut into slices")
        XCTAssertLessThan(h.clock, 5, "it did not wait out the 30 s ceiling")
        XCTAssertTrue(h.actions.isEmpty)
        XCTAssertEqual(r.report.completed, [])
        XCTAssertEqual(r.report.notRun, ["s1", "s2", "s3"])
    }

    func testStopBeforeTheFirstStepActsOnNothing() throws {
        let h = formHost()
        let r = try run(h, poll: { Self.stopLine })
        XCTAssertEqual(r.report.outcome, .stopped(step: "s1", actionDone: false))
        XCTAssertTrue(h.actions.isEmpty)
        XCTAssertEqual(r.audit.kinds, [.runStarted, .runStopped, .runFinished])
    }

    func testStopBetweenStepsKeepsWhatWasDoneAndReportsTheRest() throws {
        let h = formHost()
        let r = try run(h, poll: once(Self.stopLine) { h.actions.count >= 1 })
        XCTAssertEqual(r.report.outcome, .stopped(step: "s2", actionDone: false))
        XCTAssertEqual(r.report.completed, ["s1"])
        XCTAssertEqual(r.report.notRun, ["s2", "s3"])
        XCTAssertEqual(h.actions, ["set:full-name"], "nothing after the stop; the field the officer's run filled stays as it is")
    }

    func testStopWhileTheCardIsOpenIsRecordedAsAStopNotARejectionAndPressesNothing() throws {
        let h = formHost()
        var src: ScriptedApproval?
        let r = try run(h, answers: ["n"], poll: once(Self.stopLine) { (src?.asked ?? 0) > 0 }, asked: { src = $0 })
        XCTAssertEqual(r.report.outcome, .stopped(step: "s3", actionDone: false))
        XCTAssertEqual(r.report.completed, ["s1", "s2"])
        XCTAssertFalse(h.actions.contains("press:Create account"))
        XCTAssertEqual(r.audit.kinds, [.runStarted, .runStopped, .runFinished])
    }

    func testStopAfterApprovalButBeforeTheActionDoesNotPress() throws {
        let h = formHost()
        var src: ScriptedApproval?
        // The officer says y, and the stop arrives right behind it.
        let r = try run(h, answers: ["y"], poll: once(Self.stopLine) { (src?.asked ?? 0) > 0 }, asked: { src = $0 })
        XCTAssertEqual(r.report.outcome, .stopped(step: "s3", actionDone: false))
        XCTAssertFalse(h.actions.contains("press:Create account"))
        XCTAssertFalse(r.audit.kinds.contains(.approved), "an approval for an action that was not taken is not recorded")
    }

    func testStopWhileCheckingAStepThatWasActedOnSaysSo() throws {
        let h = FakeHost()
        h.tree = provisioningForm() // pressing does nothing visible, so the check waits
        let r = try run(h, poll: once(Self.stopLine) { h.actions.contains("press:Create account") && h.clock >= 1 })
        XCTAssertEqual(r.report.outcome, .stopped(step: "s3", actionDone: true))
        XCTAssertEqual(r.report.completed, ["s1", "s2"])
        XCTAssertEqual(r.report.notRun, ["s3"])
        XCTAssertEqual(h.actions.filter { $0.hasPrefix("press") }.count, 1)
        let shown = r.audit.events.first { $0.kind == .runStopped }?.shown ?? ""
        XCTAssertTrue(shown.contains("action was done"), shown)
    }

    func testTheStopIsInTheAuditTrailWithWhatCompletedAndWhatDidNot() throws {
        let h = formHost()
        let r = try run(h, poll: once(Self.stopLine) { h.actions.count >= 2 })
        XCTAssertEqual(r.audit.kinds, [.runStarted, .runStopped, .runFinished])
        let stop = try XCTUnwrap(r.audit.events.first { $0.kind == .runStopped })
        XCTAssertEqual(stop.step, "s3")
        XCTAssertEqual(stop.shown, "Stopped by the officer at step s3. Completed: s1, s2. Not run: s3.")
        XCTAssertEqual(r.audit.events.last?.shown, "stopped")
        // No run value in it (decision 0005 keeps those in the approval record only).
        XCTAssertFalse(r.audit.events.compactMap(\.shown).joined().contains("Aiden"))
    }

    func testAStopWithNoSwitchOfControlStillLetsAFinishedRunFinish() throws {
        let h = formHost()
        let r = try run(h, poll: once(Self.stopLine) { h.actions.count >= 3 }) // arrives after the last action
        XCTAssertEqual(r.report.outcome, .finished)
    }

    // MARK: pause

    func testPauseHoldsBeforeTheNextStepUntilResumed() throws {
        let h = formHost()
        var actionsWhilePaused = -1
        let r = try run(h, poll: once(Self.pauseLine) { true }, block: {
            actionsWhilePaused = h.actions.count
            return Self.resumeLine
        })
        XCTAssertEqual(actionsWhilePaused, 0, "nothing is done while paused")
        XCTAssertEqual(r.report.outcome, .finished)
        XCTAssertEqual(r.progress.prefix(3).map { "\($0.kind.rawValue):\($0.step)" }, ["paused:s1", "resumed:s1", "started:s1"])
    }

    func testStopWhilePausedEndsTheRunWithoutActing() throws {
        let h = formHost()
        let r = try run(h, poll: once(Self.pauseLine) { true }, block: { Self.stopLine })
        XCTAssertEqual(r.report.outcome, .stopped(step: "s1", actionDone: false))
        XCTAssertTrue(h.actions.isEmpty)
    }

    func testTheShellGoingAwayWhilePausedStopsTheRun() throws {
        let h = formHost()
        let r = try run(h, poll: once(Self.pauseLine) { true }, block: { nil })
        XCTAssertEqual(r.report.outcome, .stopped(step: "s1", actionDone: false))
        XCTAssertTrue(h.actions.isEmpty)
    }

    // MARK: timing on the real clock

    final class SleepyHost: ReplayHost {
        let tree = createdPage()
        func snapshot() -> UINode? { tree }
        func wait(_ timeout: TimeInterval) { Thread.sleep(forTimeInterval: timeout) }
        func now() -> TimeInterval { ProcessInfo.processInfo.systemUptime }
        func press(_ node: UINode) -> ActionResult { .failed("unused") }
        func setValue(_ node: UINode, to text: String) -> ActionResult { .failed("unused") }
        func choose(_ node: UINode, option: String) -> ActionResult { .failed("unused") }
        func value(of node: UINode) -> String? { nil }
    }

    func testAStopSentDuringARealWaitIsSeenWithin500ms() throws {
        let box = NSLock()
        var line: String?
        var sentAt = 0.0
        let t = try makeTask()
        let r = Replayer(task: t, inputs: try makeInputs(t), host: SleepyHost(),
                         gate: ApprovalGate(source: ScriptedApproval([]), out: { _ in }),
                         control: RunControl(poll: { box.lock(); defer { box.unlock() }; defer { line = nil }; return line }), log: { _ in })
        DispatchQueue.global().asyncAfter(deadline: .now() + 0.35) {
            box.lock(); line = Self.stopLine; sentAt = ProcessInfo.processInfo.systemUptime; box.unlock()
        }
        let report = r.run()
        let endedAt = ProcessInfo.processInfo.systemUptime
        XCTAssertEqual(report.outcome, .stopped(step: "s1", actionDone: false))
        box.lock(); defer { box.unlock() }
        XCTAssertLessThan(endedAt - sentAt, 0.5)
    }

    // MARK: waiting

    func testACancelledWaitReturnsAtOnceAndDoesNotChangeTheBackoffWhenNothingCancels() {
        var clock = 0.0
        var waits: [Double] = []
        var cancel = false
        let r: Waiting.Result<Void> = Waiting.until(ceiling: 20, wait: { waits.append($0); clock += $0; if clock >= 0.45 { cancel = true } },
                                                    now: { clock }, cancelled: { cancel }, poll: { .keepWaiting })
        guard case .cancelled = r else { return XCTFail("\(r)") }
        XCTAssertTrue(waits.allSatisfy { $0 <= Waiting.cancelSlice + 1e-9 })
        XCTAssertLessThanOrEqual(clock, 0.55)
    }

    func testAnEventStillEndsASlicedWaitEarly() {
        var clock = 0.0
        var polls = 0
        let r: Waiting.Result<Int> = Waiting.until(ceiling: 20, wait: { _ in clock += 0.01 }, now: { clock }, cancelled: { false }) {
            polls += 1
            return polls >= 3 ? .done(polls) : .keepWaiting
        }
        guard case .done = r else { return XCTFail() }
        XCTAssertLessThan(clock, 0.1, "an event woke the wait, so it polled again at once")
    }

    // MARK: over the wire

    func testAStopOverRPCEndsTheRunAndIsRecordedThroughTheShell() throws {
        let h = FakeHost()
        h.tree = createdPage()
        var out: [String] = []
        var queue = [String(decoding: try JSONSerialization.data(withJSONObject: [
            "jsonrpc": "2.0", "id": 1, "method": "replay.start",
            "params": ["task": taskJSON(), "inputs": ["full_name": "Aiden Tan", "email": "a@x.example.test"]]]), as: UTF8.self)]
        var pending = [Self.stopLine]
        RPC.serve(input: {
            if !queue.isEmpty { return queue.removeFirst() }
            guard let last = out.last, let o = RPC.parse(last), o["method"] as? String == "audit.append", let id = o["id"] else { return nil }
            return RPC.encode(["jsonrpc": "2.0", "id": id, "result": ["seq": 1]])
        }, poll: { h.waits.count >= 3 && !pending.isEmpty ? pending.removeFirst() : nil },
        output: { out.append($0) }, hostFactory: { _, _ in h })
        let msgs = out.compactMap(RPC.parse)
        let result = msgs.last { $0["id"] as? Int == 1 }?["result"] as? [String: Any]
        XCTAssertEqual(result?["outcome"] as? String, "stopped")
        XCTAssertEqual(result?["step"] as? String, "s1")
        XCTAssertEqual(result?["not_run"] as? [String], ["s1", "s2", "s3"])
        let events = msgs.filter { $0["method"] as? String == "audit.append" }.compactMap { ($0["params"] as? [String: Any])?["event"] as? String }
        XCTAssertEqual(events, ["run_started", "run_stopped", "run_finished"])
        let progress = msgs.filter { $0["method"] as? String == "run.progress" }
        XCTAssertEqual(progress.count, 1)
        XCTAssertNil(progress[0]["id"], "progress is a notification: nothing waits for it")
        XCTAssertEqual((progress[0]["params"] as? [String: Any])?["say"] as? String, "Type into Full name")
        XCTAssertTrue(h.actions.isEmpty)
    }

    func testAStopArrivingWhileTheCardIsOpenIsANo() throws {
        let h = formHostForRPC()
        var out: [String] = []
        var queue = [String(decoding: try JSONSerialization.data(withJSONObject: [
            "jsonrpc": "2.0", "id": 1, "method": "replay.start",
            "params": ["task": taskJSON(), "inputs": ["full_name": "Aiden Tan", "email": "a@x.example.test"]]]), as: UTF8.self)]
        RPC.serve(input: {
            if !queue.isEmpty { return queue.removeFirst() }
            guard let last = out.last, let o = RPC.parse(last), let method = o["method"] as? String, let id = o["id"] else { return nil }
            if method == "approval.request" { return Self.stopLine }
            return RPC.encode(["jsonrpc": "2.0", "id": id, "result": ["seq": 1]])
        }, output: { out.append($0) }, hostFactory: { _, _ in h })
        let msgs = out.compactMap(RPC.parse)
        XCTAssertEqual((msgs.last { $0["id"] as? Int == 1 }?["result"] as? [String: Any])?["outcome"] as? String, "stopped")
        XCTAssertFalse(h.actions.contains("press:Create account"))
    }

    func formHostForRPC() -> FakeHost { formHost() }

    // MARK: reading lines

    func testLineReaderPollsWithoutWaitingAndKeepsPartialLines() throws {
        var fds: [Int32] = [0, 0]
        XCTAssertEqual(pipe(&fds), 0)
        defer { close(fds[0]) }
        let reader = LineReader(fd: fds[0])
        XCTAssertNil(reader.pollLine(), "nothing there: returns at once")
        _ = "a\nb".withCString { write(fds[1], $0, 3) }
        XCTAssertEqual(reader.pollLine(), "a")
        XCTAssertNil(reader.pollLine(), "b has no line end yet")
        _ = "\n".withCString { write(fds[1], $0, 1) }
        XCTAssertEqual(reader.pollLine(), "b")
        _ = "c\n".withCString { write(fds[1], $0, 2) }
        close(fds[1])
        XCTAssertEqual(reader.readLine(), "c")
        XCTAssertNil(reader.readLine())
        XCTAssertNil(reader.pollLine())
    }
}
