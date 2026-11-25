#!/usr/bin/env node
// A stand-in for executor-macos, for the list-run tests. It speaks the shell's JSON-RPC (TD 3.2) and does
// what a replay of "create an account" does, but over HTTP against the stand-in app instead of through the
// accessibility tree, so the stand-in's own log can show what was created. Synthetic data only.
//
// Behaviour comes from `params.task.fake`:
//   url        the stand-in app, e.g. http://127.0.0.1:3000
//   die        "before_post" | "after_post": kill this process (SIGKILL) at that point for the row whose
//              full_name is `die_on`. before_post is after `approved` is recorded, before anything is created.
//   diverge_on a full_name: report a divergence before asking for approval, with nothing created.
//   steps      2: after the first account, a second consequential step (s5) creates "second.<email>", with its
//              own approval card and its own `approved` record.
import { createInterface } from "node:readline";

if (process.argv[2] !== "serve") process.exit(1);
const lines = createInterface({ input: process.stdin })[Symbol.asyncIterator]();
const send = (o) => process.stdout.write(JSON.stringify({ jsonrpc: "2.0", ...o }) + "\n");
let n = 0;
async function ask(method, params) {
  const id = `x${++n}`;
  send({ id, method, params });
  for (;;) {
    const { value, done } = await lines.next();
    if (done) process.exit(3);
    const m = JSON.parse(value);
    if (m.id === id && m.method === undefined) return m;
  }
}

const first = await lines.next();
const req = JSON.parse(first.value);
const { task, inputs } = req.params;
const fake = task.fake ?? {};
const finish = (result) => {
  process.stdout.write(JSON.stringify({ jsonrpc: "2.0", id: req.id, result }) + "\n", () => process.exit(0));
  return new Promise(() => {}); // wait for the write callback to exit
};

const must = async (event, extra = {}) => {
  const r = await ask("audit.append", { event, ...extra });
  if (r.error) process.exit(4); // a record that cannot be written stops the run, nothing done
};

await must("run_started");
if (fake.diverge_on === inputs.full_name) {
  await must("step_diverged", { step: "s4", shown: "element not found" });
  await must("run_finished", { shown: "diverged" });
  await finish({ outcome: "diverged", step: "s4", reason: "element not found", completed: [], not_run: ["s4"] });
}

const shown = `Create a staff account for ${inputs.full_name} (${inputs.email}) with role ${inputs.role}`;
const a = await ask("approval.request", { card: { step: "s4", task: task.name ?? "t", summary: shown } });
if (a.result?.approved !== true) {
  // No values: what was declined is the step, not the person it was for (decision 0005).
  await must("rejected", { step: "s4", shown: "not approved" });
  await must("run_finished", { shown: "not_approved" });
  await finish({ outcome: "not_approved", step: "s4", completed: [], not_run: ["s4"] });
}
await must("approved", { step: "s4", shown });
const dies = fake.die && fake.die_on === inputs.full_name;
if (dies && fake.die === "before_post") process.kill(process.pid, "SIGKILL");

const body = new URLSearchParams({ full_name: inputs.full_name, email: inputs.email, school: inputs.school, role: inputs.role });
const res = await fetch(`${fake.url}/accounts`, { method: "POST", body, headers: { "content-type": "application/x-www-form-urlencoded" } });
if (dies && fake.die === "after_post") process.kill(process.pid, "SIGKILL");
if (!res.ok) {
  await must("step_diverged", { step: "s4", shown: "the app refused" });
  await must("run_finished", { shown: "diverged" });
  await finish({ outcome: "diverged", step: "s4", reason: "the app refused", completed: [], not_run: ["s4"] });
}
if (fake.steps === 2) {
  const shown2 = `Create a second account for ${inputs.full_name} (second.${inputs.email})`;
  const b = await ask("approval.request", { card: { step: "s5", task: task.name ?? "t", summary: shown2 } });
  if (b.result?.approved !== true) {
    await must("rejected", { step: "s5", shown: "not approved" });
    await must("run_finished", { shown: "not_approved" });
    await finish({ outcome: "not_approved", step: "s5", completed: ["s4"], not_run: ["s5"] });
  }
  await must("approved", { step: "s5", shown: shown2 });
  const body2 = new URLSearchParams({ full_name: inputs.full_name, email: `second.${inputs.email}`, school: inputs.school, role: inputs.role });
  await fetch(`${fake.url}/accounts`, { method: "POST", body: body2, headers: { "content-type": "application/x-www-form-urlencoded" } });
}
await must("run_finished", { shown: "finished" });
await finish({ outcome: "finished", completed: fake.steps === 2 ? ["s4", "s5"] : ["s4"], not_run: [] });
