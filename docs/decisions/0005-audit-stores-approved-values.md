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
- Anyone with the store key can forge a whole trail by recomputing the hashes. Only the central copy (#29) closes that.
- Retention is not enforced by code yet; it is config for #29 to apply.
