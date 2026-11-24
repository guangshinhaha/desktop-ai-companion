import { median, type Outcome } from "./judge.ts";

export interface RunRecord {
  outcome: Outcome;
  reason: string;
  stepMs: number[]; // time for each step that completed
}

export interface Row {
  name: string;
  query: string;
  runs: RunRecord[];
}

export interface Meta {
  date: string;
  taskName: string;
  taskFile: string;
  runsPerPerturbation: number;
  executorCommit: string;
  host: string;
  ceiling: string;
}

export function summarise(row: Row) {
  const runs = row.runs.length;
  const passes = row.runs.filter((r) => r.outcome === "pass").length;
  const fails = row.runs.filter((r) => r.outcome === "fail").length;
  const stops = row.runs.filter((r) => r.outcome === "stop").length;
  const m = median(row.runs.flatMap((r) => r.stepMs));
  return { runs, passes, fails, stops, median: m };
}

const pct = (n: number, d: number) => (d === 0 ? "n/a" : `${((100 * n) / d).toFixed(0)}%`);

export function renderReport(rows: Row[], meta: Meta): string {
  const out: string[] = [];
  out.push(`# Replay reliability report: ${meta.taskName}`, "");
  out.push(`- Date: ${meta.date}`);
  out.push(`- Task file: \`${meta.taskFile}\`, replayed against the stand-in provisioning app (synthetic data only)`);
  out.push(`- Runs per perturbation: ${meta.runsPerPerturbation}`);
  out.push(`- Executor: ${meta.executorCommit}`);
  out.push(`- Host: ${meta.host}`);
  out.push(`- Step ceiling: ${meta.ceiling}`, "");
  out.push("| Perturbation | Runs | Passes | Fails | Stops at divergence | Success rate | Median step time (ms) |");
  out.push("|---|---|---|---|---|---|---|");
  for (const row of rows) {
    const s = summarise(row);
    out.push(`| ${row.name} | ${s.runs} | ${s.passes} | ${s.fails} | ${s.stops} | ${pct(s.passes, s.runs)} (${s.passes}/${s.runs}) | ${s.median === null ? "n/a" : Math.round(s.median)} |`);
  }
  out.push("");
  out.push("How to read it:", "");
  out.push("- Success rate is passes over all runs. Fails and stops are in the denominator.");
  out.push("- Pass: the executor finished and the stand-in's created-accounts log holds exactly one account with exactly the data given.");
  out.push("- Stop: the executor stopped at a divergence or an approval and the log is empty. Counted as not passing.");
  out.push("- Fail: anything else, including wrong data, extra accounts, an account created by a run that said it stopped, an executor error and a timeout.");
  out.push("- Median step time is the time from a step starting to its check passing, over every step that completed in the row. It includes the app's own response time (for the latency row, the added delay) and, for the Create step, the approval read.");
  out.push("- Approvals are answered from a file the harness writes for each run, through the executor's `--approval-file`. This happens only here, against the stand-in.", "");
  out.push("## Why runs did not pass", "");
  let any = false;
  for (const row of rows) {
    const counts = new Map<string, number>();
    for (const r of row.runs) {
      if (r.outcome === "pass") continue;
      const k = `${r.outcome}: ${r.reason}`;
      counts.set(k, (counts.get(k) ?? 0) + 1);
    }
    if (counts.size === 0) continue;
    any = true;
    out.push(`**${row.name}** (\`${row.query || "/"}\`)`, "");
    for (const [k, v] of counts) out.push(`- ${v} x ${k}`);
    out.push("");
  }
  if (!any) out.push("Every run passed.", "");
  return out.join("\n");
}
