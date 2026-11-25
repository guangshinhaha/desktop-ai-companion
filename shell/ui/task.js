// Task notes screen (PRD M-1, #44). Steps on one side, the task's notes beside them. All state
// lives in the Rust side. Notes may contain personal data, so this file keeps nothing in the
// browser (no web storage, no cookies), and the field is cleared as soon as a note is sent.
(function (root) {
  "use strict";

  function createTaskNotes(doc, invoke, mount) {
    let catalog = [];
    let detail = null;
    let error = "";
    let authorText = "";

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

    async function call(command, args) {
      error = "";
      try {
        return { ok: true, out: await invoke(command, args) };
      } catch (e) {
        error = String(e);
        return { ok: false };
      } finally {
        draw();
      }
    }

    async function pick(id) {
      if (!id) { detail = null; draw(); return; }
      const r = await call("task_detail", { taskId: id });
      if (r.ok) { detail = r.out; draw(); }
    }

    function noteRow(n) {
      const when = new Date(n.at * 1000).toLocaleString();
      return el("li", { attrs: { "data-note": String(n.id) } },
        el("div", { className: "who" }, n.author + ", " + when),
        el("div", { attrs: { "data-role": "note-body" } }, n.body),
        el("button", { type: "button", textContent: "Remove note", attrs: { "data-role": "note-remove" },
          on: { click: async () => {
            const r = await call("task_note_remove", { taskId: detail.id, noteId: n.id });
            if (r.ok) { detail = r.out; draw(); }
          } } }));
    }

    // A count and a date for the whole device (later the team). Never a name.
    function reuseLine(r) {
      const where = r.scope === "team" ? "by your team" : "on this device";
      if (!r.replays) return "Not replayed yet " + where + ".";
      const n = r.replays === 1 ? "1 time" : r.replays + " times";
      const last = r.last_success_at ? "last succeeded " + new Date(r.last_success_at * 1000).toLocaleDateString() : "no successful run yet";
      return "Replayed " + n + " " + where + ", " + last + ".";
    }

    function draw() {
      mount.replaceChildren();
      mount.appendChild(el("h1", {}, "Task notes"));
      if (!catalog.length) {
        mount.appendChild(el("p", { attrs: { "data-role": "empty" } }, "No saved tasks yet."));
      } else {
        const sel = el("select", { attrs: { "aria-label": "Task", "data-role": "task-pick" },
          on: { change: (ev) => pick(ev.target.value) } });
        sel.appendChild(el("option", { value: "" }, "Choose a task"));
        for (const t of catalog) {
          const o = el("option", { value: t.id }, t.name);
          if (detail && detail.id === t.id) o.selected = true;
          sel.appendChild(o);
        }
        mount.appendChild(el("label", {}, "Task ", sel));
      }
      if (detail) {
        const steps = el("ol", { attrs: { "data-role": "steps" } }, ...detail.steps.map((s) => el("li", { attrs: { "data-step": s.id } }, s.say)));
        const list = detail.notes.length
          ? el("ul", { attrs: { "data-role": "notes" } }, ...detail.notes.map(noteRow))
          : el("p", { attrs: { "data-role": "no-notes" } }, "No notes yet.");
        const author = el("input", { type: "text", value: authorText, attrs: { "aria-label": "Your name or officer id", "data-role": "author", autocomplete: "off" },
          on: { input: (ev) => { authorText = ev.target.value; } } });
        const text = el("textarea", { attrs: { "aria-label": "New note", "data-role": "note-text", maxlength: String(detail.max_chars), autocomplete: "off", spellcheck: "false" } });
        const add = el("button", { type: "button", textContent: "Add note", attrs: { "data-role": "note-add" },
          on: { click: async () => {
            const typed = text.value;
            if (!typed.trim()) return;
            text.value = ""; // gone from the field whatever happens next
            authorText = author.value;
            const r = await call("task_note_add", { taskId: detail.id, author: author.value, body: typed });
            if (r.ok) { detail = r.out; draw(); }
          } } });
        mount.appendChild(el("p", {}, detail.name + ", version " + detail.version));
        const entry = catalog.find((t) => t.id === detail.id);
        if (entry && entry.reuse) mount.appendChild(el("p", { className: "who", attrs: { "data-role": "reuse" } }, reuseLine(entry.reuse)));
        mount.appendChild(el("div", { className: "layout" },
          el("section", {}, el("h2", {}, "Steps"), steps),
          el("section", { className: "notes" }, el("h2", {}, "Notes and decisions"), list,
            el("p", { className: "who", attrs: { "data-role": "privacy" } },
              "Notes stay on this device, encrypted. Do not put personal data in a note unless the task needs it."),
            el("label", {}, "Added by ", author), text, el("div", { className: "actions" }, add))));
      }
      if (error) mount.appendChild(el("p", { attrs: { role: "alert", "data-role": "error" } }, error));
    }

    return {
      async load() {
        const r = await call("task_catalog", {});
        if (r.ok) { catalog = r.out; draw(); }
      },
      draw,
    };
  }

  if (typeof module !== "undefined" && module.exports) module.exports = { createTaskNotes };
  else {
    root.addEventListener("DOMContentLoaded", () => {
      createTaskNotes(root.document, root.__TAURI__.core.invoke, root.document.getElementById("task")).load();
    });
  }
})(typeof window !== "undefined" ? window : globalThis);
