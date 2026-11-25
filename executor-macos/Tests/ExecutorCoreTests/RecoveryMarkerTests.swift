import XCTest
@testable import ExecutorCore

/// Every free-text string a task holds (say lines, ids, descriptor labels, globs, paths, anchors, check targets,
/// window title patterns), seeded with a marker. The encoded request must carry none of them, except a chrome
/// target's own label, glob and dom id under `step.target`, which the chrome rule allows.
final class RecoveryMarkerTests: XCTestCase {
    final class Asking: RecoverySource {
        var requests: [[String: Any]] = []
        func ask(_ request: [String: Any]) -> [String: Any]? { requests.append(request); return nil }
    }

    func seeded(_ checks: [[String: Any]]) throws -> ReplayTask {
        try makeTask { t in
            steps(&t) { s in
                for i in s.indices { s[i]["say"] = "MKSAY\(i) Type into Full name"; s[i]["id"] = "MKID\(i)" }
                var target = s[0]["target"] as! [String: Any]
                target["label_matches"] = "MKGLOB*"
                target["path"] = ["AXWindow:MKPATHW", "AXWebArea:MKPATHA", "AXTextField:MKPATHF"]
                target["anchors"] = ["MKANCHOR near the field"]
                s[0]["target"] = target
                s[0]["expect"] = checks
            }
        }
    }

    func requests(_ t: ReplayTask, tree: UINode, onSet: ((UINode, FakeHost) -> Void)? = nil) throws -> [String] {
        let h = FakeHost(); h.tree = tree; h.onSet = onSet
        let m = Asking()
        _ = Replayer(task: t, inputs: try makeInputs(t), host: h, gate: ApprovalGate(source: ScriptedApproval([]), out: { _ in }),
                     ceiling: 2, audit: RecordingAudit(), recovery: m,
                     recoveryPolicy: RecoveryPolicy(classification: "official-closed"), log: { _ in }).run()
        XCTAssertFalse(m.requests.isEmpty, "recovery must be asked, or the test proves nothing")
        return m.requests.map { r in
            var r = r, step = r["step"] as? [String: Any] ?? [:], target = step["target"] as? [String: Any] ?? [:]
            for k in ["label", "label_matches", "dom_id"] { target[k] = nil } // the chrome rule's own fields
            step["target"] = target; r["step"] = step
            return json(r)
        }
    }

    let markers = ["MKSAY", "MKID", "MKPATH", "MKANCHOR", "MKCHECK", "MKTITLE", "MKGLOB"]

    func assertClean(_ sent: [String], file: StaticString = #filePath, line: UInt = #line) {
        for s in sent { for m in markers { XCTAssertFalse(s.contains(m), "\(m) reached the request: \(s.prefix(300))", file: file, line: line) } }
    }

    func testNothingFromTheTaskReachesARequestAfterAFindFailure() throws {
        let t = try seeded([["kind": "appears", "target": ["role": "AXStaticText", "label": "MKCHECK saved"]]])
        let form = provisioningForm(), web = form.children[0]
        let noField = UINode(role: form.role, label: form.label,
                             children: [UINode(role: web.role, label: web.label, children: web.children.filter { $0.domId != "full-name" })])
        assertClean(try requests(t, tree: noField))
    }

    func testNothingFromTheTaskReachesARequestAfterACheckFailure() throws {
        for check: [String: Any] in [["kind": "appears", "target": ["role": "AXStaticText", "label": "MKCHECK saved"]],
                                     ["kind": "window_title", "matches": "MKTITLE*"]] {
            let t = try seeded([check])
            let sent = try requests(t, tree: provisioningForm())
            assertClean(sent)
            XCTAssertTrue(sent.allSatisfy { $0.contains("\"divergence\":\"check_failed:") }, "divergence must be a fixed code: \(sent.first?.prefix(300) ?? "")")
        }
    }

    func testTheDivergenceCodeIsAlwaysFromTheClosedSet() {
        for r in ["element not found within 2 s", "check not met within 2 s: appears \"MKCHECK\" is not true yet.",
                  "check not met within 2 s: window title does not match \"MKTITLE\"", "recovery's answer was refused: MK",
                  "unexpected dialog: a dialog appeared", "MKANYTHING else", ""] {
            XCTAssertTrue(RecoveryRequest.divergenceCodes.contains(RecoveryRequest.divergenceCode(r)), r)
        }
    }
}
