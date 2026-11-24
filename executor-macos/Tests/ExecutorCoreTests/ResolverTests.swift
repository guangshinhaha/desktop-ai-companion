import XCTest
@testable import ExecutorCore

/// The stand-in provisioning form as Chrome exposes it, built by hand so no app has to run.
private func form(button: String = "Create account", extraButton: UINode? = nil) -> UINode {
    func field(_ id: String, _ label: String, role: String = "AXTextField") -> [UINode] {
        [UINode(role: "AXStaticText", label: label), UINode(role: role, label: label, domId: id)]
    }
    var kids: [UINode] = [UINode(role: "AXHeading", label: "New staff account")]
    kids += field("full-name", "Full name")
    kids += field("email", "Email")
    kids += field("school", "School", role: "AXPopUpButton")
    kids += field("role", "Role", role: "AXPopUpButton")
    kids.append(UINode(role: "AXButton", label: button, domId: "create"))
    if let extraButton { kids.append(extraButton) }
    let web = UINode(role: "AXWebArea", label: "New staff account",
                     children: [UINode(role: "AXGroup", label: "form", children: kids)])
    return UINode(role: "AXWindow", label: "New staff account", children: [web])
}

private func expectFound(_ r: Resolution, _ locator: Locator, label: String? = nil,
                         file: StaticString = #filePath, line: UInt = #line) {
    guard case .found(let m) = r else { return XCTFail("expected found, got \(r)", file: file, line: line) }
    XCTAssertEqual(m.locator, locator, file: file, line: line)
    if let label { XCTAssertEqual(m.node.label, label, file: file, line: line) }
}

final class ResolverTests: XCTestCase {
    let fields: [(String, String, String)] = [
        ("AXTextField", "Full name", "full-name"), ("AXTextField", "Email", "email"),
        ("AXPopUpButton", "School", "school"), ("AXPopUpButton", "Role", "role"),
        ("AXButton", "Create account", "create"),
    ]

    func testEveryFieldAndButtonResolveByDomId() {
        for (role, label, id) in fields {
            let d = Descriptor(role: role, label: label, domId: id)
            expectFound(Resolver.resolve(d, in: form()), .domId, label: label)
        }
    }

    func testWithoutDomIdTheyResolveByRoleAndLabel() {
        for (role, label, _) in fields {
            let d = Descriptor(role: role, label: label)
            expectFound(Resolver.resolve(d, in: form()), .roleExactLabel, label: label)
        }
    }

    func testRenamedButtonResolvesByFuzzyLabelAndAnchorsAsLocator3() {
        let d = Descriptor(role: "AXButton", label: "Create account", anchors: ["Role"])
        expectFound(Resolver.resolve(d, in: form(button: "Create")), .roleFuzzyLabelAnchors, label: "Create")
    }

    func testRenamedButtonWithAnchorMissingFromScreenIsNotFound() {
        let d = Descriptor(role: "AXButton", label: "Create account", anchors: ["Nowhere to be seen"])
        guard case .notFound = Resolver.resolve(d, in: form(button: "Create")) else { return XCTFail("expected notFound") }
    }

    func testFuzzyLabelBelowThresholdIsNotAMatch() {
        let d = Descriptor(role: "AXButton", label: "Create account")
        guard case .notFound = Resolver.resolve(d, in: form(button: "Cancel")) else { return XCTFail("expected notFound") }
    }

    func testLabelMatchesGlob() {
        let page = UINode(role: "AXWindow", children: [UINode(role: "AXStaticText", label: "Account created")])
        let d = Descriptor(role: "AXStaticText", labelMatches: "Account created*")
        expectFound(Resolver.resolve(d, in: page), .roleFuzzyLabelAnchors, label: "Account created")
    }

    func testTwoIdenticalButtonsAreAmbiguousNeverAPick() {
        let twin = UINode(role: "AXButton", label: "Create account")
        let page = form(extraButton: twin)
        // Without dom_id both buttons match equally well.
        let d = Descriptor(role: "AXButton", label: "Create account")
        guard case .ambiguous(let a) = Resolver.resolve(d, in: page) else { return XCTFail("expected ambiguous") }
        XCTAssertEqual(a.candidates.count, 2)
        XCTAssertEqual(a.locator, .roleExactLabel)
    }

    func testTwoIdenticalDomIdsAreAmbiguous() {
        let page = UINode(role: "AXWindow", children: [
            UINode(role: "AXButton", label: "Go", domId: "go"), UINode(role: "AXButton", label: "Go", domId: "go"),
        ])
        guard case .ambiguous(let a) = Resolver.resolve(Descriptor(domId: "go"), in: page) else { return XCTFail("expected ambiguous") }
        XCTAssertEqual(a.locator, .domId)
    }

    func testTwoEquallyFuzzyMatchesWithEqualAnchorsAreAmbiguous() {
        let page = UINode(role: "AXWindow", children: [
            UINode(role: "AXStaticText", label: "Role"),
            UINode(role: "AXButton", label: "Create"), UINode(role: "AXButton", label: "Create"),
        ])
        let d = Descriptor(role: "AXButton", label: "Create account", anchors: ["Role"])
        guard case .ambiguous(let a) = Resolver.resolve(d, in: page) else { return XCTFail("expected ambiguous") }
        XCTAssertEqual(a.locator, .roleFuzzyLabelAnchors)
    }

    func testAnchorsBreakATieOnlyWhenOneCandidateIsClearlyNearest() {
        let a = UINode(role: "AXButton", label: "Create")
        let b = UINode(role: "AXButton", label: "Create")
        let page = UINode(role: "AXWindow", children: [
            UINode(role: "AXGroup", children: [UINode(role: "AXStaticText", label: "Billing"), a]),
            UINode(role: "AXGroup", children: [UINode(role: "AXStaticText", label: "Staff"), b]),
        ])
        let d = Descriptor(role: "AXButton", label: "Create account", anchors: ["Staff"])
        guard case .found(let m) = Resolver.resolve(d, in: page) else { return XCTFail("expected found") }
        XCTAssertTrue(m.node === b)
    }

    func testPathLocator() {
        let d = Descriptor(role: "AXTextField", path: ["AXWebArea", "AXGroup:form", "AXTextField:2"])
        expectFound(Resolver.resolve(d, in: form()), .path, label: "Email")
    }

    func testPathWithTwoNearestSegmentsIsAmbiguous() {
        let d = Descriptor(path: ["AXWebArea", "AXGroup", "AXButton"])
        let page = form(extraButton: UINode(role: "AXButton", label: "Other"))
        guard case .ambiguous(let a) = Resolver.resolve(d, in: page) else { return XCTFail("expected ambiguous") }
        XCTAssertEqual(a.locator, .path)
    }

    func testLocatorsRunInOrder() {
        // dom_id points at the button, the label at a field: dom_id (1) wins.
        let d = Descriptor(role: "AXTextField", label: "Email", domId: "create")
        expectFound(Resolver.resolve(d, in: form()), .domId, label: "Create account")
    }

    func testFuzzyHelpers() {
        XCTAssertEqual(Resolver.similarity("Create account", "create  Account!"), 1)
        XCTAssertEqual(Resolver.similarity("Create account", "Create"), 0.9)
        XCTAssertLessThan(Resolver.similarity("Create account", "Cancel"), Resolver.fuzzyThreshold)
        XCTAssertTrue(Resolver.glob("Account created*", "account created for Bella"))
        XCTAssertFalse(Resolver.glob("Account created*", "Cannot create account"))
    }

    // MARK: waiting

    final class Clock { var t = 0.0 }

    func testNotFoundOnlyAfterTheCeilingAndItWaitsOnEventsNotSleeps() {
        let clock = Clock()
        var waits: [TimeInterval] = []
        var snapshots = 0
        let r = Resolver.resolve(
            Descriptor(role: "AXButton", label: "Nope"), ceiling: 5,
            snapshot: { snapshots += 1; return form() },
            wait: { waits.append($0); clock.t += $0 },  // the fake wait "times out"
            now: { clock.t })
        guard case .notFound = r else { return XCTFail("expected notFound") }
        XCTAssertEqual(clock.t, 5, accuracy: 1e-9, "returns exactly at the ceiling, not before")
        XCTAssertGreaterThan(snapshots, 3, "re-checks after each wait")
        XCTAssertEqual(waits.first, 0.1)
        XCTAssertLessThanOrEqual(waits.max()!, 2.0, "backoff stops at 2 s")
        XCTAssertEqual(waits, waits.sorted(), "backoff never shortens")
    }

    func testFoundAtOnceDoesNotWait() {
        var waited = false
        let r = Resolver.resolve(Descriptor(domId: "create"), ceiling: 30, snapshot: { form() },
                                 wait: { _ in waited = true }, now: { 0 })
        expectFound(r, .domId)
        XCTAssertFalse(waited)
    }

    func testFoundWhenTheElementAppearsBeforeTheCeiling() {
        let clock = Clock()
        let r = Resolver.resolve(
            Descriptor(role: "AXButton", label: "Create account"), ceiling: 30,
            snapshot: { clock.t >= 0.3 ? form() : UINode(role: "AXWindow") },
            wait: { clock.t += $0 }, now: { clock.t })
        expectFound(r, .roleExactLabel)
        XCTAssertLessThan(clock.t, 1)
    }

    func testNoWindowYetKeepsWaitingThenNotFound() {
        let clock = Clock()
        let r = Resolver.resolve(Descriptor(domId: "create"), ceiling: 1, snapshot: { nil },
                                 wait: { clock.t += $0 }, now: { clock.t })
        guard case .notFound = r else { return XCTFail("expected notFound") }
    }

    func testAmbiguousReturnsWithoutWaiting() {
        var waited = false
        let page = UINode(role: "AXWindow", children: [UINode(role: "AXButton", label: "Go"), UINode(role: "AXButton", label: "Go")])
        let r = Resolver.resolve(Descriptor(role: "AXButton", label: "Go"), ceiling: 30, snapshot: { page },
                                 wait: { _ in waited = true }, now: { 0 })
        guard case .ambiguous = r else { return XCTFail("expected ambiguous") }
        XCTAssertFalse(waited)
    }
}
