import { test } from "node:test";
import assert from "node:assert/strict";
import { readFileSync, readdirSync } from "node:fs";
import { spawnSync } from "node:child_process";
import { validateTask } from "./src/validate.js";

const fixture = (p) => JSON.parse(readFileSync(new URL(`./fixtures/${p}`, import.meta.url), "utf8"));
const valid = () => fixture("provision-account.json");
const cli = new URL("./src/cli.js", import.meta.url).pathname;
const wrapper = new URL("../scripts/validate-task", import.meta.url).pathname;
const paths = (doc) => validateTask(doc).errors.map((e) => e.path);

test("the TD section 4 example passes", () => {
  assert.deepEqual(validateTask(valid()), { valid: true, errors: [] });
});

// Every invalid fixture, with the path the error must name.
const expected = {
  "no-expect": "/steps/0/expect",
  "empty-expect": "/steps/0/expect",
  "bare-literal-value": "/steps/0/value",
  "bare-number-value": "/steps/0/value",
  "value-extra-key": "/steps/0/value/text",
  "constant-unconfirmed": "/steps/0/value/confirmed_no_personal_data",
  "constant-no-confirmation": "/steps/0/value/confirmed_no_personal_data",
  "set-value-missing-value": "/steps/0/value",
  "step-extra-literal-field": "/steps/0/text",
  "expect-bare-literal": "/steps/0/expect/0/value",
  "press-with-value": "/steps/1",
  "unknown-action": "/steps/0/action",
  "missing-consequential": "/steps/1/consequential",
  "bad-consequential": "/steps/1/consequential",
  "undeclared-input": "/steps/0/value/input",
  "duplicate-step-id": "/steps/1/id",
  "wrong-format": "/format",
  "choice-without-choices": "/inputs/2/choices",
  "read-into-non-read-input": "/steps/1/into",
};

test("every invalid fixture is covered by the table", () => {
  const files = readdirSync(new URL("./fixtures/invalid", import.meta.url)).map((f) => f.replace(/\.json$/, ""));
  assert.deepEqual(files.sort(), Object.keys(expected).sort());
});

for (const [name, path] of Object.entries(expected)) {
  test(`invalid fixture ${name} fails and names ${path}`, () => {
    const r = validateTask(fixture(`invalid/${name}.json`));
    assert.equal(r.valid, false);
    assert.ok(r.errors.some((e) => e.path === path), `expected ${path} in ${JSON.stringify(r.errors)}`);
  });
}

// The key invariant: a typed value is {input} or a confirmed constant. Nothing else.
const withValue = (v) => {
  const d = valid();
  d.steps[0].value = v;
  return d;
};

test("typed value: {input: name} passes", () => {
  assert.equal(validateTask(withValue({ input: "email" })).valid, true);
});

test("typed value: a confirmed constant passes (string, number, boolean)", () => {
  for (const c of ["Teacher", 3, true]) {
    assert.equal(validateTask(withValue({ constant: c, confirmed_no_personal_data: true })).valid, true, String(c));
  }
});

test("typed value: every bare or malformed form fails at the value path", () => {
  const bad = [
    "Alex Example Tan", "", 0, 7, true, false, null, [], ["Alex"], {},
    { input: "Alex Example Tan" }, // a literal smuggled into the name slot
    { input: "Alex" },             // not a valid name (uppercase)
    { input: "" },
    { input: "full_name", constant: "x", confirmed_no_personal_data: true },
    { constant: "Teacher" },
    { constant: "Teacher", confirmed_no_personal_data: false },
    { constant: "Teacher", confirmed_no_personal_data: "true" },
    { constant: null, confirmed_no_personal_data: true },
    { constant: { a: 1 }, confirmed_no_personal_data: true },
    { confirmed_no_personal_data: true },
    { value: "Alex Example Tan" },
  ];
  for (const v of bad) {
    const r = validateTask(withValue(v));
    assert.equal(r.valid, false, JSON.stringify(v));
    assert.ok(r.errors.some((e) => e.path.startsWith("/steps/0/value")), JSON.stringify(v));
  }
});

test("typed value: select, type_keys and open_url take the same two forms only", () => {
  const mk = (action, value) => {
    const d = valid();
    const s = d.steps[0];
    s.action = action;
    s.value = value;
    if (action !== "select") delete s.target;
    if (action === "type_keys" || action === "open_url") s.expect = [{ kind: "window_title", matches: "Admin*" }];
    return d;
  };
  for (const action of ["select", "type_keys", "open_url"]) {
    assert.equal(validateTask(mk(action, { input: "role" })).valid, true, action);
    assert.equal(validateTask(mk(action, "Teacher")).valid, false, `${action} bare literal`);
  }
});

test("expect value_equals takes an input or a confirmed constant, never a bare value", () => {
  const withExpect = (c) => {
    const d = valid();
    d.steps[0].expect = [c];
    return d;
  };
  assert.equal(validateTask(withExpect({ kind: "value_equals", input: "full_name" })).valid, true);
  assert.equal(validateTask(withExpect({ kind: "value_equals", constant: "Teacher", confirmed_no_personal_data: true })).valid, true);
  for (const c of [
    { kind: "value_equals", value: "Alex" },
    { kind: "value_equals", constant: "Teacher" },
    { kind: "value_equals", constant: "Teacher", confirmed_no_personal_data: false },
    { kind: "value_equals" },
    { kind: "value_equals", input: "full_name", constant: "x", confirmed_no_personal_data: true },
  ]) {
    assert.equal(validateTask(withExpect(c)).valid, false, JSON.stringify(c));
  }
});

test("every step needs expect and consequential", () => {
  for (const key of ["expect", "consequential"]) {
    const d = valid();
    delete d.steps[1][key];
    assert.deepEqual(paths(d), [`/steps/1/${key}`]);
  }
  for (const c of ["yes", "no", "unknown"]) {
    const d = valid();
    d.steps[0].consequential = c;
    d.steps[0].approval = { summary: "Fill in {full_name}", undoable: true };
    assert.equal(validateTask(d).valid, true, c);
  }
  for (const c of ["", "Yes", true, null, "maybe"]) {
    const d = valid();
    d.steps[0].consequential = c;
    assert.equal(validateTask(d).valid, false, String(c));
  }
});

test("all eight v1 actions and all six check kinds are accepted", () => {
  const t = { role: "AXButton", label: "Go" };
  const step = (id, action, extra) => ({
    id, say: action, app: "portal", action, expect: [{ kind: "exists", target: t }], consequential: "no", ...extra,
  });
  const d = valid();
  d.inputs.push({ name: "copied", type: "text", source: "read" });
  d.steps = [
    step("s1", "press", { target: t }),
    step("s2", "set_value", { target: t, value: { input: "full_name" } }),
    step("s3", "select", { target: t, value: { input: "role" } }),
    step("s4", "type_keys", { value: { input: "email" } }),
    step("s5", "focus_app"),
    step("s6", "open_url", { value: { constant: "https://example.gov.sg", confirmed_no_personal_data: true } }),
    step("s7", "read_value", { target: t, into: "copied" }),
    step("s8", "wait_for"),
  ];
  d.steps[7].expect = [
    { kind: "exists", target: t },
    { kind: "appears", target: t },
    { kind: "disappears", target: t },
    { kind: "value_equals", input: "copied" },
    { kind: "window_title", matches: "Admin*" },
    { kind: "focused", target: t },
  ];
  assert.deepEqual(validateTask(d), { valid: true, errors: [] });
});

test("approval placeholders must name declared inputs", () => {
  const d = valid();
  d.steps[1].approval.summary = "Create an account for {nric}";
  assert.deepEqual(paths(d), ["/steps/1/approval/summary"]);
});

test("unknown top-level and nested fields are rejected (room is left by version, not by loose fields)", () => {
  const d = valid();
  d.sharing = { team: "x" };
  d.apps[0].extra = 1;
  assert.deepEqual(paths(d).sort(), ["/apps/0/extra", "/sharing"]);
});

test("platform accepts macos and windows, nothing else", () => {
  for (const p of ["macos", "windows"]) {
    const d = valid();
    d.platform = p;
    assert.equal(validateTask(d).valid, true);
  }
  const d = valid();
  d.platform = "linux";
  assert.deepEqual(paths(d), ["/platform"]);
});

test("cli: exit 0 for the valid fixture", () => {
  const r = spawnSync("node", [cli, new URL("./fixtures/provision-account.json", import.meta.url).pathname], { encoding: "utf8" });
  assert.equal(r.status, 0);
});

test("cli: exit non-zero and print the failing path for each invalid fixture", () => {
  for (const [name, path] of Object.entries(expected)) {
    const file = new URL(`./fixtures/invalid/${name}.json`, import.meta.url).pathname;
    const r = spawnSync("node", [cli, file], { encoding: "utf8" });
    assert.equal(r.status, 1, name);
    assert.ok(r.stderr.includes(`  ${path}:`), `${name}: ${r.stderr}`);
  }
});

test("cli: usage and unreadable input exit 2", () => {
  assert.equal(spawnSync("node", [cli], { encoding: "utf8" }).status, 2);
  assert.equal(spawnSync("node", [cli, "/nonexistent.json"], { encoding: "utf8" }).status, 2);
});

test("scripts/validate-task wrapper behaves the same", () => {
  const ok = new URL("./fixtures/provision-account.json", import.meta.url).pathname;
  const bad = new URL("./fixtures/invalid/no-expect.json", import.meta.url).pathname;
  assert.equal(spawnSync(wrapper, [ok], { encoding: "utf8" }).status, 0);
  const r = spawnSync(wrapper, [bad], { encoding: "utf8" });
  assert.equal(r.status, 1);
  assert.ok(r.stderr.includes("/steps/0/expect"));
});
