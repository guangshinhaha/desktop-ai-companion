import CoreGraphics
import Foundation
import XCTest
@testable import ExecutorCore

// MARK: fakes

/// Every attribute anything asks of any element, with the element's role at that moment.
final class AccessLog {
    var requests: [(role: String, attribute: String)] = []
    func valueRequests(role: String) -> Int { requests.filter { $0.role == role && $0.attribute == "AXValue" }.count }
}

final class FakeEl: ElementReading {
    let attrs: [String: String]
    var kids: [FakeEl] = []
    weak var parent: FakeEl?
    let log: AccessLog
    let appPid: pid_t
    var titleUI: FakeEl?

    init(_ role: String, _ attrs: [String: String] = [:], subrole: String? = nil, label: String? = nil,
         domId: String? = nil, text: String? = nil, pid: pid_t = 100, log: AccessLog, kids: [FakeEl] = []) {
        var a = attrs
        a["AXRole"] = role
        if let subrole { a["AXSubrole"] = subrole }
        if let label { a["AXTitle"] = label }
        if let domId { a["AXDOMIdentifier"] = domId }
        if let text { a["text"] = text }
        self.attrs = a; self.log = log; self.appPid = pid; self.kids = kids
        for k in kids { k.parent = self }
    }

    var pid: pid_t { appPid }
    var role: String { attrs["AXRole"]! }

    func string(_ attribute: String) -> String? {
        log.requests.append((role, attribute))
        return attrs[attribute]
    }
    func staticText() -> String? {
        guard AXAttributePolicy.mayRead(attribute: "AXValue", role: role) else { return nil }
        log.requests.append((role, "AXValue"))
        return attrs["text"]
    }
    func element(_ attribute: String) -> ElementReading? {
        log.requests.append((role, attribute))
        if attribute == "AXParent" { return parent }
        if attribute == "AXTitleUIElement" { return titleUI }
        return nil
    }
    func children() -> [ElementReading] { log.requests.append((role, "AXChildren")); return kids }
    func same(as other: ElementReading) -> Bool { (other as? FakeEl) === self }
}

final class FakeSource: AccessibilitySource {
    var byPoint: [(CGPoint, FakeEl)] = []
    var focused: FakeEl?
    var bundles: [pid_t: String] = [100: "com.google.Chrome", 200: "com.other.App"]
    func element(at point: CGPoint) -> ElementReading? { byPoint.last(where: { $0.0 == point })?.1 }
    func focusedElement() -> ElementReading? { focused }
    func bundleId(forPid pid: pid_t) -> String? { bundles[pid] }
}

/// The stand-in provisioning form (stand-in/provisioning/page.ts) as Chrome exposes it.
struct Form {
    let log: AccessLog
    let name, email, school, role, password, create, other: FakeEl
    let root: FakeEl

    init(withPassword: Bool = false, button: String = "Create account") {
        let log = AccessLog()
        self.log = log
        func lab(_ t: String) -> FakeEl { FakeEl("AXStaticText", text: t, log: log) }
        name = FakeEl("AXTextField", label: "Full name", domId: "full-name", log: log)
        email = FakeEl("AXTextField", label: "Email", domId: "email", log: log)
        school = FakeEl("AXPopUpButton", label: "School", domId: "school", log: log)
        role = FakeEl("AXPopUpButton", label: "Role", domId: "role", log: log)
        password = FakeEl("AXTextField", subrole: "AXSecureTextField", label: "Password", domId: "password", log: log)
        let btnText = FakeEl("AXStaticText", text: button, log: log)
        create = FakeEl("AXButton", domId: "create", log: log, kids: [btnText])
        other = FakeEl("AXTextField", label: "Elsewhere", pid: 200, log: log)
        var kids = [lab("Full name"), name, lab("Email"), email, lab("School"), school, lab("Role"), role]
        if withPassword { kids += [lab("Password"), password] }
        kids.append(create)
        let web = FakeEl("AXWebArea", log: log, kids: [FakeEl("AXGroup", log: log, kids: kids)])
        root = FakeEl("AXWindow", log: log, kids: [web])
    }
}

func keyEvent(_ text: String) -> CGEvent {
    let e = CGEvent(keyboardEventSource: nil, virtualKey: 0, keyDown: true)!
    let chars = Array(text.utf16)
    e.keyboardSetUnicodeString(stringLength: chars.count, unicodeString: chars)
    return e
}

func keyCodeEvent(_ code: CGKeyCode, flags: CGEventFlags = []) -> CGEvent {
    let e = CGEvent(keyboardEventSource: nil, virtualKey: code, keyDown: true)!
    e.flags = flags
    return e
}

func click(at p: CGPoint) -> CGEvent {
    CGEvent(mouseEventSource: nil, mouseType: .leftMouseDown, mouseCursorPosition: p, mouseButton: .left)!
}

// MARK: tests

final class RecorderTests: XCTestCase {
    func session(_ f: Form, source: FakeSource = FakeSource()) -> (CaptureSession, FakeSource) {
        let b = TaskBuilder(appBundle: "com.google.Chrome", name: "Provision a staff account", author: "officer-id",
                            id: "tsk_test01", created: Date(timeIntervalSince1970: 1_800_000_000))
        return (CaptureSession(source: source, appBundle: "com.google.Chrome", builder: b), source)
    }

    func type(_ s: CaptureSession, _ src: FakeSource, into el: FakeEl, _ text: String) {
        src.focused = el
        for ch in text { s.handle(.keyDown, keyEvent(String(ch))) }
    }

    func press(_ s: CaptureSession, _ src: FakeSource, _ el: FakeEl, at p: CGPoint) {
        src.byPoint.append((p, el))
        s.handle(.leftMouseDown, click(at: p))
    }

    func json(_ s: CaptureSession) throws -> (text: String, obj: [String: Any]) {
        let d = try s.builder.json()
        return (String(decoding: d, as: UTF8.self), try JSONSerialization.jsonObject(with: d) as! [String: Any])
    }

    // Done when: every typed value appears as {input: name}; no value in the output.
    func testTypedValuesBecomeInputsAndNeverAppear() throws {
        let f = Form(); let (s, src) = session(f)
        let typedName = "Zorblax Quillfeather", typedEmail = "zq.synthetic@school.example.test"
        type(s, src, into: f.name, typedName)
        type(s, src, into: f.email, typedEmail)
        press(s, src, f.create, at: CGPoint(x: 5, y: 5))
        let (text, obj) = try json(s)
        for v in [typedName, typedEmail, "Zorblax", "Quillfeather", "quillfeather", "zq.synthetic", "school.example.test"] {
            XCTAssertFalse(text.localizedCaseInsensitiveContains(v), "output contains typed value \(v)")
        }
        // not even single characters of the typed strings as a value field
        let steps = obj["steps"] as! [[String: Any]]
        XCTAssertEqual(steps.count, 3)
        XCTAssertEqual((steps[0]["value"] as! [String: String]), ["input": "full_name"])
        XCTAssertEqual((steps[1]["value"] as! [String: String]), ["input": "email"])
        let inputs = obj["inputs"] as! [[String: String]]
        XCTAssertEqual(inputs.map { $0["name"] }, ["full_name", "email"])
        XCTAssertEqual(inputs.map { $0["type"] }, ["text", "email"])
    }

    // Done when: keystrokes into one field merge into one set_value step.
    func testKeystrokesIntoOneFieldMerge() throws {
        let f = Form(); let (s, src) = session(f)
        type(s, src, into: f.name, "abcdef")
        type(s, src, into: f.name, "ghi")
        XCTAssertEqual(s.builder.stepCount, 1)
        type(s, src, into: f.email, "x")
        type(s, src, into: f.name, "again")   // back to the first field after another one: a new step
        XCTAssertEqual(s.builder.stepCount, 3)
        let steps = try json(s).obj["steps"] as! [[String: Any]]
        XCTAssertEqual(steps.map { $0["action"] as! String }, ["set_value", "set_value", "set_value"])
        // the same field keeps the same input name
        XCTAssertEqual((steps[2]["value"] as! [String: String]), ["input": "full_name"])
        XCTAssertEqual((try json(s).obj["inputs"] as! [Any]).count, 2)
    }

    // Done when: consequential labels.
    func testConsequentialLabels() {
        for l in ["Create", "Submit", "Send", "Delete", "Approve", "Confirm", "Create account", "Yes, create", "SUBMIT now"] {
            XCTAssertTrue(TaskBuilder.isConsequentialLabel(l), l)
        }
        for l in ["Cancel", "Next", "Back", "Recreate", "Sender list", "Show more", ""] {
            XCTAssertFalse(TaskBuilder.isConsequentialLabel(l), l)
        }
    }

    func testPressConsequentialYesAndOtherUnknownFieldsNo() throws {
        let f = Form(); let (s, src) = session(f)
        let cancel = FakeEl("AXButton", label: "Cancel", log: f.log)
        type(s, src, into: f.name, "q")
        press(s, src, f.school, at: CGPoint(x: 1, y: 1))
        press(s, src, cancel, at: CGPoint(x: 2, y: 2))
        press(s, src, f.create, at: CGPoint(x: 3, y: 3))   // label read from the static text inside the button
        let steps = try json(s).obj["steps"] as! [[String: Any]]
        XCTAssertEqual(steps.map { $0["action"] as! String }, ["set_value", "select", "press", "press"])
        XCTAssertEqual(steps.map { $0["consequential"] as! String }, ["no", "no", "unknown", "yes"])
        XCTAssertNil(steps[0]["approval"])
        XCTAssertNotNil(steps[2]["approval"])
        XCTAssertEqual((steps[3]["target"] as! [String: Any])["label"] as? String, "Create account")
    }

    // Done when: password typing produces no value, no keystrokes; the step becomes a precondition.
    func testPasswordFieldYieldsPreconditionOnly() throws {
        let f = Form(withPassword: true); let (s, src) = session(f)
        let secret = "Hunter2-Synthetic!"
        type(s, src, into: f.password, secret)
        type(s, src, into: f.password, "more")
        // clicking into it is a step of nothing; the precondition is already there
        press(s, src, f.password, at: CGPoint(x: 9, y: 9))
        XCTAssertEqual(s.builder.stepCount, 0)
        XCTAssertEqual(s.builder.preconditionCount, 1)
        type(s, src, into: f.name, "n")
        let (text, obj) = try json(s)
        XCTAssertFalse(text.contains(secret))
        XCTAssertFalse(text.contains("Hunter"))
        let pre = obj["preconditions"] as! [[String: Any]]
        XCTAssertEqual(pre.count, 1)
        XCTAssertEqual((pre[0]["check"] as! [String: Any])["kind"] as? String, "exists")
        XCTAssertEqual(((pre[0]["check"] as! [String: Any])["target"] as! [String: Any])["label"] as? String, "Password")
        XCTAssertEqual((obj["steps"] as! [Any]).count, 1)   // only the Full name step
        XCTAssertFalse((obj["inputs"] as! [[String: String]]).contains { $0["name"] == "password" })
    }

    // The recorder must never read AXValue of a password field.
    func testPasswordFieldValueIsNeverRequested() {
        let f = Form(withPassword: true); let (s, src) = session(f)
        type(s, src, into: f.password, "secret")
        press(s, src, f.password, at: CGPoint(x: 9, y: 9))
        XCTAssertFalse(f.log.requests.isEmpty)
        XCTAssertEqual(f.log.valueRequests(role: "AXTextField"), 0)
        XCTAssertEqual(f.log.requests.filter { $0.attribute == "AXValue" }.count, 0)
        // and a secure element can never be read as static text, whatever the caller asks
        XCTAssertNil(f.password.staticText())
        XCTAssertFalse(AXAttributePolicy.mayRead(attribute: "AXValue", role: "AXTextField"))
        XCTAssertFalse(AXAttributePolicy.mayRead(attribute: "AXValue", role: "AXSecureTextField"))
        XCTAssertTrue(AXAttributePolicy.mayRead(attribute: "AXValue", role: "AXStaticText"))
    }

    func testPasswordRoleVariantsAreSecure() {
        XCTAssertTrue(Classifier.isSecure(role: "AXSecureTextField", subrole: nil))
        XCTAssertTrue(Classifier.isSecure(role: "AXTextField", subrole: "AXSecureTextField"))
        XCTAssertFalse(Classifier.isSecure(role: "AXTextField", subrole: nil))
    }

    func testOtherAppsAreDroppedBeforeAnyAttributeIsRead() {
        let f = Form(); let (s, src) = session(f)
        type(s, src, into: f.other, "private")
        press(s, src, f.other, at: CGPoint(x: 4, y: 4))
        XCTAssertEqual(s.builder.stepCount, 0)
        XCTAssertTrue(f.log.requests.filter { $0.role == "AXTextField" }.isEmpty, "read an attribute of another app's element")
    }

    func testNonTextKeysAndShortcutsAreNotTyping() {
        let f = Form(); let (s, src) = session(f)
        src.focused = f.name
        s.handle(.keyDown, keyCodeEvent(48))                          // tab
        s.handle(.keyDown, keyCodeEvent(0, flags: .maskCommand))      // cmd-a
        s.handle(.keyDown, keyCodeEvent(125))                         // arrow
        XCTAssertEqual(s.builder.stepCount, 0)
        s.handle(.keyDown, keyCodeEvent(36))                          // return
        XCTAssertEqual(s.uncapturedSubmitKeys, 1)
        XCTAssertEqual(s.builder.stepCount, 0)
    }

    func testClickOnPlainFieldOrBackgroundMakesNoStep() {
        let f = Form(); let (s, src) = session(f)
        press(s, src, f.name, at: CGPoint(x: 1, y: 1))
        press(s, src, f.root, at: CGPoint(x: 2, y: 2))
        XCTAssertEqual(s.builder.stepCount, 0)
    }

    func testDescriptorHasRoleLabelDomIdAndResolverPath() throws {
        let f = Form(); let (s, src) = session(f)
        type(s, src, into: f.email, "e")
        let step = (try json(s).obj["steps"] as! [[String: Any]])[0]
        let t = step["target"] as! [String: Any]
        XCTAssertEqual(t["role"] as? String, "AXTextField")
        XCTAssertEqual(t["label"] as? String, "Email")
        XCTAssertEqual(t["dom_id"] as? String, "email")
        XCTAssertEqual(t["path"] as? [String], ["AXWebArea", "AXTextField:2"])
        // the path locates the same element when the resolver reads a snapshot
        func snap(_ e: FakeEl) -> UINode { UINode(role: e.role, label: e.attrs["AXTitle"], children: e.kids.map(snap)) }
        let d = try Descriptor.load(from: JSONSerialization.data(withJSONObject: ["path": t["path"]!, "role": "AXTextField", "label": "Email"]))
        guard case .found(let m) = Resolver.resolve(d, in: snap(f.root)) else { return XCTFail("path did not resolve") }
        XCTAssertEqual(m.node.label, "Email")
    }

    func testOutputPassesTheSchema() throws {
        let f = Form(withPassword: true); let (s, src) = session(f)
        type(s, src, into: f.name, "abc")
        type(s, src, into: f.email, "d")
        press(s, src, f.school, at: CGPoint(x: 1, y: 1))
        type(s, src, into: f.school, "N")
        type(s, src, into: f.password, "pw")
        press(s, src, f.create, at: CGPoint(x: 3, y: 3))
        let dir = FileManager.default.temporaryDirectory.appendingPathComponent("rec-\(UUID().uuidString)")
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

    func testTapIsListenOnlyForClicksAndKeysOnly() {
        XCTAssertEqual(EventTap.mask,
                       (1 << CGEventType.leftMouseDown.rawValue) | (1 << CGEventType.keyDown.rawValue))
    }

    // MARK: found by the interrogation

    func testLabelsWithPersonalDataPatternsAreDropped() throws {
        let f = Form(); let (s, src) = session(f)
        let link = FakeEl("AXLink", label: "Tan Wei Ming S1234567A", domId: "row-S7654321B", log: f.log)
        let mail = FakeEl("AXButton", label: "Mail zq@school.example.test", log: f.log)
        let phone = FakeEl("AXTextField", label: "Call 91234567", log: f.log)
        press(s, src, link, at: CGPoint(x: 1, y: 1))
        press(s, src, mail, at: CGPoint(x: 2, y: 2))
        type(s, src, into: phone, "z")
        let text = try json(s).text
        for v in ["S1234567A", "S7654321B", "Tan Wei", "zq@school", "91234567"] {
            XCTAssertFalse(text.contains(v), v)
        }
    }

    func testBraceInLabelCannotBecomeAnInputReference() throws {
        let f = Form(); let (s, src) = session(f)
        press(s, src, FakeEl("AXButton", label: "Export {csv}", log: f.log), at: CGPoint(x: 1, y: 1))
        let step = (try json(s).obj["steps"] as! [[String: Any]])[0]
        XCTAssertFalse((step["approval"] as! [String: Any])["summary"] as! String == "" )
        XCTAssertFalse(((step["approval"] as! [String: Any])["summary"] as! String).contains("{csv}"))
    }

    func testFieldNamedPasswordWithoutSecureSubroleIsStillAPrecondition() throws {
        let f = Form(); let (s, src) = session(f)
        let pw = FakeEl("AXTextField", label: "Password", domId: "password", log: f.log)
        type(s, src, into: pw, "hunter2")
        let byId = FakeEl("AXTextField", domId: "user-pwd", log: f.log)
        type(s, src, into: byId, "x")
        XCTAssertEqual(s.builder.stepCount, 0)
        XCTAssertEqual(s.builder.preconditionCount, 2)
        XCTAssertFalse(try json(s).text.contains("hunter2"))
    }

    func testSecureFieldNeverReadsPlaceholderOrPathOrValue() throws {
        let f = Form(); let (s, src) = session(f)
        let pw = FakeEl("AXTextField", ["AXPlaceholderValue": "Your NRIC is the default"], subrole: "AXSecureTextField",
                        domId: "pw", log: f.log)
        let g = FakeEl("AXGroup", log: f.log, kids: [pw]); _ = g
        type(s, src, into: pw, "z")
        let asked = Set(f.log.requests.filter { $0.role == "AXTextField" }.map { $0.attribute })
        XCTAssertFalse(asked.contains("AXPlaceholderValue"))
        XCTAssertFalse(asked.contains("AXValue"))
        XCTAssertFalse(asked.contains("AXChildren"))
        XCTAssertFalse(try json(s).text.contains("NRIC"))
    }

    func testDoubleClickIsOnePress() {
        let f = Form(); let (s, src) = session(f)
        src.byPoint.append((CGPoint(x: 3, y: 3), f.create))
        let c1 = click(at: CGPoint(x: 3, y: 3)); c1.setIntegerValueField(.mouseEventClickState, value: 1)
        let c2 = click(at: CGPoint(x: 3, y: 3)); c2.setIntegerValueField(.mouseEventClickState, value: 2)
        s.handle(.leftMouseDown, c1); s.handle(.leftMouseDown, c2)
        XCTAssertEqual(s.builder.stepCount, 1)
    }

    func testSpaceOrReturnOnAFocusedButtonIsAPress() throws {
        let f = Form(); let (s, src) = session(f)
        src.focused = f.create
        s.handle(.keyDown, keyCodeEvent(49))
        XCTAssertEqual(s.builder.stepCount, 1)
        s.handle(.keyDown, keyCodeEvent(0))   // a letter on a button does nothing
        XCTAssertEqual(s.builder.stepCount, 1)
        XCTAssertEqual((try json(s).obj["steps"] as! [[String: Any]])[0]["consequential"] as? String, "yes")
    }

    func testPasteIntoAFieldIsAStepAndFunctionKeysAreNot() throws {
        let f = Form(); let (s, src) = session(f)
        src.focused = f.name
        s.handle(.keyDown, keyCodeEvent(122))   // F1
        XCTAssertEqual(s.builder.stepCount, 0)
        s.handle(.keyDown, keyCodeEvent(9, flags: .maskCommand))   // cmd-v
        XCTAssertEqual(s.builder.stepCount, 1)
        XCTAssertEqual(((try json(s).obj["steps"] as! [[String: Any]])[0]["value"] as! [String: String]), ["input": "full_name"])
    }

    func testEmptyNameAndAuthorFallBack() throws {
        let b = TaskBuilder(appBundle: "com.google.Chrome", name: " ", author: "")
        XCTAssertEqual(b.name, "Recorded task")
        XCTAssertEqual(b.author, "officer")
    }

    func testOutputFileIsOwnerOnlyAndNeverOverwrites() throws {
        let dir = FileManager.default.temporaryDirectory.appendingPathComponent("rec-\(UUID().uuidString)")
        try FileManager.default.createDirectory(at: dir, withIntermediateDirectories: true)
        defer { try? FileManager.default.removeItem(at: dir) }
        let path = dir.appendingPathComponent("t.json").path
        try Recorder.writePrivate(Data("{}".utf8), to: path)
        let mode = (try FileManager.default.attributesOfItem(atPath: path)[.posixPermissions] as! NSNumber).intValue
        XCTAssertEqual(mode & 0o077, 0, "group/other can read the recording")
        XCTAssertThrowsError(try Recorder.writePrivate(Data("x".utf8), to: path))
        XCTAssertEqual(try String(contentsOfFile: path, encoding: .utf8), "{}")
    }
}
