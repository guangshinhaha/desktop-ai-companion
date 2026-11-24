import { spawn, spawnSync, type ChildProcess } from "node:child_process";
import { chmodSync, mkdtempSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { judge, inputsFor, type Account, type Verdict } from "./judge.ts";

export const PERTURBATIONS = [
  { name: "none", query: "" },
  { name: "latency", query: "?latency=1500" },
  { name: "reorder", query: "?reorder=1" },
  { name: "confirm", query: "?confirm=1" },
  { name: "rename", query: "?rename=1" },
];

export interface Options {
  base: string; // stand-in URL, no trailing slash
  executor: string; // executor-macos binary
  taskPath: string;
  chrome: string;
  ceiling?: number;
  /** Wait for the page's form to show in the accessibility tree before the replay starts (default true). */
  probeReady?: boolean;
  runTimeoutMs: number;
  /** Starts the browser on url and returns the process to drive. Injectable for tests. */
  launchBrowser?: (url: string, profileDir: string) => ChildProcess;
}

export async function standIn(base: string, path: string, method = "GET"): Promise<any> {
  const r = await fetch(base + path, { method });
  if (!r.ok) throw new Error(`${method} ${path} returned ${r.status}`);
  return r.json();
}

function defaultLaunch(chrome: string) {
  return (url: string, profileDir: string) =>
    spawn(chrome, [`--user-data-dir=${profileDir}`, "--no-first-run", "--no-default-browser-check", "--new-window", url], { stdio: "ignore" });
}

/** A process started directly is not an app the executor can find until macOS registers it. Wait for that, not a fixed sleep. */
async function untilRegistered(pid: number, ceilingMs = 20000) {
  const end = Date.now() + ceilingMs;
  for (;;) {
    const r = spawnSync("lsappinfo", ["find", `pid=${pid}`], { encoding: "utf8" });
    if (r.error || r.status !== 0) return; // lsappinfo not available: let the executor decide
    if (r.stdout.trim()) return;
    if (Date.now() > end) return; // the executor reports it if it never registered
    await new Promise((res) => setTimeout(res, 100));
  }
}

/**
 * The executor wakes Chrome's accessibility tree once, when it starts. Started the instant Chrome opens, it
 * wakes a browser with no page yet and then never sees the form (found by the first smoke run: element not
 * found on step 1 every time). So the harness waits, with the executor's own `resolve`, until the form is
 * visible, then starts the replay. This is setup before the first step and is in no step time. It is a
 * Phase 1 finding that a replay started before the page is ready cannot see it; the report says so.
 */
async function untilPageVisible(executor: string, dir: string, ceilingMs = 40000): Promise<boolean> {
  const descriptor = join(dir, "ready.json");
  writeFileSync(descriptor, JSON.stringify({ role: "AXTextField", label: "Full name" }));
  const end = Date.now() + ceilingMs;
  while (Date.now() < end) {
    const code = await new Promise<number | null>((res) => {
      const c = spawn(executor, ["resolve", "--descriptor", descriptor, "--ceiling", "2"], { stdio: "ignore" });
      c.once("close", (x) => res(x));
    });
    if (code === 0) return true;
  }
  return false;
}

async function stopBrowser(p: ChildProcess) {
  if (p.exitCode !== null || p.signalCode !== null) return;
  const exited = new Promise<void>((res) => p.once("exit", () => res()));
  p.kill("SIGTERM");
  const t = setTimeout(() => p.kill("SIGKILL"), 5000);
  await exited;
  clearTimeout(t);
}

export interface RunOutcome extends Verdict {
  stepMs: number[];
}

/** One run: reset, open the page with the perturbation, replay, judge from the stand-in's log. */
export async function oneRun(o: Options, query: string, run: number): Promise<RunOutcome> {
  await standIn(o.base, "/api/reset", "POST");
  const expected = inputsFor(run);
  const dir = mkdtempSync(join(tmpdir(), "harness-"));
  const profile = join(dir, "profile");
  const inputs = join(dir, "inputs.json");
  const approvals = join(dir, "approvals");
  writeFileSync(inputs, JSON.stringify(expected), { mode: 0o600 });
  // The executor's own file mechanism, one y per approval it may ask. Test-only, against the stand-in.
  writeFileSync(approvals, "y\ny\ny\ny\n", { mode: 0o600 });
  chmodSync(approvals, 0o600);
  const browser = (o.launchBrowser ?? defaultLaunch(o.chrome))(`${o.base}/${query}`, profile);
  try {
    if (!browser.pid) throw new Error("browser did not start");
    await untilRegistered(browser.pid);
    const visible = o.probeReady === false ? true : await untilPageVisible(o.executor, dir);
    const args = ["replay", o.taskPath, "--inputs", inputs, "--pid", String(browser.pid), "--approval-file", approvals];
    if (o.ceiling) args.push("--ceiling", String(o.ceiling));
    const child = spawn(o.executor, args, { stdio: ["ignore", "pipe", "pipe"] });
    const stepMs: number[] = [];
    let started: { id: string; at: number } | null = null;
    let execReason = "";
    let buf = "";
    child.stdout!.on("data", (d: Buffer) => {
      buf += d.toString();
      let i;
      while ((i = buf.indexOf("\n")) >= 0) {
        const line = buf.slice(0, i);
        buf = buf.slice(i + 1);
        const now = performance.now();
        const s = /^step (\S+):/.exec(line);
        if (s) started = { id: s[1]!, at: now };
        const done = /^ {2}done: (\S+)/.exec(line);
        if (done && started && started.id === done[1]) stepMs.push(now - started.at);
        const why = /^ {2}reason: (.*)$/.exec(line);
        if (why) execReason = why[1]!;
      }
    });
    let stderr = "";
    child.stderr!.on("data", (d: Buffer) => { stderr += d.toString(); });
    let timedOut = false;
    const killer = setTimeout(() => { timedOut = true; child.kill("SIGKILL"); }, o.runTimeoutMs);
    const exitCode = await new Promise<number | null>((res) => child.once("close", (c) => res(c)));
    clearTimeout(killer);
    const accounts: Account[] = (await standIn(o.base, "/api/accounts")).accounts;
    const v = judge({ exitCode, timedOut, accounts }, expected);
    const detail = (visible ? "" : "the form was not visible in the accessibility tree after 40 s, before the replay started; ") + execReason || (v.outcome !== "pass" ? stderr.trim().split("\n")[0] ?? "" : "");
    // Never put the run's values in a reason: the executor's reasons name steps and elements, not inputs.
    return { ...v, reason: detail && v.outcome !== "pass" ? `${v.reason}; executor: ${detail}` : v.reason, stepMs };
  } finally {
    await stopBrowser(browser);
    rmSync(dir, { recursive: true, force: true });
  }
}
