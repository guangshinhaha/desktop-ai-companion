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

  const SUMMARY_PROMPT = (name, text) => `Summarise this document in a few short paragraphs, then list any actions or deadlines it mentions.\n\nDocument: ${name}\n\n---\n${text}`;

  function createChat(doc, api, ui) {
    // ui: { input, select, log, status }
    let messages = []; // what the service has seen and answered
    let turns = []; // what the officer sees: { role, text }
    let pendingDoc = null; // a dropped document not yet sent
    let busy = false;
    let opened = false;

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

    async function ask() {
      const text = ui.input.value.trim();
      if (!text) return false;
      const ok = await turn(text, text);
      if (ok) ui.input.value = "";
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
      ask,
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
      state: () => ({ messages, turns: turns.map((t) => ({ ...t })), pendingDoc: !!pendingDoc, busy }),
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
