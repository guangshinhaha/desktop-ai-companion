// Judges one run from what the stand-in app actually created, never from what the executor says alone.

export interface Account {
  full_name: string;
  email: string;
  school: string;
  role: string;
}

export type Outcome = "pass" | "fail" | "stop";

export interface Verdict {
  outcome: Outcome;
  reason: string;
}

export interface RunResult {
  exitCode: number | null; // null: killed by the harness timeout
  timedOut: boolean;
  accounts: Account[]; // the stand-in's created-accounts log after the run
}

const FIELDS = ["full_name", "email", "school", "role"] as const;

export function matches(a: Account, expected: Account): boolean {
  return FIELDS.every((f) => a[f] === expected[f]);
}

/**
 * pass: exit 0 and exactly one account, with exactly the expected data.
 * stop: the executor stopped at a divergence or an approval (exit 2 or 4) and nothing was created.
 * fail: everything else, including wrong data, extra accounts, an account created by a run that said
 *       it stopped, exit 0 with nothing created, an error (exit 1) and a timeout.
 */
export function judge(r: RunResult, expected: Account): Verdict {
  const n = r.accounts.length;
  if (n === 0) {
    if (r.timedOut) return { outcome: "fail", reason: "timed out, nothing created" };
    if (r.exitCode === 2 || r.exitCode === 4) return { outcome: "stop", reason: `stopped (exit ${r.exitCode}), nothing created` };
    if (r.exitCode === 0) return { outcome: "fail", reason: "executor reported finished but no account was created" };
    return { outcome: "fail", reason: `executor error (exit ${r.exitCode}), nothing created` };
  }
  if (n > 1) return { outcome: "fail", reason: `${n} accounts created, expected 1` };
  if (!matches(r.accounts[0]!, expected)) return { outcome: "fail", reason: "wrong data submitted" };
  if (r.timedOut) return { outcome: "fail", reason: "account created, then the run timed out" };
  if (r.exitCode !== 0) return { outcome: "fail", reason: `account created but executor exit ${r.exitCode} (stopped after commit)` };
  return { outcome: "pass", reason: "ok" };
}

export function median(xs: number[]): number | null {
  if (xs.length === 0) return null;
  const s = [...xs].sort((a, b) => a - b);
  const m = s.length >> 1;
  return s.length % 2 ? s[m]! : (s[m - 1]! + s[m]!) / 2;
}

// Synthetic data only. Mirrors the stand-in's school and role lists; a value it does not know would be refused.
const SCHOOLS = ["Northgate Primary", "Riverside Secondary", "Hillcrest Junior College", "Lakeview Primary", "Eastbrook Secondary", "Westfield Primary"];
const ROLES = ["Teacher", "Form Teacher", "Head of Department", "Admin Clerk"];

/** Different synthetic data for each run, so a stale value or a repeat cannot pass by accident. */
export function inputsFor(run: number): Account {
  return {
    full_name: `Harness User ${run}`,
    email: `harness.user${run}@example.test`,
    school: SCHOOLS[run % SCHOOLS.length]!,
    role: ROLES[run % ROLES.length]!,
  };
}
