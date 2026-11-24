// HTML for the provisioning stand-in. Every field has an id and a <label for>.

export const SCHOOLS = [
  "Northgate Primary",
  "Riverside Secondary",
  "Hillcrest Junior College",
  "Lakeview Primary",
  "Eastbrook Secondary",
  "Westfield Primary",
];

export const ROLES = ["Teacher", "Form Teacher", "Head of Department", "Admin Clerk"];

export interface Switches {
  latency: number;
  reorder: boolean;
  confirm: boolean;
  rename: boolean;
  password: boolean;
}

const MAX_LATENCY_MS = 10_000;

export function parseSwitches(q: URLSearchParams): Switches {
  const ms = Number.parseInt(q.get("latency") ?? "0", 10);
  return {
    latency: Number.isFinite(ms) && ms > 0 ? Math.min(ms, MAX_LATENCY_MS) : 0,
    reorder: q.get("reorder") === "1",
    confirm: q.get("confirm") === "1",
    rename: q.get("rename") === "1",
    password: q.get("password") === "1",
  };
}

/** Query string that carries the active switches forward (empty when none). */
export function switchQuery(s: Switches): string {
  const p = new URLSearchParams();
  if (s.latency) p.set("latency", String(s.latency));
  if (s.reorder) p.set("reorder", "1");
  if (s.confirm) p.set("confirm", "1");
  if (s.rename) p.set("rename", "1");
  if (s.password) p.set("password", "1");
  const str = p.toString();
  return str ? `?${str}` : "";
}

export function esc(v: string): string {
  return v.replace(/&/g, "&amp;").replace(/</g, "&lt;").replace(/>/g, "&gt;").replace(/"/g, "&quot;");
}

// Fixed-seed PRNG so reorder=1 always gives the same order.
function mulberry32(seed: number): () => number {
  let a = seed;
  return () => {
    a = (a + 0x6d2b79f5) | 0;
    let t = Math.imul(a ^ (a >>> 15), 1 | a);
    t = (t + Math.imul(t ^ (t >>> 7), 61 | t)) ^ t;
    return ((t ^ (t >>> 14)) >>> 0) / 4294967296;
  };
}

export const REORDER_SEED = 20261124;

export function schoolList(reorder: boolean): string[] {
  const list = [...SCHOOLS];
  if (!reorder) return list;
  const rnd = mulberry32(REORDER_SEED);
  for (let i = list.length - 1; i > 0; i--) {
    const j = Math.floor(rnd() * (i + 1));
    [list[i], list[j]] = [list[j]!, list[i]!];
  }
  return list;
}

const STYLE = `body{font:16px system-ui,sans-serif;max-width:32rem;margin:2rem auto;padding:0 1rem}
label{display:block;margin-top:1rem;font-weight:600}
input,select{display:block;width:100%;padding:.4rem;margin-top:.25rem;font:inherit}
button{margin-top:1.5rem;padding:.5rem 1rem;font:inherit}
dialog{padding:1.5rem}`;

function layout(title: string, body: string): string {
  return `<!doctype html>
<html lang="en">
<head><meta charset="utf-8"><title>${esc(title)}</title><style>${STYLE}</style></head>
<body>
${body}
</body>
</html>
`;
}

export function formPage(s: Switches): string {
  const options = schoolList(s.reorder)
    .map((n) => `<option value="${esc(n)}">${esc(n)}</option>`)
    .join("");
  const roles = ROLES.map((n) => `<option value="${esc(n)}">${esc(n)}</option>`).join("");
  const button = s.rename ? "Create" : "Create account";
  const password = s.password
    ? `<label for="password">Password</label>
  <input id="password" name="password" type="password" autocomplete="new-password" required>`
    : "";
  const confirm = s.confirm
    ? `<dialog id="confirm-dialog" aria-labelledby="confirm-title">
  <p id="confirm-title">Are you sure?</p>
  <button id="confirm-yes" type="button">Yes, create</button>
  <button id="confirm-no" type="button">Cancel</button>
</dialog>
<script>
(function () {
  var form = document.getElementById("account-form");
  var dlg = document.getElementById("confirm-dialog");
  var ok = false;
  form.addEventListener("submit", function (e) {
    if (ok) return;
    e.preventDefault();
    dlg.showModal();
  });
  document.getElementById("confirm-yes").addEventListener("click", function () {
    ok = true; dlg.close(); form.requestSubmit();
  });
  document.getElementById("confirm-no").addEventListener("click", function () { dlg.close(); });
})();
</script>`
    : "";
  return layout(
    "New staff account",
    `<h1>New staff account</h1>
<form id="account-form" method="post" action="/accounts${switchQuery(s)}">
  <label for="full-name">Full name</label>
  <input id="full-name" name="full_name" type="text" autocomplete="off" required>
  <label for="email">Email</label>
  <input id="email" name="email" type="email" autocomplete="off" required>
  <label for="school">School</label>
  <select id="school" name="school" required>${options}</select>
  <label for="role">Role</label>
  <select id="role" name="role" required>${roles}</select>
  ${password}
  <button id="create" type="submit">${button}</button>
</form>
${confirm}`,
  );
}

export function createdPage(
  s: Switches,
  a: { full_name: string; email: string; school: string; role: string },
): string {
  return layout(
    "Account created",
    `<h1 id="result">Account created</h1>
<p>${esc(a.full_name)} (${esc(a.email)}), ${esc(a.role)} at ${esc(a.school)}.</p>
<p><a id="another" href="/${switchQuery(s)}">Create another account</a></p>`,
  );
}

export function errorPage(message: string): string {
  return layout(
    "Cannot create account",
    `<h1>Cannot create account</h1><p id="error">${esc(message)}</p>`,
  );
}
