// Review screen (PRD R-4, R-5, R-6). All state lives in the Rust side; this file draws what
// review_view returns and sends edits. It keeps nothing in the browser (no web storage, no cookies).
// A fixed value typed here is sent once and the field is cleared straight away.
(function (root) {
  "use strict";

  const FLAG_TEXT = {
    unknown: "Not sure if this commits something. Treated as if it does until you say otherwise.",
    consequential: "Commits something (asks for approval when run)",
  };

  function createReview(doc, invoke, mount) {
    let view = null;
    let error = "";
    let openConstant = null; // step id whose fixed-value form is open

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

    async function send(command, args) {
      error = "";
      try {
        const out = await invoke(command, args);
        return { ok: true, out };
      } catch (e) {
        error = String(e);
        return { ok: false };
      } finally {
        draw();
      }
    }

    async function edit(e) {
      const r = await send("review_edit", { edit: e });
      if (r.ok) {
        view = r.out;
        openConstant = null;
        draw();
      }
      return r.ok;
    }

    function stepRow(s) {
      const kids = [el("div", { className: "say" }, s.say)];
      if (s.flag !== "none") {
        kids.push(el("div", { className: "flag " + s.flag, attrs: { "data-flag": s.flag } }, FLAG_TEXT[s.flag]));
      }
      if (s.is_constant) kids.push(el("div", { className: "note" }, "Fixed value: " + s.constant));
      const actions = el("div", { className: "actions" });
      const commits = el("select", {
        attrs: { "aria-label": "Does step " + s.say + " commit something?", "data-role": "commits" },
        on: { change: (ev) => edit({ op: "set_consequential", step: s.id, value: ev.target.value }) },
      });
      const choices = [["yes", "Commits something"], ["no", "Does not commit"]];
      if (s.consequential === "unknown") choices.unshift(["unknown", "Not sure"]);
      for (const [v, t] of choices) {
        const o = el("option", { value: v, disabled: v === "unknown" }, t);
        if (s.consequential === v) o.selected = true;
        commits.appendChild(o);
      }
      actions.appendChild(commits);
      if (s.can_be_constant) {
        actions.appendChild(
          el("button", { type: "button", textContent: "Use a fixed value", attrs: { "data-role": "constant-open" },
            on: { click: () => { openConstant = s.id; draw(); } } }),
        );
      }
      actions.appendChild(
        el("button", { type: "button", textContent: "Delete step", attrs: { "data-role": "delete" },
          on: { click: () => edit({ op: "delete_step", step: s.id }) } }),
      );
      kids.push(actions);
      if (openConstant === s.id) kids.push(constantForm(s));
      return el("li", { className: "step", attrs: { "data-step": s.id } }, ...kids);
    }

    function constantForm(s) {
      const value = el("input", { type: "text", attrs: { "aria-label": "Fixed value", autocomplete: "off", spellcheck: "false", "data-role": "constant-value" } });
      const tick = el("input", { type: "checkbox", attrs: { "data-role": "constant-tick" } });
      const ok = el("button", { type: "button", textContent: "Save fixed value", disabled: true, attrs: { "data-role": "constant-save" } });
      const sync = () => { ok.disabled = !(tick.checked && value.value.length > 0); };
      value.addEventListener("input", sync);
      tick.addEventListener("change", sync);
      ok.addEventListener("click", async () => {
        const typed = value.value;
        value.value = ""; // gone from the field whatever happens next
        await edit({ op: "mark_constant", step: s.id, value: typed, confirmed_no_personal_data: tick.checked });
      });
      const cancel = el("button", { type: "button", textContent: "Cancel", on: { click: () => { value.value = ""; openConstant = null; draw(); } } });
      return el("div", { className: "constant", attrs: { "data-role": "constant-form" } },
        el("label", {}, "Value that is the same every time ", value),
        el("label", {}, tick, " This value contains no personal data"),
        ok, cancel);
    }

    function inputRow(i) {
      const name = el("input", { type: "text", value: i.name, attrs: { "aria-label": "Input name", "data-role": "input-name" } });
      const rename = el("button", { type: "button", textContent: "Rename", attrs: { "data-role": "rename" },
        on: { click: () => edit({ op: "rename_input", from: i.name, to: name.value.trim() }) } });
      return el("li", { className: "input", attrs: { "data-input": i.name } }, name, rename,
        " " + i.type + ", used in " + (i.steps.length ? i.steps.join(", ") : "no step"));
    }

    function draw() {
      mount.replaceChildren();
      if (!view) {
        mount.appendChild(el("p", { attrs: { role: "alert" } }, error || "Nothing to review."));
        return;
      }
      const author = el("input", { type: "text", value: view.author || "", attrs: { "aria-label": "Your name or officer id", "data-role": "author", autocomplete: "off" } });
      const save = el("button", { type: "button", textContent: "Save task", attrs: { "data-role": "save" },
        on: { click: async () => {
          const r = await send("review_save", { author: author.value });
          if (r.ok) { view = null; mount.replaceChildren(el("p", { attrs: { role: "status" } }, "Saved as version " + r.out.version + ".")); }
        } } });
      const discard = el("button", { type: "button", textContent: "Discard recording", attrs: { "data-role": "discard" },
        on: { click: async () => {
          const r = await send("review_discard", {});
          if (r.ok) { view = null; mount.replaceChildren(el("p", { attrs: { role: "status" } }, "Discarded.")); }
        } } });
      const nameInput = el("input", { type: "text", value: view.name, attrs: { "aria-label": "Task name", "data-role": "task-name" },
        on: { change: (ev) => edit({ op: "rename_task", name: ev.target.value }) } });
      mount.appendChild(el("h1", {}, "Check what was recorded"));
      mount.appendChild(el("label", {}, "Task name ", nameInput));
      mount.appendChild(el("p", { attrs: { "data-role": "summary" } },
        view.steps.length + " steps. " + view.unknown_count + " not sure. " + view.consequential_count + " commit something."));
      mount.appendChild(el("p", { attrs: { "data-role": "app" } }, "Works in: " + view.apps.map((a) => a.bundle).join(", ")));
      if (view.preconditions.length) {
        mount.appendChild(el("h2", {}, "Before it runs"));
        mount.appendChild(el("ul", {}, ...view.preconditions.map((p) => el("li", {}, p))));
      }
      mount.appendChild(el("h2", {}, "Inputs"));
      mount.appendChild(view.inputs.length ? el("ul", { attrs: { "data-role": "inputs" } }, ...view.inputs.map(inputRow)) : el("p", {}, "This task needs no inputs."));
      mount.appendChild(el("h2", {}, "Steps"));
      mount.appendChild(el("ol", { attrs: { "data-role": "steps" } }, ...view.steps.map(stepRow)));
      if (view.problems.length) mount.appendChild(el("p", { attrs: { role: "alert", "data-role": "problems" } }, "Fix before saving: " + view.problems.join("; ")));
      mount.appendChild(el("label", {}, "Saved by ", author));
      mount.appendChild(el("div", { className: "actions" }, save, discard));
      if (error) mount.appendChild(el("p", { attrs: { role: "alert", "data-role": "error" } }, error));
    }

    return {
      async load() {
        const r = await send("review_view", {});
        if (r.ok) { view = r.out; draw(); }
      },
      draw,
    };
  }

  if (typeof module !== "undefined" && module.exports) module.exports = { createReview };
  else {
    root.addEventListener("DOMContentLoaded", () => {
      const ui = createReview(root.document, root.__TAURI__.core.invoke, root.document.getElementById("review"));
      ui.load();
    });
  }
})(typeof window !== "undefined" ? window : globalThis);
