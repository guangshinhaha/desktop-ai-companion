import test from "node:test";
import assert from "node:assert/strict";
import { configFromEnv, loadConfig } from "../src/config.ts";
import { withinCeiling } from "../src/classification.ts";

const ok = { route: { kind: "stub" }, ceiling: "restricted", models: ["m"] };

test("config loads and rejects bad shapes", () => {
  assert.equal(loadConfig(ok).ceiling, "restricted");
  assert.throws(() => loadConfig({ ...ok, ceiling: "nope" }));
  assert.throws(() => loadConfig({ ...ok, ceiling: undefined }));
  assert.throws(() => loadConfig({ ...ok, models: [] }));
  assert.throws(() => loadConfig({ ...ok, route: { kind: "http" } }));
  assert.throws(() => loadConfig({ ...ok, levels: ["a", "a"], ceiling: "a" }));
  assert.throws(() => loadConfig(null));
});

test("no ceiling is assumed: missing or broken env config refuses to start", () => {
  assert.throws(() => configFromEnv({}));
  assert.throws(() => configFromEnv({ COMPANION_MODEL_CONFIG: "{" }));
  assert.equal(configFromEnv({ COMPANION_MODEL_CONFIG: JSON.stringify(ok) }).models[0], "m");
});

test("withinCeiling orders levels and treats unknown as above", () => {
  const c = loadConfig(ok);
  assert.equal(withinCeiling(c, "official-open"), true);
  assert.equal(withinCeiling(c, "restricted"), true);
  assert.equal(withinCeiling(c, "confidential"), false);
  assert.equal(withinCeiling(c, "__proto__"), false);
  assert.equal(withinCeiling(c, "constructor"), false);
  assert.equal(withinCeiling(c, undefined), false);
});
