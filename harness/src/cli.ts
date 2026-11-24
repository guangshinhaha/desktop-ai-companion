import { execFileSync, spawn } from "node:child_process";
import { existsSync, mkdirSync, readFileSync, writeFileSync } from "node:fs";
import { hostname } from "node:os";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { parseArgs } from "node:util";
import { renderReport, summarise, type Row } from "./report.ts";
import { PERTURBATIONS, oneRun, standIn, type Options } from "./runner.ts";

const here = dirname(fileURLToPath(import.meta.url));
const repo = resolve(here, "../..");

const { values } = parseArgs({
  options: {
    task: { type: "string" },
    runs: { type: "string", default: "20" },
    out: { type: "string" },
    port: { type: "string", default: "3917" },
    executor: { type: "string", default: join(repo, "executor-macos/.build/debug/executor-macos") },
    chrome: { type: "string", default: "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome" },
    ceiling: { type: "string" },
  },
});
const die = (m: string): never => { console.error(m); process.exit(1); };
if (!values.task) die("usage: harness/run --task task.json --runs 20 [--out report.md] [--port 3917] [--executor path] [--chrome path] [--ceiling seconds]");
const runs = Number(values.runs);
if (!Number.isInteger(runs) || runs < 1 || runs > 1000) die("--runs must be a whole number from 1 to 1000");
const taskPath = resolve(values.task!);
if (!existsSync(taskPath)) die(`cannot read ${taskPath}`);
if (!existsSync(values.executor!)) die(`executor not built: ${values.executor} (run: cd executor-macos && swift build)`);
if (!existsSync(values.chrome!)) die(`Chrome not found: ${values.chrome}`);
const task = JSON.parse(readFileSync(taskPath, "utf8"));
const ceilingSec = values.ceiling ? Number(values.ceiling) : undefined;
if (ceilingSec !== undefined && !(ceilingSec > 0 && ceilingSec <= 3600)) die("--ceiling must be above 0 and at most 3600");

const base = `http://127.0.0.1:${values.port}`;
try { await standIn(base, "/healthz"); die(`something already answers on port ${values.port}; pick another with --port`); } catch { /* free: good */ }
const server = spawn("node", [join(repo, "stand-in/provisioning/server.ts")], { env: { ...process.env, PORT: values.port }, stdio: "ignore" });
let stopping = false;
const stopServer = () => { if (!stopping) { stopping = true; server.kill("SIGTERM"); } };
process.on("SIGINT", () => { stopServer(); process.exit(130); });
process.on("SIGTERM", () => { stopServer(); process.exit(143); });

try {
  for (let i = 0; ; i++) {
    try { await standIn(base, "/healthz"); break; } catch { if (i > 50) throw new Error("stand-in did not start"); await new Promise((r) => setTimeout(r, 100)); }
  }
  const o: Options = {
    base, executor: values.executor!, taskPath, chrome: values.chrome!, ceiling: ceilingSec,
    runTimeoutMs: ((ceilingSec ?? 30) * (task.steps?.length ?? 5) + 30) * 1000,
  };
  const rows: Row[] = [];
  for (const p of PERTURBATIONS) {
    const row: Row = { name: p.name, query: p.query, runs: [] };
    for (let n = 1; n <= runs; n++) {
      const r = await oneRun(o, p.query, n);
      row.runs.push(r);
      console.log(`${p.name} run ${n}/${runs}: ${r.outcome} (${r.reason})`);
    }
    const s = summarise(row);
    console.log(`${p.name}: ${s.passes}/${s.runs} passed, ${s.fails} failed, ${s.stops} stopped`);
    rows.push(row);
  }
  let commit = "unknown";
  try { commit = execFileSync("git", ["-C", repo, "rev-parse", "--short", "HEAD"]).toString().trim(); } catch { /* not a git checkout */ }
  const date = new Date().toISOString().slice(0, 10);
  const report = renderReport(rows, {
    date, taskName: task.name ?? taskPath, taskFile: values.task!, runsPerPerturbation: runs,
    executorCommit: `executor-macos debug build, repo commit ${commit}`,
    host: `${hostname()}, macOS ${execFileSync("sw_vers", ["-productVersion"]).toString().trim()}, Chrome with a fresh isolated profile per run`,
    ceiling: ceilingSec ? `${ceilingSec} s` : "executor default (30 s)",
  });
  const out = resolve(values.out ?? join(repo, "harness/reports", `${date}-replay-reliability.md`));
  mkdirSync(dirname(out), { recursive: true });
  writeFileSync(out, report);
  console.log(`\nreport written: ${out}\n\n${report}`);
} finally {
  stopServer();
}
