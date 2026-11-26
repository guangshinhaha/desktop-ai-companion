# 0007: The local audit chain shows tampering only to someone without the key

Status: accepted limit (Shin, 26 Nov, the Done when on #104). It was first noted as a "Known limit" on PR #77. Refs #104, #29.

## Decision

The on-device audit chain is tamper-evident only against someone who does not have the database key. We accept this on the device and do not add more local guards for it.

Someone with the key can open the store with plain SQL. They can open the purge gate and delete the oldest records, or drop the triggers and rewrite the chain. The hashes use no secret, so a rewritten chain with a matching anchor and head passes `verify_audit`. The key does not fix this: anything the client can compute, a key holder on the same device can recompute.

## The off-device anchor

#29 (audit sync to the service) is the anchor: a copy the device's key holder cannot rewrite. It is not built. Until it is, a local `verify_audit` pass does not show that a key holder left the trail alone. Even then, records changed before their first sync are outside what the service's copy can show.
