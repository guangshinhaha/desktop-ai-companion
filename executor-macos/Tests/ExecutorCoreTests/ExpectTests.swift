import CoreGraphics
import Foundation
import XCTest
@testable import ExecutorCore

// The stand-in provisioning app as Chrome exposes it, before and after Create account.
// Synthetic data only.
enum Pages {
    static let typedName = "Zorblax Quillfeather"
    static let typedEmail = "zq.synthetic@school.example.test"

    static func node(_ role: String, _ label: String? = nil, _ kids: [UINode] = []) -> UINode {
        UINode(role: role, label: label, children: kids)
    }
    static func text(_ s: String) -> UINode { node("AXStaticText", s) }
    static func head(_ s: String) -> UINode { node("AXHeading", s, [text(s)]) }

    static func chrome(_ title: String, page: [UINode]) -> UINode {
        node("AXWindow", title, [
            node("AXToolbar", nil, [node("AXTextField", "Address and search bar")]),
            node("AXWebArea", title, page),
        ])
    }

    static func form(title: String = "Create a staff account") -> UINode {
        chrome(title, page: [
            node("AXHeading", "Create a staff account", [text("Create a staff account")]),
            text("Full name"), node("AXTextField", "Full name"),
            text("Email"), node("AXTextField", "Email"),
            node("AXButton", "Create account"),
        ])
    }

    /// The confirmation page echoes the other fields' values in a paragraph.
    static func created(echo: Bool = true) -> UINode {
        var page = [node("AXHeading", "Account created", [text("Account created")])]
        if echo { page.append(text("\(typedName) (\(typedEmail)), Teacher at Northgate Primary.")) }
        page.append(node("AXLink", "Create another account"))
        return chrome("Account created", page: page)
    }

    /// Text inside a text field is its value. It must never be a check, even if a page exposes it as text.
    static func formWithFieldEcho() -> UINode {
        chrome("Create a staff account", page: [
            node("AXTextField", "Full name", [text(typedName)]),
            node("AXTextArea", "Notes", [text("Echoed notes text")]),
            node("AXButton", "Create account"),
        ])
    }
}

final class FakeSnapshots: WindowSnapshotting {
    var queue: [UINode?]
    var reads = 0
    init(_ q: [UINode?]) { queue = q }
    func snapshot() -> UINode? { reads += 1; return queue.isEmpty ? nil : queue.removeFirst() }
}

final class ExpectTests: XCTestCase {
    typealias P = ExpectProposer.Proposal

    // MARK: the proposer

    func testAccountCreatedAppearsAsStaticText() {
        let p = ExpectProposer.propose(before: Pages.form(), after: Pages.created())
        XCTAssertEqual(p, P(kind: .appears, role: "AXStaticText", label: "Account created"))
    }

    func testEchoedValuesAreNeverProposed() {
        // The paragraph that repeats the record is not a candidate, with or without the heading beside it.
        let after = Pages.chrome("Done", page: [Pages.text("\(Pages.typedName) (\(Pages.typedEmail)), Teacher at Northgate Primary.")])
        XCTAssertNil(ExpectProposer.propose(before: Pages.chrome("Done", page: []), after: after))
        let p = ExpectProposer.propose(before: Pages.form(), after: Pages.created())!
        XCTAssertFalse(String(describing: p).contains("Zorblax"))
        XCTAssertFalse(String(describing: p).contains("@"))
    }

    func testHeadingBeatsPlainTextAndDialogBeatsPage() {
        let before = Pages.chrome("x", page: [Pages.text("Intro")])
        let after = Pages.chrome("x", page: [
            Pages.text("Intro"), Pages.text("Some note"),
            Pages.node("AXDialog", nil, [Pages.node("AXHeading", "Are you sure", [Pages.text("Are you sure")])]),
        ])
        XCTAssertEqual(ExpectProposer.propose(before: before, after: after),
                       P(kind: .appears, role: "AXStaticText", label: "Are you sure"))
    }

    func testDisappearsWhenNothingAppears() {
        let before = Pages.chrome("x", page: [Pages.head("Saving"), Pages.head("Name")])
        let after = Pages.chrome("x", page: [Pages.head("Name")])
        XCTAssertEqual(ExpectProposer.propose(before: before, after: after), P(kind: .disappears, role: "AXStaticText", label: "Saving"))
    }

    func testWindowTitleWhenOnlyTheTitleChanged() {
        let before = Pages.chrome("Inbox", page: [Pages.text("Same")])
        let after = Pages.chrome("Outbox", page: [Pages.text("Same")])
        XCTAssertEqual(ExpectProposer.propose(before: before, after: after), P(kind: .windowTitle, role: nil, label: "Outbox"))
        XCTAssertEqual(P(kind: .windowTitle, role: nil, label: "Outbox").json()["matches"] as? String, "Outbox")
    }

    func testPlainParagraphsAreNeverChecksOnlyHeadingsAndDialogs() {
        let before = Pages.chrome("x", page: [Pages.head("Form")])
        let after = Pages.chrome("x", page: [Pages.head("Form"), Pages.text("Welcome Zorblax"), Pages.text("Saved")])
        XCTAssertNil(ExpectProposer.propose(before: before, after: after))
        let inDialog = Pages.chrome("x", page: [Pages.head("Form"), Pages.node("AXDialog", nil, [Pages.text("Are you sure")])])
        XCTAssertEqual(ExpectProposer.propose(before: before, after: inDialog)?.label, "Are you sure")
    }

    // Found by the interrogation: look-alike characters, names in headings, titles and removed text.
    func testLookAlikeCharactersAndNamesAreNotStable() {
        let before = Pages.chrome("Start", page: [Pages.head("Form")])
        for label in ["\u{2460}\u{2461}\u{2462}", "S\u{00B9}\u{00B2}\u{00B3}\u{2074}\u{2075}\u{2076}\u{2077}D",
                      "zq\u{FF20}school.example.test", "Welcome Zorblax Quillfeather", "Welcome Zorblax", "Zorblax Quillfeather"] {
            let after = Pages.chrome("Start", page: [Pages.head("Form"), Pages.head(label)])
            XCTAssertNil(ExpectProposer.propose(before: before, after: after), label)
            // removed text
            XCTAssertNil(ExpectProposer.propose(before: after, after: before), label)
        }
        for title in ["Profile Zorblax Quillfeather", "Accounts \u{2013} Zorblax Quillfeather", "Inbox 12"] {
            let after = Pages.chrome(title, page: [Pages.head("Form")])
            XCTAssertNil(ExpectProposer.propose(before: before, after: after), title)
        }
        XCTAssertEqual(ExpectProposer.propose(before: before, after: Pages.chrome("Start", page: [Pages.head("Form"), Pages.head("Account created")]))?.label,
                       "Account created")
    }

    func testTextUnderGridsAndEditorsIsNotACheck() {
        let before = Pages.chrome("x", page: [Pages.head("Form")])
        let grid = Pages.node("AXGrid", nil, [Pages.node("AXRow", nil, [Pages.node("AXCell", nil, [Pages.head("Done here")])])])
        XCTAssertNil(ExpectProposer.propose(before: before, after: Pages.chrome("x", page: [Pages.head("Form"), grid])))
    }

    func testDuplicateLabelsNeverMakeACheckThatIsAlreadyTrueOrNeverWillBe() {
        let one = Pages.chrome("x", page: [Pages.head("Done")])
        let two = Pages.chrome("x", page: [Pages.head("Done"), Pages.head("Done")])
        XCTAssertNil(ExpectProposer.propose(before: one, after: two))
        XCTAssertNil(ExpectProposer.propose(before: two, after: one))
    }

    func testAFailedReadGivesTheNextPressNoCheckOfItsOwnRatherThanAnOldChange() throws {
        func page(_ extra: [String]) -> UINode { Pages.chrome("x", page: [Pages.head("Form")] + extra.map { Pages.head($0) }) }
        let f = Form()
        let next = FakeEl("AXButton", label: "Next", log: f.log), save = FakeEl("AXButton", label: "Save", log: f.log)
        let src = FakeSource()
        // reads: start, Next begins, Save begins (fails), stop
        let fake = FakeSnapshots([page([]), page([]), nil, page(["Saved"])])
        let s = CaptureSession(source: src, appBundle: "com.google.Chrome",
                               builder: TaskBuilder(appBundle: "com.google.Chrome", id: "tsk_test01"), snapshotter: fake)
        s.start()
        src.byPoint.append((CGPoint(x: 1, y: 1), next)); src.byPoint.append((CGPoint(x: 2, y: 2), save))
        s.handle(.leftMouseDown, click(at: CGPoint(x: 1, y: 1)))
        s.handle(.leftMouseDown, click(at: CGPoint(x: 2, y: 2)))
        s.settle()
        let obj = try JSONSerialization.jsonObject(with: s.builder.json()) as! [String: Any]
        let kinds = (obj["steps"] as! [[String: Any]]).map { (($0["expect"] as! [[String: Any]])[0]["kind"] as! String) }
        XCTAssertEqual(kinds, ["exists", "exists"], "a read that failed must not hand its change to the next step")
    }

    func testNothingChangedProposesNothing() {
        XCTAssertNil(ExpectProposer.propose(before: Pages.form(), after: Pages.form()))
    }

    func testChangingDataIsNotStable() {
        let before = Pages.chrome("x", page: [Pages.text("Clock")])
        for unstable in ["Saved at 10:42", "Hello zq@school.example.test", "Row 12", "Aiden Tan (Teacher)", "Aiden, Teacher",
                         "A long sentence that echoes a record back to the person who just filled it in"] {
            let after = Pages.chrome("x", page: [Pages.text("Clock"), Pages.head(unstable)])
            XCTAssertNil(ExpectProposer.propose(before: before, after: after), unstable)
        }
    }

    func testTextInsideAnInputIsNeverACheck() {
        XCTAssertNil(ExpectProposer.propose(before: Pages.form(), after: Pages.formWithFieldEcho()).flatMap {
            $0.label.contains("Zorblax") || $0.label.contains("Echoed") ? $0 : nil
        })
        let p = ExpectProposer.propose(before: Pages.chrome("Create a staff account", page: []), after: Pages.formWithFieldEcho())
        XCTAssertNil(p)
    }

    func testBrowserChromeOutsideThePageIsIgnoredWhenThereIsAPage() {
        let before = Pages.node("AXWindow", "w", [Pages.head("Toolbar old"), Pages.node("AXWebArea", "w", [Pages.head("Page")])])
        let after = Pages.node("AXWindow", "w", [Pages.head("Toolbar new"), Pages.node("AXWebArea", "w", [Pages.head("Page")])])
        XCTAssertNil(ExpectProposer.propose(before: before, after: after))
    }

    // MARK: through the session

    func recordProvisioning() throws -> (CaptureSession, text: String, steps: [[String: Any]]) {
        let f = Form()
        // start, then one read as each of the 3 steps begins, then one at stop.
        let fake = FakeSnapshots([Pages.form(), Pages.form(), Pages.form(), Pages.form(), Pages.created()])
        let src = FakeSource()
        let b = TaskBuilder(appBundle: "com.google.Chrome", name: "Provision a staff account", author: "officer-id",
                            id: "tsk_test01", created: Date(timeIntervalSince1970: 1_800_000_000))
        let s = CaptureSession(source: src, appBundle: "com.google.Chrome", builder: b, snapshotter: fake)
        s.start()
        for (el, text) in [(f.name, Pages.typedName), (f.email, Pages.typedEmail)] {
            src.focused = el
            for ch in text { s.handle(.keyDown, keyEvent(String(ch))) }
        }
        src.byPoint.append((CGPoint(x: 5, y: 5), f.create))
        s.handle(.leftMouseDown, click(at: CGPoint(x: 5, y: 5)))
        s.settle()   // stop
        let d = try s.builder.json()
        let obj = try JSONSerialization.jsonObject(with: d) as! [String: Any]
        return (s, String(decoding: d, as: UTF8.self), obj["steps"] as! [[String: Any]])
    }

    func testCreateAccountCheckIsAccountCreatedAppears() throws {
        let (_, _, steps) = try recordProvisioning()
        XCTAssertEqual(steps.count, 3)
        let expect = steps[2]["expect"] as! [[String: Any]]
        XCTAssertEqual(expect.count, 1)
        XCTAssertEqual(expect[0]["kind"] as? String, "appears")
        XCTAssertEqual(expect[0]["target"] as? [String: String], ["role": "AXStaticText", "label": "Account created"])
    }

    func testFieldStepsCheckTheirOwnInput() throws {
        let (_, _, steps) = try recordProvisioning()
        for (i, n) in ["full_name", "email"].enumerated() {
            let e = steps[i]["expect"] as! [[String: Any]]
            XCTAssertEqual(e.count, 1)
            XCTAssertEqual(e[0]["kind"] as? String, "value_equals")
            XCTAssertEqual(e[0]["input"] as? String, n)
            XCTAssertNil(e[0]["constant"])
            XCTAssertNil(e[0]["matches"])
        }
    }

    // Done when: a test searches the checks for the other fields' synthetic values and finds none.
    func testNoChecksHoldAnyFieldsSyntheticValue() throws {
        let (_, text, steps) = try recordProvisioning()
        let checks = String(decoding: try JSONSerialization.data(withJSONObject: steps.map { $0["expect"]! }), as: UTF8.self)
        for v in [Pages.typedName, Pages.typedEmail, "Zorblax", "Quillfeather", "zq.synthetic", "school.example.test",
                  "Teacher", "Northgate"] {
            XCTAssertFalse(checks.localizedCaseInsensitiveContains(v), "a check holds \(v)")
            XCTAssertFalse(text.localizedCaseInsensitiveContains(v), "the file holds \(v)")
        }
    }

    func testPlacementOfSnapshotsSettlesBeforeTheNextStepBegins() throws {
        let f = Form()
        let fake = FakeSnapshots([Pages.form(), Pages.form(), Pages.created()])
        let src = FakeSource()
        let b = TaskBuilder(appBundle: "com.google.Chrome", id: "tsk_test01")
        let s = CaptureSession(source: src, appBundle: "com.google.Chrome", builder: b, snapshotter: fake)
        s.start()
        XCTAssertEqual(fake.reads, 1)
        src.byPoint.append((CGPoint(x: 5, y: 5), f.create))
        s.handle(.leftMouseDown, click(at: CGPoint(x: 5, y: 5)))
        XCTAssertEqual(fake.reads, 2, "the first step's own start reads the screen as its 'before'")
        s.handle(.leftMouseDown, click(at: CGPoint(x: 5, y: 5)))   // a second click is a new step; the first settles
        XCTAssertEqual(fake.reads, 3)
        XCTAssertEqual(s.timings.map { $0.step }, ["s1"])
        XCTAssertEqual(s.timings[0].kind, "appears")
    }

    func testPressWithNoChangeKeepsTheExistsCheck() throws {
        let f = Form()
        let src = FakeSource()
        let fake = FakeSnapshots([Pages.form(), Pages.form(), Pages.form()])
        let s = CaptureSession(source: src, appBundle: "com.google.Chrome",
                               builder: TaskBuilder(appBundle: "com.google.Chrome", id: "tsk_test01"), snapshotter: fake)
        s.start()
        src.byPoint.append((CGPoint(x: 5, y: 5), f.create))
        s.handle(.leftMouseDown, click(at: CGPoint(x: 5, y: 5)))
        s.settle()
        let steps = try JSONSerialization.jsonObject(with: s.builder.json()) as! [String: Any]
        let e = ((steps["steps"] as! [[String: Any]])[0]["expect"] as! [[String: Any]])[0]
        XCTAssertEqual(e["kind"] as? String, "exists")
    }

    func testWithoutASnapshotterNothingChanges() throws {
        let f = Form()
        let src = FakeSource()
        let s = CaptureSession(source: src, appBundle: "com.google.Chrome", builder: TaskBuilder(appBundle: "com.google.Chrome", id: "tsk_test01"))
        src.byPoint.append((CGPoint(x: 5, y: 5), f.create))
        s.handle(.leftMouseDown, click(at: CGPoint(x: 5, y: 5)))
        s.settle()
        XCTAssertTrue(s.timings.isEmpty)
        XCTAssertEqual(s.builder.stepCount, 1)
    }

    func testPasswordFieldIsNeverReadByTheCheckPath() throws {
        let f = Form(withPassword: true)
        let src = FakeSource()
        let fake = FakeSnapshots([Pages.form(), Pages.created()])
        let s = CaptureSession(source: src, appBundle: "com.google.Chrome",
                               builder: TaskBuilder(appBundle: "com.google.Chrome", id: "tsk_test01"), snapshotter: fake)
        s.start()
        src.focused = f.password
        s.handle(.keyDown, keyEvent("s"))
        src.byPoint.append((CGPoint(x: 5, y: 5), f.create))
        s.handle(.leftMouseDown, click(at: CGPoint(x: 5, y: 5)))
        s.settle()
        XCTAssertEqual(s.builder.stepCount, 1)
        XCTAssertEqual(f.log.requests.filter { $0.attribute == "AXValue" && $0.role != "AXStaticText" }.count, 0)
        XCTAssertEqual(f.log.valueRequests(role: "AXSecureTextField"), 0)
        XCTAssertEqual(f.log.valueRequests(role: "AXTextField"), 0)
    }

    // Done when: under 300 ms per step. The compare alone, on a page of ten thousand nodes.
    func testCompareIsFarUnderTheBudgetOnALargePage() {
        func big(_ extra: Bool) -> UINode {
            var rows: [UINode] = []
            for i in 0..<5000 { rows.append(Pages.node("AXGroup", "row", [Pages.text("Label \(i % 40) word")])) }
            if extra { rows.append(Pages.node("AXHeading", "Account created", [Pages.text("Account created")])) }
            return Pages.chrome("big", page: rows)
        }
        let a = big(false), b = big(true)
        let t0 = ProcessInfo.processInfo.systemUptime
        let p = ExpectProposer.propose(before: a, after: b)
        let ms = (ProcessInfo.processInfo.systemUptime - t0) * 1000
        XCTAssertEqual(p?.label, "Account created")
        XCTAssertLessThan(ms, 300, "compare took \(ms) ms")
        print("compare on 10,000 nodes: \(ms) ms")
    }

    func testRecordedTaskWithProposedChecksPassesTheSchema() throws {
        let (s, _, _) = try recordProvisioning()
        let dir = FileManager.default.temporaryDirectory.appendingPathComponent("exp-\(UUID().uuidString)")
        try FileManager.default.createDirectory(at: dir, withIntermediateDirectories: true)
        defer { try? FileManager.default.removeItem(at: dir) }
        let file = dir.appendingPathComponent("task.json")
        try s.builder.json().write(to: file)
        var root = URL(fileURLWithPath: #filePath)
        for _ in 0..<4 { root.deleteLastPathComponent() }
        let script = root.appendingPathComponent("scripts/validate-task").path
        guard FileManager.default.isExecutableFile(atPath: script) else { throw XCTSkip("scripts/validate-task not found") }
        let p = Process()
        p.executableURL = URL(fileURLWithPath: "/usr/bin/env")
        p.arguments = ["bash", script, file.path]
        let err = Pipe(); p.standardError = err; p.standardOutput = Pipe()
        do { try p.run() } catch { throw XCTSkip("cannot run validate-task: \(error)") }
        p.waitUntilExit()
        let msg = String(decoding: err.fileHandleForReading.readDataToEndOfFile(), as: UTF8.self)
        if p.terminationStatus == 127 || msg.contains("node: command not found") { throw XCTSkip("node not available") }
        XCTAssertEqual(p.terminationStatus, 0, msg)
    }
}
