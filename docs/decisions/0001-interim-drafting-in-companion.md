# 0001: Drafting and summarising are built into the companion, as an interim

Status: accepted, 24 Nov 2026. Refs #17, PRD v2 "What changed" item 1, PRD section 8 (Build now, consolidate later), TD section 14.

## Context

The companion exists to do the operating work Desk and Pair do not: record a task once, replay it by driving the real app. Officers also draft and summarise every day, today in Pair or Desk.

PRD v1 assumed the companion would reach Desk, the GovTech Harness, GovBrain and Muninn by integration from day one. PRD v2 says those paths are uncertain and that waiting for them would delay the evidence this product exists to collect. Inner-source access is itself an open question (PRD section 13, question 3), so no plan may depend on it.

Without drafting, an officer needs two tools during the pilot and has little reason to open the companion on a day with nothing to replay.

## Decision

Build the thinnest drafting and summarising jobs inside the companion, as an interim. They give the officer a reason to open it daily. They are not a bid to replace Desk (PRD positioning: "do what Desk can't").

Rules that keep it interim:

- Model calls go through the one model adapter, to a route cleared for the data's classification. The client holds no model credentials. Which route is open (PRD section 13, question 2; 0003 will record it).
- Drafting sits behind an adapter, so moving to the consolidation target is a configuration change, not a rewrite.
- Until Phase 2 ends, drafting is the one feature allowed that does not improve recording, replay or the library (CLAUDE.md, Scope). It stays thin.
- Only replay returns count toward the adoption kill criterion. Chat returns are tracked separately (PRD "What changed" item 9).
- Phase 0 does not depend on it. Phase 0 has no model and no server.

## Consolidation target and the open owner question

Target, per the PRD: **Desk, or a shared Harness agent that Desk and the companion both call.** The PRD's Build now, consolidate later table says "Desk or a shared Harness agent". Which of the two is not decided.

Open: **who on the Desk team owns the consolidation conversation, and which target** (PRD section 13, question 5). Owners are assigned on 7 Dec. Until then the target is a stated end, not an agreed plan. The PRD lists "Brief the Desk team before 7 Dec" as the mitigation for Desk turf. Nothing in the repo shows that has happened.

## Rejected alternatives

- **Integrate with Desk or the Harness from the start (PRD v1).** Access is unconfirmed and the dependency would delay Phase 1. Rejected for now. It stays the end state.
- **Do not draft at all; replay only.** Smallest build, but the officer keeps two tools and has less reason to return daily. Rejected because the adoption evidence needs daily opens. Unproven: that drafting does drive returns. The separate chat-returns count will show it.

## Consequences

- GRL carries a second, thin product surface. PRD section 12 names the risks: "The interim build becomes permanent" and "Solo capacity". The narrowing criterion in PRD section 10 applies: if the interim builds (drafting, audit, library store) take so much capacity that replay work stalls, narrow to recorder plus replay and re-decide.
- The model adapter and route clearance (0003) are on the path for drafting as well as for Phase 1 recovery.
- Drafting must not become the reason any other feature is in scope before Phase 2 ends.
- Consolidation is part of the Phase 3 exit (PRD section 12).

## Reopen or retire

Retire (mark superseded) when drafting moves to Desk or a shared Harness agent, which is the intended end.

Reopen if any of these happens:

- Desk or the Harness becomes reachable, with an owner named, before Phase 3. Then bring consolidation forward and stop building drafting.
- The Desk team objects to the overlap. Then narrow to replay only.
- The narrowing criterion in PRD section 10 fires.
- The 7 Dec review does not approve the interim-build approach.
