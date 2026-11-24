// Duplicate keys, error paths, the approval requirement, and a pin for every schema constraint,
// so that loosening any of them fails a test.
import { test } from "node:test";
import assert from "node:assert/strict";
import { readFileSync, readdirSync } from "node:fs";
import { spawnSync } from "node:child_process";
import { validateTask, validateText } from "./src/validate.js";

const fixture = (p) => JSON.parse(readFileSync(new URL(`./fixtures/${p}`, import.meta.url), "utf8"));
const valid = () => fixture("provision-account.json");
const cli = new URL("./src/cli.js", import.meta.url).pathname;
const paths = (doc) => validateTask(doc).errors.map((e) => e.path);

// ---- duplicate keys: JSON.parse keeps the last one, so the file bytes could hide a literal ----
const dupExpected = {
  "duplicate-top-level": "/name",
  "duplicate-nested-value": "/steps/0/value",
  "duplicate-array-element": "/steps/1/consequential",
  "duplicate-constant-override": "/steps/0/value/confirmed_no_personal_data",
  "duplicate-constant-value": "/steps/0/value/constant",
};
const textFixture = (n) => new URL(`./fixtures/invalid-text/${n}.json`, import.meta.url);

test("every invalid-text fixture is covered by the table", () => {
  const files = readdirSync(new URL("./fixtures/invalid-text", import.meta.url)).map((f) => f.replace(/\.json$/, ""));
  assert.deepEqual(files.sort(), Object.keys(dupExpected).sort());
});

for (const [name, path] of Object.entries(dupExpected)) {
  test(`duplicate key fixture ${name}: JSON.parse looks clean but validation fails at ${path}`, () => {
    const text = readFileSync(textFixture(name), "utf8");
    // the trap: the parsed value alone passes the schema
    assert.equal(validateTask(JSON.parse(text)).valid, true, "parsed value should look valid");
    const r = validateText(text);
    assert.equal(r.valid, false);
    assert.ok(r.errors.some((e) => e.path === path && /duplicate key/.test(e.message)), JSON.stringify(r.errors));
    const c = spawnSync("node", [cli, textFixture(name).pathname], { encoding: "utf8" });
    assert.equal(c.status, 1);
    assert.ok(c.stderr.includes(`  ${path}: duplicate key`), c.stderr);
  });
}

test("duplicate keys: escaped spellings, __proto__ and keys containing / are caught; other depths are not", () => {
  const dup = (frag) => validateText(`{"a":{${frag}}}`).errors.filter((e) => /duplicate key/.test(e.message)).map((e) => e.path);
  assert.deepEqual(dup('"value":1,"\\u0076alue":2'), ["/a/value"]);
  assert.deepEqual(dup('"__proto__":1,"__proto__":2'), ["/a/__proto__"]);
  assert.deepEqual(dup('"x/y":1,"x/y":2'), ["/a/x~1y"]);
  assert.deepEqual(dup('"k":1,"j":{"k":1},"k":2'), ["/a/k"]);
  assert.deepEqual(dup('"k":1,"j":{"k":1}'), []);
});

test("syntax errors still throw (the cli maps them to exit 2)", () => {
  assert.throws(() => validateText('{"a":1,}'), SyntaxError);
  assert.throws(() => validateText(""), SyntaxError);
});

// ---- error paths ----
test("precondition check errors point at /preconditions/N/check, with no stray index", () => {
  const d = valid();
  d.preconditions = [{ app: "portal", check: { kind: "value_equals", input: "nope" } }];
  assert.deepEqual(paths(d), ["/preconditions/0/check/input"]);
  const e = valid();
  e.preconditions = [{ app: "ghost", check: { kind: "window_title", matches: "x" } }];
  assert.deepEqual(paths(e), ["/preconditions/0/app"]);
});

// ---- approval is required before a consequential or unknown step (TD 7.1) ----
test("consequential yes or unknown needs approval; no does not", () => {
  for (const c of ["yes", "unknown"]) {
    const d = valid();
    d.steps[0].consequential = c;
    assert.deepEqual(paths(d), ["/steps/0/approval"], c);
    d.steps[0].approval = { summary: "Fill in {full_name}", undoable: true };
    assert.equal(validateTask(d).valid, true, `${c} with approval`);
  }
  const d = valid();
  delete d.steps[1].approval;
  assert.deepEqual(paths(d), ["/steps/1/approval"]);
  assert.equal(valid().steps[0].consequential, "no");
  assert.equal(validateTask(valid()).valid, true);
});

// ---- constraints: each is pinned so loosening the schema fails a test ----
const bad = (name, fn, path) =>
  test(`constraint: ${name}`, () => {
    const d = valid();
    fn(d);
    const r = validateTask(d);
    assert.equal(r.valid, false);
    assert.ok(r.errors.some((e) => e.path === path), `${path} not in ${JSON.stringify(r.errors)}`);
  });

bad("id must look like tsk_xxx", (d) => (d.id = "anything goes"), "/id");
bad("id rejects empty", (d) => (d.id = ""), "/id");
bad("step id must look like sN", (d) => (d.steps[0].id = "first"), "/steps/0/id");
bad("app key is a lowercase name", (d) => { d.apps[0].key = "Sheet App"; }, "/apps/0/key");
bad("input name is a lowercase name", (d) => (d.inputs[0].name = "Full Name"), "/inputs/0/name");
bad("steps must not be empty", (d) => (d.steps = []), "/steps");
bad("apps must not be empty", (d) => (d.apps = []), "/apps");
bad("version 0", (d) => (d.version = 0), "/version");
bad("version negative", (d) => (d.version = -5), "/version");
bad("version integer", (d) => (d.version = 1.5), "/version");
bad("timeout_seconds 0", (d) => (d.steps[0].timeout_seconds = 0), "/steps/0/timeout_seconds");
bad("timeout_seconds negative", (d) => (d.steps[0].timeout_seconds = -1), "/steps/0/timeout_seconds");
bad("non-choice input cannot carry choices", (d) => (d.inputs[0].choices = ["a"]), "/inputs/0/choices");
bad("choice input needs at least one choice", (d) => (d.inputs[2].choices = []), "/inputs/2/choices");
bad("input type is an enum", (d) => (d.inputs[0].type = "secret"), "/inputs/0/type");
bad("input source is an enum", (d) => (d.inputs[0].source = "typed"), "/inputs/0/source");
bad("descriptor needs a property", (d) => (d.steps[0].target = {}), "/steps/0/target");
bad("descriptor path needs an item", (d) => (d.steps[0].target.path = []), "/steps/0/target/path");
bad("descriptor rejects unknown keys", (d) => (d.steps[0].target.text = "Alex"), "/steps/0/target/text");
bad("precondition rejects unknown keys", (d) => (d.preconditions[0].note = "x"), "/preconditions/0/note");
bad("provenance rejects unknown keys", (d) => (d.provenance.reviewed_by = "x"), "/provenance/reviewed_by");
bad("provenance versions are strings", (d) => (d.provenance.recorded_on_app_versions.portal = 131), "/provenance/recorded_on_app_versions/portal");
bad("provenance last_verified is a date", (d) => (d.provenance.last_verified = "soon"), "/provenance/last_verified");
bad("input rejects unknown keys", (d) => (d.inputs[0].default = "Alex"), "/inputs/0/default");
bad("app rejects unknown keys", (d) => (d.apps[0].user = "x"), "/apps/0/user");
bad("approval rejects unknown keys", (d) => (d.steps[1].approval.shown = "x"), "/steps/1/approval/shown");
bad("approval needs undoable", (d) => delete d.steps[1].approval.undoable, "/steps/1/approval/undoable");
bad("approval summary not empty", (d) => (d.steps[1].approval.summary = ""), "/steps/1/approval/summary");
bad("check rejects unknown keys", (d) => (d.steps[0].expect[0].text = "x"), "/steps/0/expect/0/text");
bad("check kind is an enum", (d) => (d.steps[0].expect[0].kind = "looks_like"), "/steps/0/expect/0/kind");
bad("appears needs a target", (d) => (d.steps[1].expect = [{ kind: "appears" }]), "/steps/1/expect/0/target");
bad("window_title needs matches", (d) => (d.preconditions[0].check = { kind: "window_title" }), "/preconditions/0/check/matches");
bad("created is a date-time", (d) => (d.created = "yesterday"), "/created");
bad("platform is an enum", (d) => (d.platform = "linux"), "/platform");
bad("name not empty", (d) => (d.name = ""), "/name");
bad("author not empty", (d) => (d.author = ""), "/author");
bad("say not empty", (d) => (d.steps[0].say = ""), "/steps/0/say");
bad("top-level required: apps", (d) => delete d.apps, "/apps");
bad("top-level required: inputs", (d) => delete d.inputs, "/inputs");
bad("set_value needs a target", (d) => delete d.steps[0].target, "/steps/0/target");
bad("read_value needs into", (d) => { d.steps[0].action = "read_value"; delete d.steps[0].value; }, "/steps/0/into");
bad("press cannot carry into", (d) => { d.inputs.push({ name: "got", type: "text", source: "read" }); d.steps[1].into = "got"; }, "/steps/1");
bad("focus_app cannot carry a value", (d) => { d.steps[0].action = "focus_app"; }, "/steps/0");
