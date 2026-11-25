// Deterministic routing for the canvas (#32, TD 8). No model call, no network: the text is matched
// against saved task names and a few fixed phrases, on this machine. Anything not recognised is chat.
(function (root) {
  "use strict";

  const norm = (s) => String(s).toLowerCase().replace(/[^\p{L}\p{N}]+/gu, " ").trim();
  const words = (s) => ` ${norm(s)} `;

  // Verbs of operating an app by hand. Writing and reading verbs (draft, summarise, explain) are chat.
  const MANUAL_VERBS = ["create", "add", "update", "edit", "change", "submit", "enter", "fill", "copy", "paste", "register", "provision", "approve", "upload", "download", "export", "import", "book", "apply", "process", "delete", "remove", "transfer", "reset", "assign", "send out", "log", "key in", "set up"];
  const LEAD_INS = ["i need to", "i have to", "i want to", "i must", "can you", "could you", "please", "how do i", "help me"];

  /** "what did you do?" and close variants. */
  function isWhatDidYouDo(text) {
    const t = norm(text);
    return /^(what|which) (did|have) (you|i) (do|done|take|taken|pick|picked|choose|chosen)( with (that|this|it))?$/.test(t) ||
      /^(what|which) (path|route) did you (take|use|pick|choose)$/.test(t) ||
      /^what (just )?happened$/.test(t);
  }

  const YES = ["yes", "y", "yep", "ok", "okay", "sure", "go", "go ahead", "do it", "run it", "start"];
  const NO = ["no", "n", "nope", "cancel", "stop", "never mind", "nevermind", "not now", "chat"];
  const isYes = (t) => YES.includes(norm(t));
  const isNo = (t) => NO.includes(norm(t));

  /** The explicit command: "run <task>". Returns the rest, or null. */
  function runTarget(text) {
    const m = /^\s*run\s+(.+?)\s*$/i.exec(text);
    return m ? m[1] : null;
  }

  /**
   * Which saved task does the text name? A task matches when its whole name appears in the text as
   * whole words. The longest name wins. Two tasks with the same name are ambiguous: none is offered.
   * Returns { task } | { ambiguous: true } | null.
   */
  function matchTask(text, tasks) {
    const t = words(text);
    const hits = tasks.filter((k) => norm(k.name) && t.includes(words(k.name)));
    if (!hits.length) return null;
    const longest = Math.max(...hits.map((k) => norm(k.name).length));
    const top = hits.filter((k) => norm(k.name).length === longest);
    return top.length === 1 ? { task: top[0] } : { ambiguous: true };
  }

  /** Does the text describe operating an app by hand? Only when it opens with such a verb. */
  function looksManual(text) {
    let t = norm(text);
    for (let again = true; again; ) {
      again = false;
      for (const l of LEAD_INS) if (t.startsWith(l + " ")) { t = t.slice(l.length + 1); again = true; }
    }
    return MANUAL_VERBS.some((v) => t.startsWith(v + " "));
  }

  /**
   * One request, one path. Never calls anything.
   *   { path: "task", task, explicit }   a saved task named, or "run <task>"
   *   { path: "none", target, reason }   "run <x>" with no (or two) saved tasks of that name
   *   { path: "learn" }                  a manual task with no recording
   *   { path: "chat" }                   anything else
   */
  function route(text, tasks) {
    const target = runTarget(text);
    if (target !== null) {
      const m = matchTask(target, tasks);
      if (m && m.task) return { path: "task", task: m.task, explicit: true };
      return { path: "none", target, reason: m ? "ambiguous" : "unknown" };
    }
    const m = matchTask(text, tasks);
    if (m && m.task) return { path: "task", task: m.task, explicit: false };
    if (m) return { path: "none", target: text, reason: "ambiguous" };
    if (looksManual(text)) return { path: "learn" };
    return { path: "chat" };
  }

  const api = { norm, route, matchTask, looksManual, isWhatDidYouDo, isYes, isNo, runTarget };
  if (typeof module !== "undefined" && module.exports) module.exports = api;
  else root.CompanionRouter = api;
})(typeof window !== "undefined" ? window : globalThis);
