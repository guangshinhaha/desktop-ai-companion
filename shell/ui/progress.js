// Replay progress (PRD C-3, C-4). The shell sends a view of the run; this file draws it and sends Pause,
// Resume and Stop. Stop is one click, or Escape or Cmd+Shift+Period (the shell also holds that one
// system-wide while a run is on). Text goes in as text nodes only. Nothing is kept in the browser.
(function (root) {
  "use strict";

  const MARK = { todo: "[ ]", current: "[>]", done: "[x]", stopped: "[stopped]", failed: "[failed]" };
  const OVER = {
    finished: "Finished.",
    stopped: "Stopped. Nothing further was done.",
    diverged: "The run stopped because the app did not match the recording.",
    not_approved: "The run stopped: a step was not approved.",
    refused: "The run was refused.",
    ended: "The run ended before it finished.",
  };

  function createProgress(doc, invoke, mount) {
    let view = null;
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

    async function send(cmd) {
      try {
        await invoke(cmd, {});
        error = "";
      } catch (e) {
        error = "Could not send that to the run. " + String(e);
        draw();
      }
    }

    function names(ids) {
      return ids.length ? ids.join(", ") : "none";
    }

    // The step the stop may have caught after its action: done, or not known. Otherwise nothing was acted on.
    function actedStep(r) {
      return r && view.state === "stopped" && (r.action_done === true || r.action_done === "unknown") && r.step ? r.step : null;
    }

    function outcomeText() {
      const acted = actedStep(view.report);
      if (acted) return "Stopped. Step " + acted + " may have taken effect.";
      return OVER[view.state] || "The run ended.";
    }

    function draw() {
      mount.replaceChildren();
      if (!view) {
        mount.appendChild(el("p", { attrs: { "data-role": "empty" } }, "No replay is running."));
        return;
      }
      const total = view.steps.length;
      const done = view.steps.filter((s) => s.status === "done").length;
      mount.appendChild(el("h1", {}, view.task));
      mount.appendChild(el("p", { attrs: { "data-role": "count" } }, done + " of " + total + " steps done, " + (total - done) + " remaining."));
      if (!view.over) {
        const say = view.current && view.current.say ? view.current.say : "Starting.";
        mount.appendChild(el("p", { className: "say", attrs: { "data-role": "say" } }, say));
        const label = { running: "Running", pausing: "Pausing after this step", paused: "Paused", stopping: "Stopping" }[view.state] || view.state;
        mount.appendChild(el("p", { attrs: { "data-role": "state" } }, label));
      } else {
        mount.appendChild(el("p", { attrs: { "data-role": "outcome" } }, outcomeText()));
      }
      mount.appendChild(el("ol", { attrs: { "data-role": "steps" } },
        ...view.steps.map((s) => el("li", { attrs: { "data-status": s.status } }, MARK[s.status] + " " + s.say))));
      if (view.over && view.report) {
        const r = view.report;
        const acted = actedStep(r);
        mount.appendChild(el("p", { attrs: { "data-role": "completed" } }, "Completed: " + names(r.completed || [])));
        // A step that may have been acted on is not listed as not done.
        mount.appendChild(el("p", { attrs: { "data-role": "not-run" } }, "Not done: " + names((r.not_run || []).filter((id) => id !== acted))));
        if (acted) {
          const how = r.action_done === true ? "its action was done and its check was not confirmed" : "whether its action happened is unknown";
          mount.appendChild(el("p", { className: "flag", attrs: { "data-role": "acted" } }, "Step " + acted + ": " + how + ". Check the app before running again."));
        }
        if (r.reason) mount.appendChild(el("p", { attrs: { "data-role": "reason" } }, "Reason: " + r.reason));
        if (view.state === "stopped" && !acted) {
          mount.appendChild(el("p", { attrs: { "data-role": "left-as-is" } }, "The app was left as it is."));
        }
      } else if (!view.over) {
        const stopping = view.state === "stopping";
        const paused = view.state === "paused" || view.state === "pausing";
        const pause = el("button", {
          type: "button", disabled: stopping || view.state === "pausing",
          attrs: { "data-role": paused ? "resume" : "pause" },
          on: { click: () => send(paused ? "run_resume" : "run_pause") },
        }, paused ? "Resume" : "Pause");
        const stop = el("button", {
          type: "button", disabled: stopping,
          attrs: { "data-role": "stop", autofocus: "" },
          on: { click: () => send("run_stop") },
        }, stopping ? "Stopping" : "Stop");
        mount.appendChild(el("div", { className: "actions" }, pause, stop));
        mount.appendChild(el("p", { attrs: { "data-role": "hint" } }, view.shortcut === false
          ? "Stop: the Stop button or Escape in this window. The system-wide shortcut (Cmd+Shift+Period) is not available."
          : "Stop: Escape or Cmd+Shift+Period."));
      }
      if (error) mount.appendChild(el("p", { attrs: { role: "alert", "data-role": "error" } }, error));
    }

    return {
      async load() {
        try {
          view = (await invoke("progress_current", {})) || null;
        } catch (e) {
          view = null;
          error = String(e);
        }
        draw();
      },
      update(v) {
        view = v || null;
        draw();
      },
      key(e) {
        if (!e || !view || view.over) return;
        if (e.key === "Escape") return send("run_stop");
      },
      draw,
    };
  }

  if (typeof module !== "undefined" && module.exports) module.exports = { createProgress };
  else {
    root.addEventListener("DOMContentLoaded", () => {
      const ui = createProgress(root.document, root.__TAURI__.core.invoke, root.document.getElementById("progress"));
      root.document.addEventListener("keydown", (e) => ui.key(e));
      root.__TAURI__.event.listen("progress", (e) => ui.update(e.payload));
      ui.load();
    });
  }
})(typeof window !== "undefined" ? window : globalThis);
