import XCTest
@testable import ExecutorCore

/// Records every audit event, and can be told to refuse some.
final class RecordingAudit: AuditSink {
    var events: [AuditEvent] = []
    var failOn: Set<AuditEventKind> = []
    /// What the app had been told to do when each record was written (to prove the record comes first).
    var actionsSeen: [Int] = []
    var host: FakeHost?
    func record(_ e: AuditEvent) -> Bool {
        if failOn.contains(e.kind) { return false }
        events.append(e)
        actionsSeen.append(host?.actions.count ?? 0)
        return true
    }
    var kinds: [AuditEventKind] { events.map(\.kind) }
}

final class AuditVerifierTests: XCTestCase {
    /// Written by the shell's store (shell/src-tauri/tests/audit.rs keeps it in step), so this is the
    /// Rust hash checked by the Swift one.
    func fixture() throws -> [String] {
        let url = URL(fileURLWithPath: #filePath).deletingLastPathComponent().deletingLastPathComponent()
            .appendingPathComponent("fixtures/audit/trail.jsonl")
        return try String(contentsOf: url, encoding: .utf8).components(separatedBy: "\n")
    }

    func testTheStoresOwnTrailPasses() throws {
        XCTAssertEqual(AuditVerifier.verify(lines: try fixture()), .ok(records: 5))
    }

    func testAnEditedRecordFailsAndNamesItsSeq() throws {
        var l = try fixture()
        l[2] = l[2].replacingOccurrences(of: "Test Person", with: "Test Persoo")
        guard case .broken(let seq, _) = AuditVerifier.verify(lines: l) else { return XCTFail("must fail") }
        XCTAssertEqual(seq, 2)
        for field in ["officer", "event", "task", "run", "step", "item", "at"] {
            var m = try fixture()
            let before = m[3]
            m[3] = m[3].replacingOccurrences(of: "\"\(field)\":", with: "\"\(field)\":\"x\",\"\(field)_old\":")
            XCTAssertNotEqual(before, m[3], field)
            if case .broken(let s, _) = AuditVerifier.verify(lines: m) { XCTAssertEqual(s, 3, field) }
            else if case .unreadable = AuditVerifier.verify(lines: m) { /* a type change is unreadable, also a failure */ }
            else { XCTFail("\(field) edit passed") }
        }
    }

    func testNumbersThatOnlyLookLikeTheOriginalFail() throws {
        for (from, to) in [("\"at\":1700000005", "\"at\":1700000005.9"), ("\"at\":1700000005", "\"at\":1700000005e0"),
                           ("\"seq\":2", "\"seq\":2.7"), ("\"seq\":2", "\"seq\":2.0")] {
            var l = try fixture()
            XCTAssertTrue(l[2].contains(from), from)
            l[2] = l[2].replacingOccurrences(of: from, with: to)
            if case .ok = AuditVerifier.verify(lines: l) { XCTFail("\(to) passed") }
        }
    }

    func testAnExtraOrRepeatedKeyFails() throws {
        for tail in [",\"note\":\"hi\"}", ",\"officer\":\"Mallory\"}"] {
            var l = try fixture()
            l[2] = String(l[2].dropLast()) + tail
            guard case .broken(let seq, _) = AuditVerifier.verify(lines: l) else { return XCTFail(tail) }
            XCTAssertEqual(seq, 2)
        }
    }

    func testARemovedRecordFailsAndNamesTheMissingSeq() throws {
        let f = try fixture()
        for (drop, named) in [(1, 1), (3, 3), (5, 5)] as [(Int, Int64)] {
            var l = f
            l.remove(at: drop)   // line 0 is the control line, so line n is record n
            guard case .broken(let seq, _) = AuditVerifier.verify(lines: l) else { return XCTFail("record \(drop)") }
            XCTAssertEqual(seq, named, "removing record \(drop)")
        }
    }

    func testAnAppendedOrSwappedOrEmptyTrailFails() throws {
        let f = try fixture()
        var swapped = f
        swapped.swapAt(2, 3)
        if case .ok = AuditVerifier.verify(lines: swapped) { XCTFail("swap passed") }
        var doubled = f
        doubled.insert(f[2], at: 3)
        if case .ok = AuditVerifier.verify(lines: doubled) { XCTFail("duplicate passed") }
        if case .ok = AuditVerifier.verify(lines: []) { XCTFail("nothing passed") }
        // A wiped trail: control line only, which still names a head.
        if case .ok = AuditVerifier.verify(lines: [f[0]]) { XCTFail("wipe passed") }
    }
}

final class ApprovalFilePolicyTests: XCTestCase {
    func testAReleaseBuildRefusesTheApprovalFile() {
        XCTAssertNotNil(ApprovalFilePolicy.refusal(debugBuild: false))
        XCTAssertNil(ApprovalFilePolicy.refusal(debugBuild: true), "the harness runs a debug build")
    }
}

final class CardHardeningTests: XCTestCase {
    func testConstantsAreShownWithTheFieldEachGoesInto() throws {
        let t = try makeTask { steps(&$0) { s in
            s.insert(["id": "s0", "say": "Type Role", "app": "chrome", "action": "set_value",
                      "target": ["role": "AXTextField", "label": "Role", "dom_id": "role"],
                      "value": ["constant": "Teacher", "confirmed_no_personal_data": true], "expect": [["kind": "value_equals", "constant": "Teacher", "confirmed_no_personal_data": true]], "consequential": "no"], at: 0)
        } }
        let out = ApprovalGate.card(step: t.steps[3], task: t, inputs: try makeInputs(t), target: createButton)
        XCTAssertTrue(out.contains("into Role: \"Teacher\""), out)
    }

    func testValuesAreHiddenWhenOutputIsNotATerminal() throws {
        let t = try makeTask(), i = try makeInputs(t, name: "Mei Ling", email: "mei@x.example.test")
        var printed = ""
        let g = ApprovalGate(source: ScriptedApproval(["n"]), out: { printed += $0 + "\n" }, showValues: false)
        _ = g.request(step: t.steps[2], task: t, inputs: i, target: createButton)
        XCTAssertFalse(printed.contains("Mei Ling"))
        XCTAssertFalse(printed.contains("mei@x"))
        XCTAssertTrue(printed.contains("<hidden>"))
        XCTAssertTrue(printed.contains("Will do: press"), "the action is still shown")
    }

    func testTaskTextCannotReachTheLogWithEscapes() throws {
        let t = try makeTask { steps(&$0) { $0[0]["say"] = "Type\u{1B}[2J name"; $0[0]["id"] = "s1\u{1B}[2K" } }
        let h = FakeHost(); h.tree = provisioningForm()
        h.onPress = { n, h in if n.domId == "create" { h.tree = createdPage() } }
        var lines: [String] = []
        let r = Replayer(task: t, inputs: try makeInputs(t), host: h, gate: ApprovalGate(source: ScriptedApproval(["n"]), out: { _ in }), log: { lines.append($0) })
        _ = r.run()
        XCTAssertFalse(lines.joined().contains("\u{1B}"))
        XCTAssertTrue(lines.joined().contains("\\u{1B}"))
    }

    func testTheCardAsJSONCarriesEverythingTheShellShows() throws {
        let t = try makeTask(), i = try makeInputs(t)
        let j = ApprovalGate.makeCard(step: t.steps[2], task: t, inputs: i, target: createButton).json
        XCTAssertEqual(j["app"] as? String, "com.google.Chrome")
        XCTAssertEqual(j["undoable_claim"] as? Bool, false)
        XCTAssertEqual(j["summary"] as? String, "Create an account for Aiden Tan (a@x.example.test)")
        XCTAssertEqual((j["inputs"] as? [[String: String]])?.count, 2)
        XCTAssertTrue((j["will_do"] as? String ?? "").contains("press"))
    }
}

final class ReplayAuditTests: XCTestCase {
    func host() -> FakeHost {
        let h = FakeHost()
        h.tree = provisioningForm()
        h.onPress = { n, h in if n.domId == "create" { h.tree = createdPage() } }
        return h
    }

    func run(_ answers: [String?], audit: RecordingAudit, host h: FakeHost) throws -> ReplayReport {
        audit.host = h
        let t = try makeTask()
        let gate = ApprovalGate(source: ScriptedApproval(answers), out: { _ in })
        return Replayer(task: t, inputs: try makeInputs(t), host: h, gate: gate, audit: audit, log: { _ in }).run()
    }

    func testAnApprovedRunRecordsStartApprovalWithValuesAndFinish() throws {
        let a = RecordingAudit(), h = host()
        let rep = try run(["y"], audit: a, host: h)
        XCTAssertEqual(rep.outcome, .finished)
        XCTAssertEqual(a.kinds, [.runStarted, .approved, .runFinished])
        let approved = a.events[1]
        XCTAssertEqual(approved.step, "s3")
        XCTAssertTrue(approved.shown?.contains("Create an account for Aiden Tan (a@x.example.test)") ?? false, "decision 0005: the values are kept")
        XCTAssertTrue(approved.shown?.contains("Task says it can be undone: no") ?? false)
        XCTAssertEqual(a.events[2].shown, "finished")
    }

    func testTheApprovalIsRecordedBeforeTheActionHappens() throws {
        let a = RecordingAudit(), h = host()
        _ = try run(["y"], audit: a, host: h)
        // s1 and s2 had typed (2 actions) when the approval was written; the press (3rd) came after.
        XCTAssertEqual(a.actionsSeen[1], 2)
        XCTAssertEqual(h.actions.count, 3)
    }

    func testARejectionIsRecordedAndNothingIsPressed() throws {
        let a = RecordingAudit(), h = host()
        let rep = try run(["n"], audit: a, host: h)
        XCTAssertEqual(rep.outcome, .notApproved(step: "s3"))
        XCTAssertEqual(a.kinds, [.runStarted, .rejected, .runFinished])
        XCTAssertFalse(h.actions.contains { $0.hasPrefix("press") })
        XCTAssertEqual(a.events[2].shown, "not_approved")
    }

    func testNoAnswerIsARejectionToo() throws {
        let a = RecordingAudit(), h = host()
        let rep = try run([nil], audit: a, host: h)
        XCTAssertEqual(rep.outcome, .notApproved(step: "s3"))
        XCTAssertEqual(a.kinds, [.runStarted, .rejected, .runFinished])
    }

    func testIfTheApprovalCannotBeRecordedNothingIsPressed() throws {
        let a = RecordingAudit(), h = host()
        a.failOn = [.approved]
        let rep = try run(["y"], audit: a, host: h)
        guard case .diverged(let step, let why) = rep.outcome else { return XCTFail("must stop") }
        XCTAssertEqual(step, "s3")
        XCTAssertTrue(why.contains("audit"))
        XCTAssertFalse(h.actions.contains { $0.hasPrefix("press") })
        XCTAssertEqual(a.kinds, [.runStarted, .stepDiverged, .runFinished])
    }

    func testIfTheRunCannotBeRecordedAtAllNothingIsDone() throws {
        let a = RecordingAudit(), h = host()
        a.failOn = [.runStarted]
        let rep = try run(["y"], audit: a, host: h)
        guard case .refused = rep.outcome else { return XCTFail("must refuse") }
        XCTAssertTrue(h.actions.isEmpty)
    }

    func testADivergenceIsRecordedWithItsStep() throws {
        let a = RecordingAudit(), h = host()
        h.tree = createdPage()   // the form is not there
        let t = try makeTask()
        let r = Replayer(task: t, inputs: try makeInputs(t), host: h, gate: ApprovalGate(source: ScriptedApproval(["y"]), out: { _ in }),
                         ceiling: 1, audit: a, log: { _ in })
        _ = r.run()
        XCTAssertEqual(a.kinds, [.runStarted, .stepDiverged, .runFinished])
        XCTAssertEqual(a.events[1].step, "s1")
        XCTAssertEqual(a.events[2].shown, "diverged")
    }

    func testNoRunValueReachesAnyRecordExceptTheApprovedCard() throws {
        let a = RecordingAudit(), h = host()
        _ = try run(["n"], audit: a, host: h)
        // A rejected card is shown to the officer and kept (decision 0005); the other events never carry values.
        for e in a.events where e.kind != .rejected && e.kind != .approved {
            XCTAssertFalse((e.shown ?? "").contains("Aiden"))
        }
    }
}

/// The shell side of the conversation, played by a closure that answers each request the executor writes.
final class RPCReplayTests: XCTestCase {
    struct Shell {
        var approve: (Any?) -> Any? = { _ in true }
        var auditResult: (String) -> [String: Any]? = { _ in ["seq": 1] }
        var endAtApproval = false
    }

    func session(shell: Shell, task: [String: Any]? = nil, host h: FakeHost) throws -> (out: [[String: Any]], audit: [[String: Any]]) {
        var out: [String] = []
        var queue: [String] = []
        let start: [String: Any] = ["jsonrpc": "2.0", "id": 1, "method": "replay.start",
                                    "params": ["task": task ?? taskJSON(), "inputs": ["full_name": "Aiden Tan", "email": "a@x.example.test"]]]
        queue.append(String(decoding: try JSONSerialization.data(withJSONObject: start), as: UTF8.self))
        var seqn = 0
        RPC.serve(input: {
            if !queue.isEmpty { return queue.removeFirst() }
            guard let last = out.last, let o = RPC.parse(last), let method = o["method"] as? String, let id = o["id"] else { return nil }
            if method == "approval.request" {
                if shell.endAtApproval { return nil }
                return RPC.encode(["jsonrpc": "2.0", "id": id, "result": ["approved": shell.approve(o)!]])
            }
            seqn += 1
            let p = o["params"] as? [String: Any]
            if let r = shell.auditResult(p?["event"] as? String ?? "") { return RPC.encode(["jsonrpc": "2.0", "id": id, "result": r]) }
            return RPC.encode(["jsonrpc": "2.0", "id": id, "error": ["code": -32000, "message": "no"]])
        }, output: { out.append($0) }, hostFactory: { _, _ in h })
        let parsed = out.compactMap(RPC.parse)
        return (parsed, parsed.filter { $0["method"] as? String == "audit.append" })
    }

    func host() -> FakeHost {
        let h = FakeHost()
        h.tree = provisioningForm()
        h.onPress = { n, h in if n.domId == "create" { h.tree = createdPage() } }
        return h
    }

    func result(_ out: [[String: Any]]) -> [String: Any]? { out.last { $0["id"] as? Int == 1 }?["result"] as? [String: Any] }

    func testApproveOverRPCRunsTheTaskAndAuditsIt() throws {
        let h = host()
        let (out, audit) = try session(shell: Shell(), host: h)
        XCTAssertEqual(result(out)?["outcome"] as? String, "finished")
        XCTAssertEqual(audit.map { ($0["params"] as? [String: Any])?["event"] as? String }, ["run_started", "approved", "run_finished"])
        let card = out.first { $0["method"] as? String == "approval.request" }
        let c = (card?["params"] as? [String: Any])?["card"] as? [String: Any]
        XCTAssertEqual(c?["summary"] as? String, "Create an account for Aiden Tan (a@x.example.test)")
        XCTAssertEqual(c?["app"] as? String, "com.google.Chrome")
        XCTAssertEqual(c?["undoable_claim"] as? Bool, false)
        // The executor does not name an officer or a run: those are the shell's.
        XCTAssertNil(c?["officer"])
        XCTAssertTrue(h.actions.contains("press:Create account"))
    }

    func testRejectStopsAndPressesNothing() throws {
        let h = host()
        let (out, audit) = try session(shell: Shell(approve: { _ in false }), host: h)
        XCTAssertEqual(result(out)?["outcome"] as? String, "not_approved")
        XCTAssertFalse(h.actions.contains { $0.hasPrefix("press") })
        XCTAssertTrue(audit.contains { ($0["params"] as? [String: Any])?["event"] as? String == "rejected" })
    }

    func testOnlyTheJSONValueTrueApproves() throws {
        for bad: Any in ["true", "y", 1, "yes", NSNull(), ["approved": true]] {
            let h = host()
            let (out, _) = try session(shell: Shell(approve: { _ in bad }), host: h)
            XCTAssertNotEqual(result(out)?["outcome"] as? String, "finished", "\(bad)")
            XCTAssertFalse(h.actions.contains { $0.hasPrefix("press") }, "\(bad)")
        }
    }

    func testTheShellGoingAwayAtTheApprovalIsNotConsent() throws {
        let h = host()
        let (out, _) = try session(shell: Shell(endAtApproval: true), host: h)
        XCTAssertFalse(h.actions.contains { $0.hasPrefix("press") })
        // The input ended, so there may be no result line; if there is one, it is not a finish.
        XCTAssertNotEqual(result(out)?["outcome"] as? String, "finished")
    }

    func testAnAuditErrorFromTheShellStopsTheRunBeforeTheAction() throws {
        let h = host()
        let (out, _) = try session(shell: Shell(auditResult: { $0 == "approved" ? nil : ["seq": 1] }), host: h)
        XCTAssertEqual(result(out)?["outcome"] as? String, "diverged")
        XCTAssertFalse(h.actions.contains { $0.hasPrefix("press") })
    }

    func testAnAuditReplyWithoutASeqIsNotAcknowledgement() throws {
        let h = host()
        let (out, _) = try session(shell: Shell(auditResult: { _ in [:] }), host: h)
        XCTAssertEqual(result(out)?["outcome"] as? String, "refused")
        XCTAssertTrue(h.actions.isEmpty)
    }

    func testABadTaskIsAnErrorBeforeAnythingHappens() throws {
        let h = host()
        var bad = taskJSON()
        bad["format"] = "task/v9"
        let (out, audit) = try session(shell: Shell(), task: bad, host: h)
        XCTAssertNotNil((out.first { $0["id"] as? Int == 1 })?["error"])
        XCTAssertTrue(audit.isEmpty)
        XCTAssertTrue(h.actions.isEmpty)
    }

    func testACardTooBigToKeepIsNotApproved() throws {
        let h = host()
        var t = taskJSON()
        var steps = t["steps"] as! [[String: Any]]
        let i = steps.firstIndex { ($0["consequential"] as? String) == "yes" }!
        var ap = steps[i]["approval"] as? [String: Any] ?? [:]
        ap["summary"] = String(repeating: "x", count: 20_000)
        steps[i]["approval"] = ap
        t["steps"] = steps
        let (out, _) = try session(shell: Shell(), task: t, host: h)
        XCTAssertNotEqual(result(out)?["outcome"] as? String, "finished")
        XCTAssertFalse(h.actions.contains { $0.hasPrefix("press") })
    }

    func testPingStillWorks() {
        XCTAssertNotNil(RPC.handle(line: #"{"jsonrpc":"2.0","id":1,"method":"ping"}"#))
    }
}
