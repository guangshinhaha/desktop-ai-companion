// List run screen (PRD E-5, TD 6). The officer names a CSV on this device, matches its columns to the task's
// inputs, starts the run, and sees each row's state. A stopped row shows the answers the officer may give.
// The file is read by the shell, never here: this page only ever sees column names, a row count and states,
// never a cell. Text goes in as text nodes only. Nothing is kept in the browser.
(function (root) {
  "use strict";

  const LABELS = {
    pending: "Waiting",
    running: "Running",
    awaiting_approval: "Waiting for your approval",
    committing: "Being submitted",
    done: "Done",
    diverged: "Stopped: the app did not do what the task expected",
    rejected: "You did not approve this row",
    unknown: "Unknown: check the app",
    skipped: "Skipped",
  };

  // What the officer may do with a row, by state. Mirrors the shell, which refuses anything else.
  function answersFor(state) {
    if (state === "unknown") return [["mark_done", "It was created: mark done"], ["mark_not_done", "It was not created: mark not done"]];
    if (state === "diverged") return [["skip", "Skip this row"], ["fixed_by_hand", "I did it by hand"]];
    return [];
  }

  // A run needs the officer when a row is unknown or diverged. Nothing continues until they answer.
  function needsOfficer(items) {
    return items.filter((i) => i.state === "unknown" || i.state === "diverged");
  }

  function summary(items) {
    const n = {};
    for (const i of items) n[i.state] = (n[i.state] || 0) + 1;
    return n;
  }

  function createListRun(doc, invoke, mount, poll) {
    const s = { path: "", taskId: "", version: "1", officer: "", preview: null, mapping: {}, run: null, status: null, error: "", busy: false };

    function el(tag, props, ...kids) {
      const e = doc.createElement(tag);
      for (const [k, v] of Object.entries(props || {})) {
        if (k === "on") for (const [ev, fn] of Object.entries(v)) e.addEventListener(ev, fn);
        else if (k === "attrs") for (const [a, b] of Object.entries(v)) e.setAttribute(a, b);
        else e[k] = v;
      }
      for (const k of kids) e.appendChild(typeof k === "string" ? doc.createTextNode(k) : k);
      return e;
    }

    async function call(cmd, args) {
      s.error = "";
      try { return await invoke(cmd, args); }
      catch (e) { s.error = String(e); return undefined; }
      finally { draw(); }
    }

    function field(label, key, role) {
      const input = el("input", { type: "text", value: s[key], attrs: { "data-role": role } });
      input.addEventListener("input", () => { s[key] = input.value; });
      return el("label", {}, label + " ", input);
    }

    async function choose() {
      const p = await call("listrun_preview", { path: s.path, taskId: s.taskId, version: Number(s.version) });
      if (p) { s.preview = p; s.mapping = { ...p.suggested }; draw(); }
    }

    async function start() {
      const run = await call("listrun_start", { path: s.path, taskId: s.taskId, version: Number(s.version), mapping: s.mapping, officer: s.officer, pid: null });
      if (run) { s.run = run; await refresh(); if (poll) poll(refresh); }
    }

    async function refresh() {
      if (!s.run) return;
      const st = await call("listrun_status", { run: s.run });
      if (st) { s.status = st; draw(); }
    }

    async function answer(item, a) {
      await call("listrun_answer", { run: s.run, item, answer: a, officer: s.officer });
      await refresh();
    }

    async function resume(from) {
      await call("listrun_resume", { run: s.run, path: s.path, from: from ?? null, officer: s.officer, pid: null });
      await refresh();
    }

    async function end() {
      await call("listrun_end", { run: s.run, officer: s.officer });
      await refresh();
    }

    function drawSetup() {
      mount.appendChild(el("h1", {}, "Run a task over a spreadsheet"));
      mount.appendChild(el("p", {}, "The file is read on this device. You approve each row; nothing is sent anywhere. CSV only: save an Excel sheet as CSV first."));
      mount.appendChild(field("CSV file path", "path", "path"));
      mount.appendChild(field("Task id", "taskId", "task"));
      mount.appendChild(field("Task version", "version", "version"));
      mount.appendChild(field("Your name (recorded with every approval)", "officer", "officer"));
      mount.appendChild(el("button", { type: "button", attrs: { "data-role": "choose" }, on: { click: choose } }, "Read the file"));
      if (!s.preview) return;
      mount.appendChild(el("p", { attrs: { "data-role": "found" } }, s.preview.rows + " rows. Match each input to a column."));
      for (const name of s.preview.inputs) {
        const sel = el("select", { attrs: { "data-role": "map-" + name } });
        sel.appendChild(el("option", { value: "" }, "(choose a column)"));
        for (const c of s.preview.columns) {
          const o = el("option", { value: c, selected: s.mapping[name] === c }, c);
          sel.appendChild(o);
        }
        sel.addEventListener("change", () => { if (sel.value) s.mapping[name] = sel.value; else delete s.mapping[name]; });
        mount.appendChild(el("label", {}, name + " comes from ", sel));
      }
      mount.appendChild(el("button", { type: "button", attrs: { "data-role": "start" }, on: { click: start } }, "Start: ask me before each row"));
    }

    function drawRun() {
      const items = s.status ? s.status.items : [];
      mount.appendChild(el("h1", {}, "Run " + s.run));
      const counts = summary(items);
      mount.appendChild(el("p", { attrs: { "data-role": "counts" } },
        Object.keys(LABELS).filter((k) => counts[k]).map((k) => counts[k] + " " + LABELS[k].toLowerCase()).join(", ")));
      const stuck = needsOfficer(items);
      if (stuck.length) {
        mount.appendChild(el("p", { className: "flag unknown", attrs: { "data-role": "blocked", role: "alert" } },
          "The run is stopped. Check the app, then answer for each row below. Nothing continues until you do."));
      }
      const list = el("ul", { attrs: { "data-role": "rows" } });
      for (const i of items) {
        const li = el("li", { attrs: { "data-item": String(i.item), "data-state": i.state } }, "Row " + i.item + ": " + (LABELS[i.state] || i.state));
        for (const [a, text] of answersFor(i.state)) {
          li.appendChild(el("button", { type: "button", attrs: { "data-role": a }, on: { click: () => answer(i.item, a) } }, text));
        }
        if (i.state === "diverged" || i.state === "rejected" || i.state === "pending") {
          li.appendChild(el("button", { type: "button", attrs: { "data-role": "resume-from" }, on: { click: () => resume(i.item) } }, "Continue from this row"));
        }
        list.appendChild(li);
      }
      mount.appendChild(list);
      mount.appendChild(el("p", {}, "To continue after a stop, pick the same file again above if this window was reopened: a different file is refused."));
      mount.appendChild(el("button", { type: "button", disabled: stuck.length > 0, attrs: { "data-role": "resume" }, on: { click: () => resume(null) } }, "Continue the run"));
      mount.appendChild(el("button", { type: "button", attrs: { "data-role": "end" }, on: { click: end } }, "End the run"));
    }

    function draw() {
      mount.replaceChildren();
      if (s.run) drawRun(); else drawSetup();
      if (s.error) mount.appendChild(el("p", { attrs: { role: "alert", "data-role": "error" } }, s.error));
    }

    return { draw, state: s, choose, start, refresh, answer, resume, end };
  }

  if (typeof module !== "undefined" && module.exports) module.exports = { createListRun, answersFor, needsOfficer, LABELS };
  else {
    root.addEventListener("DOMContentLoaded", () => {
      const poll = (fn) => { const t = root.setInterval(fn, 1500); return t; };
      const ui = createListRun(root.document, root.__TAURI__.core.invoke, root.document.getElementById("listrun"), poll);
      ui.draw();
    });
  }
})(typeof window !== "undefined" ? window : globalThis);
