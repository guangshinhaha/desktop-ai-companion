// Validates a recorded task against schema/task.v1.json, then runs the cross-reference
// checks a JSON Schema cannot express (declared inputs and apps, unique ids).
// Errors are { path, message } where path is a JSON Pointer to the failing place.
import { readFileSync } from "node:fs";
import Ajv2020 from "ajv/dist/2020.js";
import addFormats from "ajv-formats";

const schema = JSON.parse(readFileSync(new URL("../task.v1.json", import.meta.url), "utf8"));
const ajv = new Ajv2020({ allErrors: true, strict: true, strictRequired: false, allowUnionTypes: true });
addFormats(ajv);
const check = ajv.compile(schema);

function fromAjv(e) {
  let path = e.instancePath;
  let message = e.message;
  if (e.keyword === "required") {
    path += "/" + e.params.missingProperty;
    message = "is required";
  } else if (e.keyword === "additionalProperties") {
    path += "/" + e.params.additionalProperty;
    message = "is not allowed in task/v1";
  } else if (e.keyword === "oneOf" && /\/value$/.test(path)) {
    message = 'must be {"input": name} or {"constant": value, "confirmed_no_personal_data": true}';
  } else if (e.keyword === "not") {
    message = "is not allowed with this action or check kind";
  }
  return { path: path || "/", message };
}

function semantic(doc) {
  const errors = [];
  const add = (path, message) => errors.push({ path, message });
  const inputs = new Map();
  doc.inputs.forEach((inp, i) => {
    if (inputs.has(inp.name)) add(`/inputs/${i}/name`, `duplicate input name "${inp.name}"`);
    else inputs.set(inp.name, inp);
  });
  const apps = new Set();
  doc.apps.forEach((a, i) => {
    if (apps.has(a.key)) add(`/apps/${i}/key`, `duplicate app key "${a.key}"`);
    apps.add(a.key);
  });
  const needInput = (path, name) => {
    if (!inputs.has(name)) add(path, `refers to undeclared input "${name}"`);
  };
  const needApp = (path, key) => {
    if (!apps.has(key)) add(path, `refers to undeclared app "${key}"`);
  };
  const checks = (base, list) =>
    list.forEach((c, j) => {
      if (c.input !== undefined) needInput(`${base}/${j}/input`, c.input);
    });

  (doc.preconditions ?? []).forEach((p, i) => {
    needApp(`/preconditions/${i}/app`, p.app);
    checks(`/preconditions/${i}/check`, [p.check]);
  });
  const ids = new Set();
  doc.steps.forEach((s, i) => {
    const at = `/steps/${i}`;
    if (ids.has(s.id)) add(`${at}/id`, `duplicate step id "${s.id}"`);
    ids.add(s.id);
    needApp(`${at}/app`, s.app);
    if (s.value?.input !== undefined) needInput(`${at}/value/input`, s.value.input);
    if (s.into !== undefined) {
      needInput(`${at}/into`, s.into);
      if (inputs.get(s.into) && inputs.get(s.into).source !== "read")
        add(`${at}/into`, `input "${s.into}" must have source "read"`);
    }
    checks(`${at}/expect`, s.expect);
    for (const m of s.approval?.summary.matchAll(/\{([^}]*)\}/g) ?? [])
      needInput(`${at}/approval/summary`, m[1]);
  });
  return errors;
}

export function validateTask(doc) {
  if (check(doc)) {
    const errors = semantic(doc);
    return { valid: errors.length === 0, errors };
  }
  const seen = new Set();
  const errors = [];
  for (const e of check.errors.map(fromAjv)) {
    const key = `${e.path}|${e.message}`;
    if (!seen.has(key)) {
      seen.add(key);
      errors.push(e);
    }
  }
  return { valid: false, errors };
}
