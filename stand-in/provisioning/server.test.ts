import { after, before, describe, test } from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import type { AddressInfo } from "node:net";
import type { Server } from "node:http";
import { createApp, type Account } from "./server.ts";
import { schoolList, SCHOOLS } from "./page.ts";
import { Browser, findChrome } from "./cdp.ts";
import { STAFF, toCsv, toXlsx } from "../data/generate.ts";

let server: Server;
let base: string;

before(async () => {
  server = createApp();
  await new Promise<void>((r) => server.listen(0, "127.0.0.1", () => r()));
  base = `http://127.0.0.1:${(server.address() as AddressInfo).port}`;
});
after(() => server.close());

const get = async (path: string) => (await fetch(base + path)).text();
const post = (path: string, fields: Record<string, string>) =>
  fetch(base + path, {
    method: "POST",
    headers: { "content-type": "application/x-www-form-urlencoded" },
    body: new URLSearchParams(fields),
  });
const reset = () => fetch(base + "/api/reset", { method: "POST" });
const valid = { full_name: "Test Person", email: "t@x.test", school: SCHOOLS[0]!, role: "Teacher" };
const schoolsIn = (html: string) =>
  [...html.matchAll(/<option value="([^"]+)">/g)].map((m) => m[1]!).filter((v) => SCHOOLS.includes(v));

describe("form", () => {
  test("serves the fields, each with an id and a label for it", async () => {
    const html = await get("/");
    for (const [id, label] of [["full-name", "Full name"], ["email", "Email"], ["school", "School"], ["role", "Role"]]) {
      assert.match(html, new RegExp(`<label for="${id}">${label}</label>`));
      assert.match(html, new RegExp(`id="${id}"`));
    }
    assert.match(html, /<button id="create" type="submit">Create account<\/button>/);
    assert.doesNotMatch(html, /type="password"/);
  });

  test("submit shows Account created and logs the account", async () => {
    await reset();
    const res = await post("/accounts", valid);
    assert.equal(res.status, 200);
    assert.match(await res.text(), /Account created/);
    const log = (await (await fetch(base + "/api/accounts")).json()) as { accounts: Account[] };
    assert.equal(log.accounts.length, 1);
    assert.equal(log.accounts[0].full_name, "Test Person");
    assert.equal(log.accounts[0].id, 1);
  });

  test("rejects an incomplete submission without logging it", async () => {
    await reset();
    const res = await post("/accounts", { ...valid, email: "" });
    assert.equal(res.status, 400);
    assert.deepEqual(((await (await fetch(base + "/api/accounts")).json()) as { accounts: Account[] }).accounts, []);
  });
});

describe("account log and reset", () => {
  test("reset empties the log and restarts ids", async () => {
    await post("/accounts", valid);
    await post("/accounts", valid);
    const r = await reset();
    assert.deepEqual(await r.json(), { ok: true, accounts: 0 });
    await post("/accounts", valid);
    const log = (await (await fetch(base + "/api/accounts")).json()) as { accounts: Account[] };
    assert.deepEqual(log.accounts.map((a) => a.id), [1]);
  });

  test("the password is never stored", async () => {
    await reset();
    await post("/accounts?password=1", { ...valid, password: "hunter2-not-real" });
    const raw = await get("/api/accounts");
    assert.doesNotMatch(raw, /hunter2/);
    assert.doesNotMatch(raw, /password/);
  });
});

describe("switches (HTTP)", () => {
  test("latency=<ms> delays the submit response", async () => {
    const t0 = Date.now();
    await post("/accounts", valid);
    const fast = Date.now() - t0;
    const t1 = Date.now();
    await post("/accounts?latency=400", valid);
    const slow = Date.now() - t1;
    assert.ok(slow >= 390, `slow=${slow}`);
    assert.ok(fast < 200, `fast=${fast}`);
    assert.match(await get("/?latency=400"), /action="\/accounts\?latency=400"/);
  });

  test("reorder=1 shuffles the school list with a fixed seed", async () => {
    const plain = schoolsIn(await get("/"));
    const a = schoolsIn(await get("/?reorder=1"));
    const b = schoolsIn(await get("/?reorder=1"));
    assert.deepEqual(plain, SCHOOLS);
    assert.deepEqual(a, b);
    assert.deepEqual([...a].sort(), [...SCHOOLS].sort());
    assert.notDeepEqual(a, plain);
    assert.deepEqual(a, schoolList(true));
  });

  test("confirm=1 adds an Are you sure? dialog", async () => {
    assert.doesNotMatch(await get("/"), /Are you sure\?/);
    assert.match(await get("/?confirm=1"), /<dialog id="confirm-dialog"[^>]*>\s*<p id="confirm-title">Are you sure\?<\/p>/);
  });

  test("rename=1 relabels the button Create", async () => {
    assert.match(await get("/?rename=1"), /<button id="create" type="submit">Create<\/button>/);
  });

  test("password=1 adds a labelled password field", async () => {
    const html = await get("/?password=1");
    assert.match(html, /<label for="password">Password<\/label>/);
    assert.match(html, /<input id="password" name="password" type="password"/);
  });
});

describe("synthetic data", () => {
  test("staff.csv is what the generator produces: 5 rows, no personal-data columns", () => {
    const csv = readFileSync(new URL("../data/staff.csv", import.meta.url), "utf8");
    assert.equal(csv, toCsv());
    assert.equal(STAFF.length, 5);
    assert.equal(csv.trim().split("\n")[0], "full_name,email,school,role");
    assert.doesNotMatch(csv, /\b[STFGM]\d{7}[A-Z]\b/i);
    assert.doesNotMatch(csv, /nric|phone|address/i);
  });

  test("staff.xlsx is the generator's output", () => {
    const buf = readFileSync(new URL("../data/staff.xlsx", import.meta.url));
    assert.equal(buf.subarray(0, 2).toString(), "PK");
    assert.ok(buf.equals(toXlsx()));
  });
});

// Real browser. CI sets CI=true, where Chrome must be present (macOS runners ship it);
// elsewhere the test skips when no Chrome is found.
const chrome = findChrome();
if (!chrome && process.env.CI) throw new Error("CI requires Chrome for the browser test");

describe("switches (real headless Chrome)", { skip: chrome ? false : "no Chrome found" }, () => {
  let b: Browser;
  before(async () => {
    b = await Browser.launch(chrome!);
  });
  after(async () => {
    await b?.close();
  });

  const schools = () =>
    b.eval<string[]>(`[...document.querySelectorAll("#school option")].map(o => o.textContent)`);
  const fill = async () => {
    await b.eval(`(() => {
      document.getElementById("full-name").value = "Browser Test";
      document.getElementById("email").value = "b@x.test";
      const pw = document.getElementById("password"); if (pw) pw.value = "x";
    })()`);
  };

  test("baseline: labelled fields resolve through their labels, submit shows Account created", async () => {
    await reset();
    await b.goto(`${base}/`);
    const labelled = await b.eval<string[]>(
      `[...document.querySelectorAll("label")].map(l => l.control && l.control.id === l.htmlFor ? l.textContent : "BAD")`,
    );
    assert.deepEqual(labelled, ["Full name", "Email", "School", "Role"]);
    assert.equal(await b.eval(`document.getElementById("create").textContent`), "Create account");
    await fill();
    await b.eval(`document.getElementById("create").click()`);
    await b.waitFor(`document.body.innerText.includes("Account created")`);
  });

  test("reorder=1 changes the rendered school list", async () => {
    await b.goto(`${base}/`);
    const plain = await schools();
    await b.goto(`${base}/?reorder=1`);
    const shuffled = await schools();
    assert.deepEqual(plain, SCHOOLS);
    assert.deepEqual(shuffled, schoolList(true));
    assert.notDeepEqual(shuffled, plain);
  });

  test("rename=1 relabels the button", async () => {
    await b.goto(`${base}/?rename=1`);
    assert.equal(await b.eval(`document.getElementById("create").textContent`), "Create");
  });

  test("password=1 shows a labelled password field", async () => {
    await b.goto(`${base}/?password=1`);
    assert.equal(
      await b.eval(`document.getElementById("password").type + "|" + document.querySelector('label[for="password"]').textContent`),
      "password|Password",
    );
  });

  test("confirm=1 shows Are you sure? on submit and only submits after Yes", async () => {
    await reset();
    await b.goto(`${base}/?confirm=1`);
    await fill();
    assert.equal(await b.eval(`document.getElementById("confirm-dialog").open`), false);
    await b.eval(`document.getElementById("create").click()`);
    await b.waitFor(`document.getElementById("confirm-dialog").open`);
    assert.match(await b.eval<string>(`document.getElementById("confirm-dialog").innerText`), /Are you sure\?/);
    assert.equal(await b.eval(`location.pathname`), "/");
    await b.eval(`document.getElementById("confirm-yes").click()`);
    await b.waitFor(`document.body.innerText.includes("Account created")`);
  });

  test("latency=600 delays the submit result in the browser", async () => {
    await b.goto(`${base}/?latency=600`);
    await fill();
    const t0 = Date.now();
    await b.eval(`document.getElementById("create").click()`);
    await b.waitFor(`document.body.innerText.includes("Account created")`);
    assert.ok(Date.now() - t0 >= 550, `took ${Date.now() - t0}ms`);
  });
});
