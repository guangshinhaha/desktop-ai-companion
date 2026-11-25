import XCTest
@testable import ExecutorCore

/// Adversary round 3 at 712f66b: recovery bounds. Synthetic data only.
final class RecoveryRound3BoundsTests: XCTestCase {
    final class Picking: RecoverySource {
        var plan: [(String?, [String: Any])]; var requests: [[String: Any]] = []
        var onAsk: (() -> Void)?
        init(_ p: [(String?, [String: Any])]) { plan = p }
        func ask(_ request: [String: Any]) -> [String: Any]? {
            requests.append(request); onAsk?()
            guard !plan.isEmpty else { return nil }
            let p = plan.removeFirst(); var a = p.1
            if let l = p.0 { a["ref"] = ref(in: request, label: l) ?? "n-none" }
            return a
        }
    }
    let press: [String: Any] = ["answer": "act", "action": "press"]

    func run(_ h: FakeHost, _ t: ReplayTask, _ m: Picking, control: RunControl? = nil) throws -> (ReplayReport, RecordingAudit) {
        let a = RecordingAudit(); a.host = h
        let r = Replayer(task: t, inputs: try makeInputs(t), host: h, gate: ApprovalGate(source: ScriptedApproval([]), out: { _ in }),
                         ceiling: 2, audit: a, control: control, recovery: m,
                         recoveryPolicy: RecoveryPolicy(classification: "official-closed"), log: { _ in }).run()
        return (r, a)
    }

    func formWith(_ extra: UINode) -> UINode {
        let form = provisioningForm()
        return UINode(role: form.role, label: form.label, children: form.children + [extra])
    }

    /// R1: a pause sent by the officer arrives after recovery's post-answer hold. `stopAsked` reads it
    /// (checkpoint applies it, state becomes paused), returns false, and the press goes ahead while paused.
    func testR1_PauseArrivingAfterTheAnswerHoldIsSwallowedAndTheActHappensWhilePaused() throws {
        let t = try makeTask()
        let h = FakeHost()
        h.tree = formBehindNotice()
        var armed = false, pollsAfterAsk = 0
        let control = RunControl(poll: {
            guard armed else { return nil }
            pollsAfterAsk += 1
            // Poll 1 is hold()'s checkpoint right after the answer; poll 2 is the stop check after it.
            return pollsAfterAsk == 2 ? "{\"jsonrpc\":\"2.0\",\"method\":\"replay.pause\"}" : nil
        }, block: { "{\"jsonrpc\":\"2.0\",\"method\":\"replay.stop\"}" })
        var stateAtPress: [RunControl.State] = []
        h.onPress = { n, h in stateAtPress.append(control.state); if n.domId == "notice-close" { h.tree = provisioningForm() } }
        let m = Picking([("Close", press)])
        m.onAsk = { armed = true }
        let (r, _) = try run(h, t, m, control: control)
        print("R1 outcome:", r.outcome, "actions:", h.actions, "state at press:", stateAtPress)
        XCTAssertFalse(stateAtPress.contains(.paused), "recovery pressed while the run was paused")
    }

    /// R2: the dialog's text uses an inflection of the button's word ("Closing" vs "Close"). askedAbout matches
    /// whole words only, so "Close" here is the commit and is pressed.
    func testR2_InflectedQuestionWordLetsTheCommitDismissalThrough() throws {
        let t = try makeTask()
        let h = FakeHost()
        let dlg = UINode(role: "AXGroup", children: [UINode(role: "AXStaticText", label: "Closing this case is permanent and notifies the applicant."),
                                                      UINode(role: "AXButton", label: "Close", domId: "confirm-close"),
                                                      UINode(role: "AXButton", label: "Keep open", domId: "keep")],
                         subrole: "AXApplicationDialog")
        h.tree = formWith(dlg)
        h.onPress = { _, h in h.tree = provisioningForm() }
        let (r, _) = try run(h, t, Picking([("Close", press)]))
        print("R2 outcome:", r.outcome, "actions:", h.actions)
        XCTAssertFalse(h.actions.contains("press:Close"), "a 'Close' that commits closing the case was pressed")
    }

    /// R2b: same with "Dismiss" under "Dismissal of this appeal is final."
    func testR2b_DismissUnderDismissalIsPressed() throws {
        let t = try makeTask()
        let h = FakeHost()
        let dlg = UINode(role: "AXGroup", children: [UINode(role: "AXStaticText", label: "Dismissal of this appeal is final."),
                                                      UINode(role: "AXButton", label: "Dismiss", domId: "confirm-dismiss"),
                                                      UINode(role: "AXButton", label: "Back", domId: "back")],
                         subrole: "AXApplicationDialog")
        h.tree = formWith(dlg)
        h.onPress = { _, h in h.tree = provisioningForm() }
        let (r, _) = try run(h, t, Picking([("Dismiss", press)]))
        print("R2b outcome:", r.outcome, "actions:", h.actions)
        XCTAssertFalse(h.actions.contains("press:Dismiss"), "'Dismiss' that dismisses the appeal was pressed")
    }

    /// R3: a web modal without role=dialog (plain AXGroup, no subrole) whose buttons sit in their own button bar.
    /// askedAbout falls back to the button's parent (the bar), which holds only buttons, so the question is never seen.
    /// s1's field is missing so s1 diverges at find (no dialog is detected, so this is the only way in).
    func testR3_NonAriaModalWithButtonBarHidesTheQuestion() throws {
        let t = try makeTask()
        let h = FakeHost()
        let form = provisioningForm()
        let web = form.children[0]
        let bar = UINode(role: "AXGroup", children: [UINode(role: "AXButton", label: "Close", domId: "confirm-close"),
                                                      UINode(role: "AXButton", label: "Keep open", domId: "keep")])
        let modal = UINode(role: "AXGroup", children: [UINode(role: "AXStaticText", label: "Close this case for good?"), bar])
        h.tree = UINode(role: form.role, label: form.label,
                        children: [UINode(role: web.role, label: web.label, children: web.children.filter { $0.domId != "full-name" } + [modal])])
        let (r, _) = try run(h, t, Picking([("Close", press)]))
        print("R3 outcome:", r.outcome, "actions:", h.actions)
        XCTAssertFalse(h.actions.contains("press:Close"), "'Close' under 'Close this case for good?' was pressed")
    }

    /// R3b: same nesting inside a real dialog (subrole set) is caught: the dialog ancestor is found. Control case.
    func testR3b_SameNestingInsideARealDialogIsRefused() throws {
        let t = try makeTask()
        let h = FakeHost()
        let bar = UINode(role: "AXGroup", children: [UINode(role: "AXButton", label: "Close", domId: "confirm-close")])
        let dlg = UINode(role: "AXGroup", children: [UINode(role: "AXStaticText", label: "Close this case for good?"), bar],
                         subrole: "AXApplicationDialog")
        h.tree = formWith(dlg)
        let (_, _) = try run(h, t, Picking([("Close", press)]))
        XCTAssertFalse(h.actions.contains("press:Close"))
    }

    /// R4: Chrome's tab strip is in the window snapshot. Its tab close button is an AXButton described "Close";
    /// recovery may press it (closing the recorded page and its unsaved form), or press another AXTab (switching
    /// to a different web app). Both leave the recorded page.
    func testR4_BrowserTabStripCloseAndOtherTabArePressable() throws {
        let t = try makeTask()
        for target in ["Close", "Personal mail"] {
            let h = FakeHost()
            let base = formBehindNotice()
            let strip = UINode(role: "AXTabGroup", children: [
                UINode(role: "AXTab", label: "New staff account", children: [UINode(role: "AXButton", label: "Close", domId: nil)]),
                UINode(role: "AXTab", label: "Personal mail"),
            ])
            // The notice's own Close is removed so the only "Close" is the tab's.
            let dlg = base.children[1]
            let notice = UINode(role: dlg.role, children: dlg.children.filter { $0.domId != "notice-close" }, subrole: dlg.subrole)
            h.tree = UINode(role: base.role, label: base.label, children: [strip, base.children[0], notice])
            let (r, _) = try run(h, t, Picking([(target, press)]))
            print("R4 \(target) outcome:", r.outcome, "actions:", h.actions)
            XCTAssertFalse(h.actions.contains("press:\(target)"), "recovery pressed the browser's \(target) in the tab strip")
        }
    }
}
