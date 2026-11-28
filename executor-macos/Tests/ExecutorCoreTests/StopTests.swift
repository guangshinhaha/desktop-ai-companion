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
        XCTAssertEqual(r.report.outcome, .stopped(step: "s1"))
        XCTAssertLessThanOrEqual(h.clock - (stopAt ?? 99), 0.5, "the wait is cut into slices")
        XCTAssertLessThan(h.clock, 5, "it did not wait out the 30 s ceiling")
        XCTAssertTrue(h.actions.isEmpty)
        XCTAssertEqual(r.report.completed, [])
        XCTAssertEqual(r.report.notRun, ["s1", "s2", "s3"])
    }

    func testStopBeforeTheFirstStepActsOnNothing() throws {
        let h = formHost()
        let r = try run(h, poll: { Self.stopLine })
        XCTAssertEqual(r.report.outcome, .stopped(step: "s1"))
        XCTAssertTrue(h.actions.isEmpty)
        XCTAssertEqual(r.audit.kinds, [.runStarted, .runStopped, .runFinished])
    }

    func testStopBetweenStepsKeepsWhatWasDoneAndReportsTheRest() throws {
        let h = formHost()
        let r = try run(h, poll: once(Self.stopLine) { h.actions.count >= 1 })
        XCTAssertEqual(r.report.outcome, .stopped(step: "s2"))
        XCTAssertEqual(r.report.completed, ["s1"])
        XCTAssertEqual(r.report.notRun, ["s2", "s3"])
        XCTAssertEqual(h.actions, ["set:full-name"], "nothing after the stop; the field the officer's run filled stays as it is")
    }

    func testStopWhileTheCardIsOpenIsRecordedAsAStopNotARejectionAndPressesNothing() throws {
        let h = formHost()
        var src: ScriptedApproval?
        let r = try run(h, answers: ["n"], poll: once(Self.stopLine) { (src?.asked ?? 0) > 0 }, asked: { src = $0 })
        XCTAssertEqual(r.report.outcome, .stopped(step: "s3"))
        XCTAssertEqual(r.report.completed, ["s1", "s2"])
        XCTAssertFalse(h.actions.contains("press:Create account"))
        XCTAssertEqual(r.audit.kinds, [.runStarted, .runStopped, .runFinished])
    }

    func testStopAfterApprovalButBeforeTheActionDoesNotPress() throws {
        let h = formHost()
        var src: ScriptedApproval?
        // The officer says y, and the stop arrives right behind it.
        let r = try run(h, answers: ["y"], poll: once(Self.stopLine) { (src?.asked ?? 0) > 0 }, asked: { src = $0 })
        XCTAssertEqual(r.report.outcome, .stopped(step: "s3"))
        XCTAssertFalse(h.actions.contains("press:Create account"))
        XCTAssertFalse(r.audit.kinds.contains(.approved), "an approval for an action that was not taken is not recorded")
    }

    func testStopWhileCheckingAStepThatWasActedOnSaysSo() throws {
        let h = FakeHost()
        h.tree = provisioningForm() // pressing does nothing visible, so the check waits
        let r = try run(h, poll: once(Self.stopLine) { h.actions.contains("press:Create account") && h.clock >= 1 })
        XCTAssertEqual(r.report.outcome, .stopped(step: "s3"))
        XCTAssertTrue(r.report.actionTried)
        XCTAssertEqual(r.report.completed, ["s1", "s2"])
        XCTAssertEqual(r.report.notRun, [], "the pressed step is not listed as not run (#138)")
        XCTAssertEqual(h.actions.filter { $0.hasPrefix("press") }.count, 1)
        let shown = r.audit.events.first { $0.kind == .runStopped }?.shown
        XCTAssertEqual(shown, "Stopped by the officer at step s3. Completed: s1, s2. Not run: none. An action was attempted at step s3 and is not confirmed: check the app.")
    }

    /// #138, the second case: recovery typed, then the stop came while the target was still being looked for.
    func testAStopAfterRecoveryTypedWhileFindingTheTargetDoesNotListTheStepAsNotRun() throws {
        let h = FakeHost()
        h.tree = formBehindNotice()
        let control = RunControl()
        h.onSet = { _, _ in control.stop() }
        let t = try makeTask()
        let audit = RecordingAudit()
        let r = Replayer(task: t, inputs: try makeInputs(t), host: h, gate: ApprovalGate(source: ScriptedApproval([]), out: { _ in }),
                         ceiling: 2, audit: audit, control: control,
                         recovery: RecoveryReplayTests.PickingModel([("Full name", RecoveryReplayTests.retype)]),
                         recoveryPolicy: RecoveryPolicy(classification: "official-closed"), log: { _ in }).run()
        guard case .stopped(step: "s1") = r.outcome else { return XCTFail("\(r.outcome)") }
        XCTAssertEqual(h.actions, ["set:full-name"], "only recovery's typing")
        XCTAssertEqual(r.completed, [])
        XCTAssertEqual(r.notRun, ["s2", "s3"])
        XCTAssertEqual(audit.events.first { $0.kind == .runStopped }?.shown,
                       "Stopped by the officer at step s1. Completed: none. Not run: s2, s3. An action was attempted at step s1 and is not confirmed: check the app.")
    }

    func testAStopOverRPCAfterAnActionSendsItAsUnconfirmedAndNotAsNotRun() throws {
        let h = FakeHost()
        h.tree = provisioningForm()
        var stopped = false
        let result = try serveRunPolling(h) { if !stopped && h.actions.contains("press:Create account") { stopped = true; return Self.stopLine }; return nil }
        XCTAssertEqual(result?["outcome"] as? String, "stopped")
        XCTAssertEqual(result?["not_run"] as? [String], [])
        XCTAssertEqual(result?["action_done"] as? String, "unknown", "attempted is not the same as done")
    }

    func testADivergedRunDoesNotListTheAttemptedStepAsNotRun() throws {
        let h = formHost()
        h.pressResult = .failed("the app refused")
        let result = try serveRunPolling(h) { nil }
        XCTAssertEqual(result?["outcome"] as? String, "diverged")
        XCTAssertEqual(result?["step"] as? String, "s3")
        XCTAssertEqual(result?["completed"] as? [String], ["s1", "s2"])
        XCTAssertEqual(result?["not_run"] as? [String], [])
        XCTAssertEqual(result?["action_done"] as? String, "unknown")
    }

    func testADivergedRunThatTriedNothingOnTheStepStillListsItAsNotRun() throws {
        let h = FakeHost()
        h.tree = formBehindNotice() // a dialog is open before the first step acts
        let result = try serveRunPolling(h) { nil }
        XCTAssertEqual(result?["outcome"] as? String, "diverged")
        XCTAssertEqual(result?["not_run"] as? [String], ["s1", "s2", "s3"])
        XCTAssertNil(result?["action_done"])
    }

    func serveRunPolling(_ h: FakeHost, poll: @escaping () -> String?) throws -> [String: Any]? {
        var out: [String] = []
        var queue = [String(decoding: try JSONSerialization.data(withJSONObject: [
            "jsonrpc": "2.0", "id": 1, "method": "replay.start",
            "params": ["task": taskJSON(), "inputs": ["full_name": "Aiden Tan", "email": "a@x.example.test"]]]), as: UTF8.self)]
        RPC.serve(input: {
            if !queue.isEmpty { return queue.removeFirst() }
            guard let last = out.last, let o = RPC.parse(last), let method = o["method"] as? String, let id = o["id"] else { return nil }
            if method == "approval.request" { return RPC.encode(["jsonrpc": "2.0", "id": id, "result": ["approved": true]]) }
            return RPC.encode(["jsonrpc": "2.0", "id": id, "result": ["seq": 1]])
        }, poll: poll, output: { out.append($0) }, hostFactory: { _, _ in h })
        return out.compactMap(RPC.parse).last { $0["id"] as? Int == 1 }?["result"] as? [String: Any]
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
        XCTAssertEqual(r.report.outcome, .stopped(step: "s1"))
        XCTAssertTrue(h.actions.isEmpty)
    }

    func testTheShellGoingAwayWhilePausedStopsTheRun() throws {
        let h = formHost()
        let r = try run(h, poll: once(Self.pauseLine) { true }, block: { nil })
        XCTAssertEqual(r.report.outcome, .stopped(step: "s1"))
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
        XCTAssertEqual(report.outcome, .stopped(step: "s1"))
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

    /// Runs a replay over the wire. `intercept` sees each request the executor sends and may return a line to
    /// deliver before the usual answer (a stop, say); the question is then answered as the shell would.
    func serveRun(_ h: FakeHost, intercept: @escaping (_ method: String, _ params: [String: Any]) -> String?) throws -> [[String: Any]] {
        var out: [String] = []
        var queue = [String(decoding: try JSONSerialization.data(withJSONObject: [
            "jsonrpc": "2.0", "id": 1, "method": "replay.start",
            "params": ["task": taskJSON(), "inputs": ["full_name": "Aiden Tan", "email": "a@x.example.test"]]]), as: UTF8.self)]
        var answered = Set<String>()
        RPC.serve(input: {
            if !queue.isEmpty { return queue.removeFirst() }
            guard let last = out.last, let o = RPC.parse(last), let method = o["method"] as? String, let id = o["id"] as? String else { return nil }
            let params = o["params"] as? [String: Any] ?? [:]
            if !answered.contains(id) {
                answered.insert(id)
                if let line = intercept(method, params) { return line }
            }
            if method == "approval.request" { return RPC.encode(["jsonrpc": "2.0", "id": id, "result": ["approved": true]]) }
            return RPC.encode(["jsonrpc": "2.0", "id": id, "result": ["seq": 1]])
        }, output: { out.append($0) }, hostFactory: { _, _ in h })
        return out.compactMap(RPC.parse)
    }

    func events(_ msgs: [[String: Any]]) -> [String] {
        msgs.filter { $0["method"] as? String == "audit.append" }.compactMap { ($0["params"] as? [String: Any])?["event"] as? String }
    }

    func testAStopWhileRunStartedIsBeingRecordedIsAStopNotARefusal() throws {
        let h = formHost()
        let msgs = try serveRun(h) { m, p in m == "audit.append" && p["event"] as? String == "run_started" ? Self.stopLine : nil }
        let result = msgs.last { $0["id"] as? Int == 1 }?["result"] as? [String: Any]
        XCTAssertEqual(result?["outcome"] as? String, "stopped")
        XCTAssertEqual(events(msgs), ["run_started", "run_stopped", "run_finished"])
        XCTAssertTrue(h.actions.isEmpty)
    }

    func testAStopWhileTheApprovalIsBeingRecordedDoesNotLeaveAnApprovalForAnActionNeverTaken() throws {
        let h = formHost()
        let msgs = try serveRun(h) { m, p in m == "audit.append" && p["event"] as? String == "approved" ? Self.stopLine : nil }
        let result = msgs.last { $0["id"] as? Int == 1 }?["result"] as? [String: Any]
        XCTAssertEqual(result?["outcome"] as? String, "stopped")
        XCTAssertEqual(events(msgs), ["run_started", "approved", "run_stopped", "run_finished"])
        XCTAssertFalse(h.actions.contains("press:Create account"))
        let stop = msgs.compactMap { $0["params"] as? [String: Any] }.first { $0["event"] as? String == "run_stopped" }
        XCTAssertTrue((stop?["shown"] as? String ?? "").contains("approval for step s3 was recorded and its action was not taken"), "\(String(describing: stop))")
    }

    func testAPausePressedWhileTheCardIsOpenHoldsTheApprovedActionUntilResumed() throws {
        let h = formHost()
        var src: ScriptedApproval?
        var atResume: (actions: Int, approved: Bool)?
        let audit = RecordingAudit()
        audit.host = h
        let t = try makeTask()
        let s = ScriptedApproval(["y"]); src = s
        let ctl = RunControl(poll: once(Self.pauseLine) { (src?.asked ?? 0) > 0 }, block: {
            atResume = (h.actions.filter { $0.hasPrefix("press") }.count, audit.kinds.contains(.approved))
            return Self.resumeLine
        })
        let r = Replayer(task: t, inputs: try makeInputs(t), host: h, gate: ApprovalGate(source: s, out: { _ in }), audit: audit, control: ctl, log: { _ in }).run()
        XCTAssertEqual(r.outcome, .finished)
        XCTAssertEqual(atResume?.actions, 0, "the approved press waits while paused")
        XCTAssertEqual(atResume?.approved, false, "and is not recorded as approved until it can run")
    }

    // MARK: a stop on a run that diverges (#105)

    /// The one `run_stopped` of a diverged run: after `step_diverged`, with `run_finished` still "diverged".
    func divergedStop(_ kinds: [String], _ shown: [String?], file: StaticString = #filePath, line: UInt = #line) -> String {
        XCTAssertEqual(Array(kinds.suffix(3)), ["step_diverged", "run_stopped", "run_finished"], file: file, line: line)
        XCTAssertEqual(kinds.filter { $0 == "run_stopped" }.count, 1, file: file, line: line)
        XCTAssertEqual(shown.last ?? nil, "diverged", file: file, line: line)
        return shown.dropLast().last.flatMap { $0 } ?? ""
    }

    func testAStopDuringTheApprovalRecordOfARunThatThenDivergesIsRecorded() throws {
        let h = formHost()
        // The stop arrives while the approval is being recorded, and the window goes before the check after it.
        let msgs = try serveRun(h) { m, p in
            guard m == "audit.append", p["event"] as? String == "approved" else { return nil }
            h.tree = nil
            return Self.stopLine
        }
        XCTAssertEqual((msgs.last { $0["id"] as? Int == 1 }?["result"] as? [String: Any])?["outcome"] as? String, "diverged")
        let records = msgs.filter { $0["method"] as? String == "audit.append" }.compactMap { $0["params"] as? [String: Any] }
        let shown = divergedStop(events(msgs), records.map { $0["shown"] as? String })
        XCTAssertEqual(records.dropLast().last?["step"] as? String, "s3")
        XCTAssertEqual(shown, "Stopped by the officer. The run had already diverged at step s3. Completed: s1, s2. Not run: s3. The approval for step s3 was recorded and its action was not taken.")
        XCTAssertFalse(h.actions.contains("press:Create account"))
    }

    func testAStopDuringAFailedActionIsRecordedAndTheActionIsNotCalledNotRun() throws {
        let h = formHost()
        h.onPress = nil
        h.pressResult = .failed("the press was refused")
        let r = try run(h, poll: once(Self.stopLine) { h.actions.contains("press:Create account") })
        guard case .diverged(let step, _) = r.report.outcome else { return XCTFail("\(r.report.outcome)") }
        XCTAssertEqual(step, "s3")
        let shown = divergedStop(r.audit.kinds.map(\.rawValue), r.audit.events.map(\.shown))
        XCTAssertEqual(shown, "Stopped by the officer. The run had already diverged at step s3. Completed: s1, s2. Not run: none. An action was attempted at step s3 and is not confirmed: check the app.")
    }

    func testAnActionRecoveryTookCountsAsAttemptedInTheStopOfADivergedRun() throws {
        // A notice is in front of step s1, so the step's own action never happens: only recovery types.
        let h = FakeHost()
        h.tree = formBehindNotice()
        let t = try makeTask()
        final class TypeThenNonsense: RecoverySource {
            var asked = 0
            func ask(_ request: [String: Any]) -> [String: Any]? {
                asked += 1
                if asked > 1 { return ["answer": "nonsense"] }
                var a = RecoveryReplayTests.retype; a["ref"] = ref(in: request, label: "Full name") ?? "n-none"
                return a
            }
        }
        let audit = RecordingAudit()
        audit.host = h
        // The stop arrives as the refusal of the second answer is recorded, after the last stop check.
        let control = RunControl(poll: once(Self.stopLine) { audit.kinds.contains(.stepDiverged) })
        let r = Replayer(task: t, inputs: try makeInputs(t), host: h, gate: ApprovalGate(source: ScriptedApproval([]), out: { _ in }),
                         ceiling: 2, audit: audit, control: control, recovery: TypeThenNonsense(),
                         recoveryPolicy: RecoveryPolicy(classification: "official-closed"), log: { _ in }).run()
        guard case .diverged = r.outcome else { return XCTFail("\(r.outcome)") }
        XCTAssertEqual(h.actions, ["set:full-name"])
        XCTAssertEqual(audit.kinds, [.runStarted, .recovered, .stepDiverged, .stepDiverged, .runStopped, .runFinished])
        XCTAssertEqual(audit.events.first { $0.kind == .runStopped }?.shown,
                       "Stopped by the officer. The run had already diverged at step s1. Completed: none. Not run: s2, s3. An action was attempted at step s1 and is not confirmed: check the app.")
    }

    func testARunWithNoStopThatDivergesWritesNoStop() throws {
        let h = formHost()
        h.onPress = nil
        h.pressResult = .failed("the press was refused")
        let r = try run(h, poll: { nil })
        guard case .diverged = r.report.outcome else { return XCTFail("\(r.report.outcome)") }
        XCTAssertEqual(r.audit.kinds, [.runStarted, .approved, .stepDiverged, .runFinished])
        XCTAssertEqual(r.audit.events.last?.shown, "diverged")
    }

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
