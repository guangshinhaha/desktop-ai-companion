# stand-in

Local web apps rebuilt from real workflow shapes, with synthetic data only. Targets for recording and replay (TD section 11). Node 24+, no install step, no dependencies.

## provisioning

An account-provisioning form: full name, email, school, role, and a Create account button.

```
cd stand-in && npm start          # http://127.0.0.1:3000/  (PORT=4000 npm start to change)
```

Submitting shows a page headed "Account created". Every field has an `id` and a `<label for>`, so it shows in the macOS accessibility tree with a label.

### Perturbation switches

Set by query parameter on the form URL. The form carries them into its submit URL, so they hold through the submit.

| Switch | Effect |
|---|---|
| `latency=<ms>` | Delays the submit response by that many ms (capped at 10000) |
| `reorder=1` | Shuffles the school list with a fixed seed (same order every time) |
| `confirm=1` | Submit first opens an "Are you sure?" dialog (`Yes, create` / `Cancel`) |
| `rename=1` | Button reads "Create" instead of "Create account" |
| `password=1` | Adds a required Password field (`#password`), for the recorder's redaction test. The server never stores or logs it. |

Combine freely: `/?latency=800&reorder=1&confirm=1`.

### Harness API (stable)

The reliability harness uses these two. Do not change their shape without updating the harness.

- `GET /api/accounts` returns `200` with `{"accounts":[{"id":1,"full_name":"...","email":"...","school":"...","role":"...","created_at":"<ISO time>"}]}` in creation order. `id` counts from 1.
- `POST /api/reset` (no body) returns `200` `{"ok":true,"accounts":0}`, empties the log and restarts `id` at 1.
- `GET /healthz` returns `200` `{"ok":true}`, for waiting until the server is up.

The log is in memory only and is lost when the server stops. Only accounts that pass validation (name and email present, known school and role, password present when `password=1`) are logged; an invalid submit returns `400`.

## data

`data/staff.csv` and `data/staff.xlsx` hold 5 rows (`full_name,email,school,role`) written by `npm run generate` from a fixed list of made-up names in `data/generate.ts`. Output is deterministic. No NRIC, phone or address fields. Names, schools and domains are invented (`.example.test`).

## tests

`npm test` runs HTTP-level tests and a real-browser test. The browser test drives headless Chrome over the DevTools protocol (`provisioning/cdp.ts`, no dependencies). It finds Chrome at `$CHROME`, the macOS app path or `/usr/bin/google-chrome`, skips if none is found, and fails when `CI` is set and Chrome is missing.
