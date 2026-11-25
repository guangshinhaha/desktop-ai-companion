// Chat in the canvas (#31, TD 8). The conversation lives in memory for as long as the canvas is
// open; the Rust side saves each answered turn to the local chat table. This file keeps nothing in
// the browser (no web storage, no cookies) and builds the page with text nodes only.
//
// Rules the tests pin down:
// - No classification set: nothing is sent, and what the officer typed stays in the input.
// - Refused or failed: the turn is taken back off the screen and the input is left as it was.
// - A dropped document asks for a summary; if it cannot be sent yet it waits for the officer.
(function (root) {
  "use strict";

  const R = typeof module !== "undefined" && module.exports ? require("./router.js") : root.CompanionRouter;

  const SUMMARY_PROMPT = (name, text) => `Summarise this document in a few short paragraphs, then list any actions or deadlines it mentions.\n\nDocument: ${name}\n\n---\n${text}`;

  function createChat(doc, api, ui) {
    // ui: { input, select, log, status }
    let messages = []; // what the service has seen and answered
    let turns = []; // what the officer sees: { role, text }
    let pendingDoc = null; // a dropped document not yet sent
    let busy = false;
    let opened = false;
    let flow = null; // a question the companion has asked and is waiting on (task offer, inputs, learn)
    let officer = null; // who approves steps in a run; asked once, kept in memory only
    let lastRequest = null; // which path the latest request took, for "what did you do?"

    function el(tag, text, attrs) {
      const e = doc.createElement(tag);
      if (text) e.textContent = text;
      for (const [k, v] of Object.entries(attrs || {})) e.setAttribute(k, v);
      return e;
    }

    function open() {
      if (opened) return;
      opened = true;
      api.expand();
    }

    function draw() {
      ui.log.replaceChildren(...turns.map((t) => el("p", t.text, { "data-role": t.role })));
      if (turns.length) open();
    }

    function say(text, alert) {
      ui.status.textContent = text;
      ui.status.setAttribute("role", alert ? "alert" : "status");
      if (text) open();
    }

    function classification() {
      return ui.select.value;
    }

    // One turn. `label` is what the officer sees for their side; `content` is what is sent.
    async function turn(content, label) {
      if (busy) return false;
      if (!classification()) {
        say("Choose a classification for this chat first. Nothing was sent.", true);
        return false;
      }
      busy = true;
      say("");
      const shown = turns.length;
      turns.push({ role: "user", text: label }, { role: "assistant", text: "" });
      draw();
      const reply = turns[turns.length - 1];
      const outgoing = [...messages, { role: "user", content }];
      let r;
      try {
        r = await api.send(classification(), outgoing, (chunk) => {
          reply.text += chunk;
          draw();
        });
      } catch (e) {
        r = { ok: false, error: { message: "Could not send that. Your text is still here. Try again." } };
      }
      busy = false;
      if (!r.ok) {
        turns.length = shown; // take the turn back; the officer's text is still where it was
        draw();
        say((r.error && r.error.message) || "Could not send that. Your text is still here. Try again.", true);
        return false;
      }
      messages = [...outgoing, { role: "assistant", content: reply.text }];
      return true;
    }

    // A local turn: answered on this machine, never sent to chat, never saved to chat history.
    function local(shown, reply) {
      turns.push({ role: "user", text: shown }, { role: "assistant", text: reply });
      draw();
    }

    async function chatPath(text) {
      const ok = await turn(text, text);
      lastRequest = { path: "chat", sent: ok };
      return ok;
    }

    function offerTask(task, explicit, text) {
      flow = { kind: "offer_task", task, explicit, text };
      lastRequest = { path: "task", task: task.name, started: false };
      local(text, offerText(task));
    }

    // What the officer is told before a task runs (#42). A task that diverged because the app changed says so,
    // and a newer recording that has not passed yet is offered on its own: "new" runs it to verify it.
    function offerText(task) {
      const step = /^s[0-9]+$/.test(task.flagged_step || "") ? ` at step ${task.flagged_step}` : "";
      const cand = task.candidate && task.candidate.version ? ` A new recording of it has been saved but has not passed yet. Say "new" to run the new recording and verify it, or "yes" to run the current one.` : "";
      if (task.status === "needs_verification") {
        return `I have a saved task called "${task.name}", but it needs verification: it stopped${step} last time, so the app may have changed.${cand || " Run it anyway? Say yes or no."} Each step that changes something still asks you first.`;
      }
      if (cand) return `I have a saved task called "${task.name}".${cand}`;
      if (task.status === "unverified") return `I have a saved task called "${task.name}". It has not finished a run yet. Run it? Say yes or no.`;
      return `I have a saved task called "${task.name}". Run it? Say yes or no.`;
    }

    async function startTask() {
      const { task, values } = flow;
      flow = null;
      const inputs = {};
      task.inputs.forEach((inp, i) => { inputs[inp.name] = values[i]; });
      busy = true;
      let r;
      try {
        r = await api.replay(task, inputs, officer);
      } catch (e) {
        r = { ok: false, message: "The task could not be started." };
      }
      busy = false;
      lastRequest.started = !!r.ok;
      lastRequest.outcome = r.ok ? r.outcome || "done" : "not started";
      const note = r.ok && typeof r.verification_note === "string" ? ` ${r.verification_note}` : "";
      turns.push({ role: "assistant", text: (r.ok ? `Finished with outcome: ${lastRequest.outcome}.` : r.message || "The task could not be started.") + note });
      draw();
    }

    // Asks for the officer's name if it is not known, then each input in turn. Nothing is defaulted
    // or guessed: an empty answer asks again, and the run starts only once every input is given.
    async function collect() {
      if (!officer) {
        flow.kind = "need_officer";
        turns.push({ role: "assistant", text: "Who is approving this run? Give your name or officer id." });
        draw();
        return true;
      }
      flow.kind = "collecting";
      const inp = flow.task.inputs[flow.values.length];
      if (!inp) return startTask();
      const extra = inp.choices ? ` Choices: ${inp.choices.join(", ")}.` : "";
      turns.push({ role: "assistant", text: `What is ${inp.name.replace(/_/g, " ")}?${extra}` });
      draw();
      return true;
    }

    async function continueFlow(text) {
      const f = flow;
      const cancel = R.norm(text) === "cancel";
      if (f.kind === "offer_task") {
        if (f.task.candidate && f.task.candidate.version && ["new", "verify", "verify new", "run new", "the new one"].includes(R.norm(text))) {
          // The new recording runs as its own version, with its own inputs. Only its own finished run makes it the default.
          const c = f.task.candidate;
          flow = { kind: "collecting", task: { id: f.task.id, name: f.task.name, version: c.version, inputs: c.inputs || [] }, values: [] };
          turns.push({ role: "user", text });
          draw();
          return collect();
        }
        if (R.isYes(text)) {
          flow = { kind: "collecting", task: f.task, values: [] };
          turns.push({ role: "user", text });
          draw();
          return collect();
        }
        if (R.isNo(text)) {
          flow = null;
          if (f.explicit) { lastRequest.outcome = "declined"; local(text, "OK. I did not run it."); return true; }
          return chatPath(f.text); // only mentioned the task: it was a chat after all
        }
        flow = null;
        return ask(text); // something else: a new request
      }
      if (f.kind === "offer_learn") {
        if (R.isYes(text)) {
          flow = { kind: "ask_app" };
          local(text, "Which app will you do it in? Give its bundle id, for example com.apple.TextEdit. Say cancel to stop.");
          return true;
        }
        if (R.isNo(text)) { flow = null; return chatPath(f.text); }
        flow = null;
        return ask(text);
      }
      if (f.kind === "ask_app") {
        if (cancel) { flow = null; local(text, "OK. Nothing was recorded."); return true; }
        const bundle = text.trim();
        if (!/^[A-Za-z0-9][A-Za-z0-9.-]*$/.test(bundle) || !bundle.includes(".")) {
          local(text, "That is not a bundle id. It looks like com.apple.TextEdit. Try again, or say cancel.");
          return true;
        }
        flow = null;
        let r;
        try { r = await api.record(bundle); } catch (e) { r = { ok: false, message: "Recording could not start." }; }
        lastRequest.recording = !!r.ok;
        local(text, r.ok ? "Recording. Do the task once in that app. I will show you what I learned to review when you stop." : r.message || "Recording could not start.");
        return true;
      }
      if (cancel) { flow = null; lastRequest.outcome = "cancelled"; local(text, "OK. I did not run it."); return true; }
      if (f.kind === "need_officer") {
        const name = text.trim();
        turns.push({ role: "user", text: "(answer entered)" });
        if (name.length > 80) {
          turns.push({ role: "assistant", text: "That name is too long. Try again, or say cancel." });
          draw();
          return true;
        }
        officer = name;
        return collect();
      }
      // collecting: one input at a time
      const inp = f.task.inputs[f.values.length];
      let value = text.trim();
      if (inp.choices) value = inp.choices.find((c) => R.norm(c) === R.norm(value)) || "";
      turns.push({ role: "user", text: "(answer entered)" });
      if (!value) {
        turns.push({ role: "assistant", text: inp.choices ? `Pick one of: ${inp.choices.join(", ")}.` : "I need a value for that. I will not guess one." });
        draw();
        return true;
      }
      f.values.push(value);
      return collect();
    }

    function whatDidYouDo() {
      const r = lastRequest;
      if (!r) return "I have not done anything yet.";
      if (r.path === "chat") return r.sent ? "I took the chat path: your request went to chat." : "I took the chat path, but it was not sent.";
      if (r.path === "task") {
        const how = r.started ? `ran it (${r.outcome})` : r.outcome ? `${r.outcome}, so it did not run` : "offered it, and it has not run";
        return `I took the task path: I matched your saved task "${r.task}" and ${how}. Nothing went to chat.`;
      }
      if (r.path === "learn") return `I took the learn path: I have no recording for that, so I offered to learn it${r.recording ? " and started recording" : ""}. Nothing went to chat.`;
      return "I looked for a saved task and found no match, so I answered here. Nothing went to chat.";
    }

    async function ask(given) {
      const text = (given !== undefined ? given : ui.input.value).trim();
      if (!text || busy) return false;
      const clear = () => { if (given === undefined) ui.input.value = ""; };
      say("");
      if (flow) { clear(); return continueFlow(text); }
      if (R.isWhatDidYouDo(text)) { clear(); local(text, whatDidYouDo()); return true; }
      let tasks = [];
      try { tasks = (await api.tasks()) || []; } catch (e) { tasks = []; }
      const r = R.route(text, tasks);
      if (r.path === "task") { clear(); offerTask(r.task, r.explicit, text); return true; }
      if (r.path === "none") {
        clear();
        lastRequest = { path: "none" };
        local(text, r.reason === "ambiguous" ? "Two saved tasks have that name. Rename one so I can tell them apart." : `I have no saved task called "${r.target}".${tasks.length ? " Saved tasks: " + tasks.map((k) => k.name).join(", ") + "." : " None are saved yet."}`);
        return true;
      }
      if (r.path === "learn") {
        clear();
        flow = { kind: "offer_learn", text };
        lastRequest = { path: "learn" };
        local(text, "I do not have a recording for that yet. Do it once and I will learn it? Say yes, or no to ask chat instead.");
        return true;
      }
      const ok = await chatPath(text);
      if (ok) clear();
      return ok;
    }

    async function sendDoc() {
      if (!pendingDoc) return false;
      const d = pendingDoc;
      const ok = await turn(SUMMARY_PROMPT(d.name, d.text), `Summarise: ${d.name}`);
      if (ok) pendingDoc = null;
      return ok;
    }

    return {
      ask: () => ask(),
      sendDoc,
      /** A file dropped on the canvas. Extraction happens on this machine; then a summary is asked for. */
      async drop(paths) {
        if (busy || !paths || !paths.length) return false;
        if (paths.length > 1) say("Drop one document at a time.", true);
        const r = await api.attach(paths[0]);
        if (!r.ok) {
          say(r.message, true);
          return false;
        }
        pendingDoc = { name: r.name, text: r.text };
        say("");
        const ok = await sendDoc();
        if (!ok && pendingDoc && !classification()) say(`Ready to summarise ${r.name}. Choose a classification and I will send it.`, false);
        return ok;
      },
      /** The classification changed. A waiting document goes now. */
      async classificationChanged() {
        say("");
        if (pendingDoc && classification()) return sendDoc();
        return false;
      },
      state: () => ({ flow: flow && flow.kind, lastRequest: lastRequest && { ...lastRequest }, messages, turns: turns.map((t) => ({ ...t })), pendingDoc: !!pendingDoc, busy }),
    };
  }

  if (typeof module !== "undefined" && module.exports) module.exports = { createChat, SUMMARY_PROMPT };
  else {
    root.addEventListener("DOMContentLoaded", () => {
      const tauri = root.__TAURI__;
      const invoke = tauri.core.invoke;
      const d = root.document;
      const ui = { input: d.getElementById("ask"), select: d.getElementById("classification"), log: d.getElementById("log"), status: d.getElementById("status") };
      const api = {
        expand: () => invoke("expand_canvas"),
        async send(classification, messages, onChunk) {
          const ch = new tauri.core.Channel();
          ch.onmessage = (m) => m && typeof m.text === "string" && onChunk(m.text);
          try {
            await invoke("chat_send", { classification, messages, onEvent: ch });
            return { ok: true };
          } catch (e) {
            return { ok: false, error: e && typeof e === "object" ? e : { message: String(e) } };
          }
        },
        async tasks() {
          return invoke("task_catalog");
        },
        async replay(task, inputs, officer) {
          try {
            const r = await invoke("replay_start", { taskId: task.id, version: task.version, inputs, officer, pid: null });
            return { ok: true, outcome: r && r.outcome };
          } catch (e) {
            return { ok: false, message: typeof e === "string" ? e : "The task could not be started." };
          }
        },
        async record(bundle) {
          try {
            await invoke("record_start", { appBundle: bundle });
            return { ok: true };
          } catch (e) {
            return { ok: false, message: typeof e === "string" ? e : "Recording could not start." };
          }
        },
        async attach(path) {
          try {
            const r = await invoke("chat_attach", { path });
            return { ok: true, name: r.name, text: r.text };
          } catch (e) {
            return { ok: false, message: String(e) };
          }
        },
      };
      const chat = createChat(d, api, ui);
      ui.input.addEventListener("keydown", (e) => {
        if (e.key === "Enter" && !e.isComposing) chat.ask();
      });
      ui.select.addEventListener("change", () => chat.classificationChanged());
      tauri.event.listen("tauri://drag-drop", (e) => chat.drop(e.payload && e.payload.paths));
    });
  }
})(typeof window !== "undefined" ? window : globalThis);
