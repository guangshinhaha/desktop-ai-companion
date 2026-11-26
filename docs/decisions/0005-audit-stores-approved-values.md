# 0005: The audit trail stores the approved values

Status: accepted by Shin, 24 Nov 2026, on the advisor-free answer in chat; line-by-line review pending. Refs #27, #22, TD section 7.2, TD section 9.

## Context

An approval record can say "approved creating an account for this person", or only "approved step s9 with inputs full_name and email". The first is accountable; the second holds less personal data. This is a personal-data and audit-trail change, so it is `risk:high`.

## Decision

- The audit stores the approved values, in the `shown` field, in the encrypted local store (the store is already encrypted).
- Retention defaults to 7 years (2555 days), as config.
- A central copy comes later, via #29.
- #22 is hands-on, so the PR that builds this uses `Refs #22`.

## Consequences

- Values live only in the audit store. Other events carry no values. The export for verification goes over stdin and never to disk.
- Run values print on a card only to a terminal; into a pipe or file they are hidden (#76).
- The hashes use no secret. Anyone holding a plain-text export can edit it or cut its tail, recompute the hashes and rewrite the control line, with no key, so `verify-audit` shows only that an export is consistent with itself. The live store has the limits in decision 0007: a key holder can rewrite the trail, and someone with access to the files can restore an older copy or delete the store. The off-device anchor is the central copy (#29), which is not built; even then, records changed before their first sync are outside what it can show.
- The `approved` record's values are the card the shell showed, stored as JSON. A `rejected` record stores none (#107).
- Retention is not enforced by code yet; it is config for #29 to apply.
