// Approval card (PRD A-1, TD 7.1). The shell holds the pending question; this file draws the card it
// returns and sends one answer. Only the Approve button sends `true`. Reject, Escape and closing the
// window all mean no, and nothing here counts time: waiting is never consent. Text goes in as text
// nodes only, never as markup, so task text cannot change the card. Nothing is kept in the browser.
(function (root) {
  "use strict";

  function createApproval(doc, invoke, mount) {
    let card = null;
    let answered = false;
    let error = "";

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

    async function answer(approved) {
      if (answered) return;
      answered = true;
      draw();
      try {
        await invoke("approval_answer", { approved: approved === true });
      } catch (e) {
        error = "Could not send your answer. The run stops. " + String(e);
        draw();
      }
    }

    function pairs(list, role, join) {
      return el("ul", { attrs: { "data-role": role } },
        ...list.map((p) => el("li", {}, join(p))));
    }

    function draw() {
      mount.replaceChildren();
      if (!card) {
        mount.appendChild(el("p", { attrs: { "data-role": "empty" } }, "Nothing is waiting for approval."));
        return;
      }
      mount.appendChild(el("h1", {}, "Approval needed"));
      mount.appendChild(el("p", { attrs: { "data-role": "who" } }, "Approving as " + card.officer + ". Run " + card.run + (card.row ? ", row " + card.row : "") + "."));
      mount.appendChild(el("p", { attrs: { "data-role": "where" } }, "Task: " + card.task + ". App: " + card.app + ". Step " + card.step + "."));
      if (card.unknown) {
        mount.appendChild(el("p", { className: "flag unknown", attrs: { "data-role": "unknown" } },
          "The recording could not tell whether this step commits, so it is treated as one that does."));
      }
      mount.appendChild(el("h2", {}, "The task says"));
      mount.appendChild(el("p", { attrs: { "data-role": "summary" } }, card.summary || "(the task gives no summary for this step)"));
      mount.appendChild(el("h2", {}, "What will happen"));
      mount.appendChild(el("p", { attrs: { "data-role": "will-do" } }, card.will_do));
      mount.appendChild(el("p", { attrs: { "data-role": "undo" } },
        "The task says this can be undone: " + (card.undoable_claim ? "yes" : "no") + ". The companion cannot check that."));
      mount.appendChild(el("h2", {}, "Values this run enters"));
      mount.appendChild(card.inputs.length
        ? pairs(card.inputs, "inputs", (p) => p.name + ": " + p.value)
        : el("p", {}, "None."));
      if (card.constants.length) {
        mount.appendChild(el("h2", {}, "Fixed values the task types"));
        mount.appendChild(pairs(card.constants, "constants", (p) => p.value + " into " + p.name));
      }
      const reject = el("button", { type: "button", disabled: answered, attrs: { "data-role": "reject", autofocus: "" }, on: { click: () => answer(false) } }, "Reject and stop");
      const approve = el("button", { type: "button", disabled: answered, attrs: { "data-role": "approve" }, on: { click: () => answer(true) } }, "Approve this step");
      mount.appendChild(el("div", { className: "actions" }, reject, approve));
      if (error) mount.appendChild(el("p", { attrs: { role: "alert", "data-role": "error" } }, error));
    }

    return {
      async load() {
        try {
          card = (await invoke("approval_current", {})) || null;
        } catch (e) {
          card = null;
          error = String(e);
        }
        draw();
      },
      // Escape rejects. There is no key for Approve.
      key(e) {
        if (e && e.key === "Escape") return answer(false);
      },
      draw,
    };
  }

  if (typeof module !== "undefined" && module.exports) module.exports = { createApproval };
  else {
    root.addEventListener("DOMContentLoaded", () => {
      const ui = createApproval(root.document, root.__TAURI__.core.invoke, root.document.getElementById("approval"));
      root.document.addEventListener("keydown", (e) => ui.key(e));
      ui.load();
    });
  }
})(typeof window !== "undefined" ? window : globalThis);
