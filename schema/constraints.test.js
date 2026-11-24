// Pins the rest of the schema's constraints, found by mutation testing: required keys, empty
// strings, per-action and per-check-kind rules, and types. Loosening any of them fails here.
import { test } from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { validateTask } from "./src/validate.js";

const valid = () => JSON.parse(readFileSync(new URL("./fixtures/provision-account.json", import.meta.url), "utf8"));
const at = (d, ptr) => ptr.split("/").filter(Boolean).reduce((o, k) => o[k], d);

// A fixture variant that has one of everything, so each required key can be removed in place.
const rich = () => {
  const d = valid();
  d.steps[1].approval = { summary: "Create {full_name}", undoable: false };
  return d;
};

const requiredKeys = {
  "": ["format", "id", "version", "name", "author", "created", "platform", "apps", "inputs", "steps"],
  "/apps/0": ["key", "bundle"],
  "/preconditions/0": ["app", "check"],
  "/preconditions/0/check": ["kind"],
  "/inputs/0": ["name", "type", "source"],
  "/steps/0": ["id", "say", "app", "action", "expect", "consequential"],
  "/steps/1/approval": ["summary", "undoable"],
  "/steps/0/expect/0": ["kind"],
};

for (const [ptr, keys] of Object.entries(requiredKeys)) {
  for (const key of keys) {
    test(`required: ${ptr || "/"} needs ${key}`, () => {
      const d = rich();
      delete at(d, ptr)[key];
      const r = validateTask(d);
      assert.equal(r.valid, false);
      assert.ok(r.errors.some((e) => e.path === `${ptr}/${key}`), JSON.stringify(r.errors));
    });
  }
}

test("empty strings are rejected wherever a string names something", () => {
  const cases = [
    ["/apps/0/bundle", (d) => (d.apps[0].bundle = "")],
    ["/apps/1/url", (d) => (d.apps[1].url = "")],
    ["/steps/0/target/role", (d) => (d.steps[0].target.role = "")],
    ["/steps/0/target/label", (d) => (d.steps[0].target.label = "")],
    ["/steps/1/expect/0/target/label_matches", (d) => (d.steps[1].expect[0].target.label_matches = "")],
    ["/steps/0/target/dom_id", (d) => (d.steps[0].target.dom_id = "")],
    ["/steps/0/target/path/0", (d) => (d.steps[0].target.path[0] = "")],
    ["/steps/0/target/anchors/0", (d) => (d.steps[0].target.anchors[0] = "")],
    ["/preconditions/0/check/matches", (d) => (d.preconditions[0].check.matches = "")],
  ];
  for (const [path, fn] of cases) {
    const d = valid();
    fn(d);
    assert.ok(validateTask(d).errors.some((e) => e.path === path), path);
  }
});

test("types: description string, undoable boolean, choices strings", () => {
  const cases = [
    ["/description", (d) => (d.description = 5)],
    ["/steps/1/approval/undoable", (d) => (d.steps[1].approval.undoable = "no")],
    ["/inputs/2/choices/0", (d) => (d.inputs[2].choices[0] = 5)],
  ];
  for (const [path, fn] of cases) {
    const d = valid();
    fn(d);
    assert.ok(validateTask(d).errors.some((e) => e.path === path), path);
  }
});

// ---- per-action rules ----
const step = (action, extra = {}) => ({
  id: "s1", say: action, app: "portal", action,
  expect: [{ kind: "exists", target: { label: "Go" } }], consequential: "no", ...extra,
});
const withSteps = (...steps) => {
  const d = valid();
  d.inputs.push({ name: "got", type: "text", source: "read" });
  d.steps = steps;
  return d;
};
const T = { role: "AXButton", label: "Go" };
const V = { input: "full_name" };

const actionRules = [
  // [action, valid extras, [invalid extras with why]]
  ["set_value", { target: T, value: V }, [{ target: T, value: V, into: "got" }, { value: V }, { target: T }]],
  ["select", { target: T, value: V }, [{ target: T, value: V, into: "got" }, { value: V }, { target: T }]],
  ["type_keys", { value: V }, [{ value: V, into: "got" }, {}]],
  ["open_url", { value: V }, [{ value: V, target: T }, { value: V, into: "got" }, {}]],
  ["press", { target: T }, [{ target: T, value: V }, { target: T, into: "got" }, {}]],
  ["read_value", { target: T, into: "got" }, [{ target: T, into: "got", value: V }, { into: "got" }, { target: T }]],
  ["focus_app", {}, [{ value: V }, { into: "got" }]],
  ["wait_for", {}, [{ value: V }, { into: "got" }]],
];
for (const [action, ok, bads] of actionRules) {
  test(`action ${action}: allowed shape passes, each wrong shape fails`, () => {
    assert.equal(validateTask(withSteps(step(action, ok))).valid, true, "valid shape");
    for (const extra of bads) {
      assert.equal(validateTask(withSteps(step(action, extra))).valid, false, JSON.stringify(extra));
    }
  });
}

// ---- per-check-kind rules ----
const T2 = { role: "AXStaticText", label: "Done" };
const checkRules = [
  ["exists", { target: T2 }, [{}, { target: T2, matches: "x" }, { target: T2, input: "full_name" }, { target: T2, constant: "x" }, { target: T2, confirmed_no_personal_data: true }]],
  ["appears", { target: T2 }, [{}, { target: T2, matches: "x" }, { target: T2, input: "full_name" }]],
  ["disappears", { target: T2 }, [{}, { target: T2, matches: "x" }]],
  ["focused", { target: T2 }, [{}, { target: T2, constant: "x" }]],
  ["window_title", { matches: "Admin*" }, [{}, { matches: "x", target: T2 }, { matches: "x", input: "full_name" }, { matches: "x", constant: "y" }, { matches: "x", confirmed_no_personal_data: true }]],
  ["value_equals", { input: "full_name" }, [{}, { matches: "x", input: "full_name" }, { confirmed_no_personal_data: true }, { constant: "x" }]],
];
for (const [kind, ok, bads] of checkRules) {
  test(`check ${kind}: allowed shape passes, each wrong shape fails`, () => {
    const mk = (extra) => {
      const d = valid();
      d.steps[0].expect = [{ kind, ...extra }];
      return d;
    };
    assert.equal(validateTask(mk(ok)).valid, true, "valid shape");
    for (const extra of bads) assert.equal(validateTask(mk(extra)).valid, false, JSON.stringify(extra));
  });
}
