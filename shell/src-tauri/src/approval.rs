//! Runs a replay through the executor and holds the approval gate on the shell's side (TD 7.1, 7.2).
//!
//! The executor is started with `serve` and given `replay.start`. While it runs it sends two kinds of
//! request and waits for each answer:
//!   `approval.request {card}`  -> the officer decides; the answer is `{approved: bool}`.
//!   `audit.append {event, step?, shown?, item?}` -> written through the store's API; the answer is `{seq}`.
//!   `recover.request {request}` -> passed to the companion service's `POST /recover` (#34); the answer is the
//!   service's answer object. Only a masked request (`values: "masked"`) goes: the shell never clears values.
//!   No service, a refusal or a bad reply is an error, and the executor then asks the officer.
//! The officer, the task reference, the run id and the time are the shell's: the executor cannot set them.
//! No answer is never a yes: if the executor ends, or the officer's window closes, the run stops.
//! Nothing here logs, prints or puts a value in an error.

use crate::progress::{RunHandle, SharedWriter, STOP_GRACE};
use crate::store::{AuditEntry, Store};
use serde_json::{json, Value};
use std::io::{BufRead, Write};
use std::path::Path;
use std::process::{Command, Stdio};
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex};
use std::time::Duration;

/// Events the executor may append. The shell's own events (`task_saved` and the rest) are not among them.
pub const EXECUTOR_EVENTS: [&str; 7] =
    ["run_started", "approved", "rejected", "step_diverged", "run_stopped", "recovered", "run_finished"];

/// Longest `shown` or `step` text taken from the executor.
const MAX_FIELD: usize = 16 * 1024;

pub struct RunContext {
    pub officer: String,
    /// `<task id>@<version>`
    pub task_ref: String,
    pub run: String,
    /// The row of a list run. The shell's word: it overrides any `item` the executor sends.
    pub item: Option<i64>,
}

/// A list run's row bookkeeping, called by `drive_with` at the points where the row's state must change.
pub trait ItemObserver {
    /// An approval card is about to be shown.
    fn approval_asked(&mut self, _store: &mut Store) {}
    /// The executor's `approved` is about to be recorded, and it acts right after. Return false to refuse:
    /// the executor is told the record failed and does not act.
    fn commit_begins(&mut self, _store: &mut Store) -> bool {
        true
    }
    /// The `approved` record could not be written, so the executor will not act.
    fn commit_aborted(&mut self, _store: &mut Store) {}
}

pub struct NoObserver;
impl ItemObserver for NoObserver {}

/// An officer name the shell will put in the trail: 1 to 80 characters, no control characters.
pub fn valid_officer(s: &str) -> bool {
    let n = s.chars().count();
    (1..=80).contains(&n) && !s.chars().any(|c| c.is_control() || hidden_char(c)) && !s.trim().is_empty()
}

/// Zero-width and text-direction characters: invisible, or they make text read as something it is not.
pub fn hidden_char(c: char) -> bool {
    matches!(c, '\u{061C}' | '\u{200B}'..='\u{200F}' | '\u{202A}'..='\u{202E}' | '\u{2060}'..='\u{2069}' | '\u{FEFF}')
}

/// A text field of an `audit.append`: absent or null is fine; anything else must be a string within the
/// limit. A field that is too long or the wrong type refuses the append, so a record is never kept with
/// its content silently dropped.
fn text_field(params: &Value, k: &str) -> Result<Option<String>, ()> {
    match &params[k] {
        Value::Null => Ok(None),
        Value::String(s) if s.len() <= MAX_FIELD => Ok(Some(s.clone())),
        _ => Err(()),
    }
}

pub fn start_request(id: u64, task: &Value, inputs: &Value, pid: Option<i64>) -> String {
    let mut params = json!({ "task": task, "inputs": inputs });
    if let Some(p) = pid {
        params["pid"] = json!(p);
    }
    json!({ "jsonrpc": "2.0", "id": id, "method": "replay.start", "params": params }).to_string()
}

/// One `write_all` per line: the officer's stop can share this writer, and a line is never split by it.
fn send_line(w: &mut impl Write, line: String) -> Result<(), String> {
    w.write_all(format!("{line}\n").as_bytes()).and_then(|_| w.flush()).map_err(|_| "the executor is gone".to_string())
}

fn reply(w: &mut impl Write, id: &Value, result: Value) -> Result<(), String> {
    send_line(w, json!({ "jsonrpc": "2.0", "id": id, "result": result }).to_string())
}

fn reply_err(w: &mut impl Write, id: &Value, code: i64, msg: &str) -> Result<(), String> {
    send_line(w, json!({ "jsonrpc": "2.0", "id": id, "error": { "code": code, "message": msg } }).to_string())
}

/// What the shell saw of a run beyond its result: for the progress view, and for knowing whether the
/// trail already holds the stop.
#[derive(Default)]
pub struct Observed {
    pub done: Vec<String>,
    pub stop_recorded: bool,
    pub finish_recorded: bool,
    /// The step that had started and not finished when the last notification came.
    pub in_flight: Option<String>,
}

/// Serves the executor's requests until it answers `replay.start` (`start_id`). Returns that result.
/// `decide` is given the card (with the officer and run added) and returns the officer's decision;
/// it may block for as long as the officer takes. `now` supplies the record time.
pub fn drive<R: BufRead, W: Write>(
    reader: R,
    writer: &mut W,
    store: &mut Store,
    ctx: &RunContext,
    start_id: u64,
    now: &dyn Fn() -> i64,
    decide: &mut dyn FnMut(&Value) -> bool,
) -> Result<Value, String> {
    drive_with(reader, writer, store, ctx, start_id, now, decide, &mut |_| {}, &mut Observed::default(), &mut NoObserver, &mut |_| None)
}

/// A recovery request the shell will pass on: an object of kind `recover` with its values masked.
fn recovery_request_ok(r: &Value) -> bool {
    r.is_object() && r["kind"] == "recover" && r["values"] == "masked"
}

/// `drive`, also passing each `run.progress` notification to `on_progress` and noting what it saw.
/// A notification has no id and gets no reply. `recover` sends a recovery request to the service and returns its
/// answer, or None.
#[allow(clippy::too_many_arguments)]
pub fn drive_with<R: BufRead, W: Write>(
    reader: R,
    writer: &mut W,
    store: &mut Store,
    ctx: &RunContext,
    start_id: u64,
    now: &dyn Fn() -> i64,
    decide: &mut dyn FnMut(&Value) -> bool,
    on_progress: &mut dyn FnMut(&Value),
    seen: &mut Observed,
    obs: &mut dyn ItemObserver,
    recover: &mut dyn FnMut(&Value) -> Option<Value>,
) -> Result<Value, String> {
    for line in reader.lines() {
        let line = line.map_err(|_| "cannot read the executor's output".to_string())?;
        if line.trim().is_empty() {
            continue;
        }
        let Ok(msg) = serde_json::from_str::<Value>(&line) else { continue };
        let id = msg.get("id").cloned().unwrap_or(Value::Null);
        let Some(method) = msg.get("method").and_then(Value::as_str) else {
            // An answer. Only the one to replay.start ends the run.
            if id == json!(start_id) {
                return match msg.get("error") {
                    Some(e) => Err(format!("the executor refused the run: {}", e["message"].as_str().unwrap_or("no reason"))),
                    None => Ok(msg["result"].clone()),
                };
            }
            continue;
        };
        let params = &msg["params"];
        match method {
            "run.progress" => {
                if params["state"] == "done" {
                    if let Some(s) = params["step"].as_str() {
                        seen.done.push(s.to_string());
                        if seen.in_flight.as_deref() == Some(s) {
                            seen.in_flight = None;
                        }
                    }
                } else if params["state"] == "started" {
                    seen.in_flight = params["step"].as_str().map(str::to_string);
                }
                on_progress(params);
            }
            "audit.append" => {
                let event = params["event"].as_str().unwrap_or("");
                if !EXECUTOR_EVENTS.contains(&event) {
                    reply_err(writer, &id, -32602, "not an event the executor may record")?;
                    continue;
                }
                let (Ok(step), Ok(shown)) = (text_field(params, "step"), text_field(params, "shown")) else {
                    reply_err(writer, &id, -32602, "a field is too long or not text")?;
                    continue;
                };
                let begins_commit = event == "approved";
                if begins_commit && !obs.commit_begins(store) {
                    reply_err(writer, &id, -32000, "the row could not be marked as committing")?;
                    continue;
                }
                let entry = AuditEntry {
                    at: now(),
                    officer: &ctx.officer,
                    event,
                    task: Some(&ctx.task_ref),
                    run: Some(&ctx.run),
                    item: ctx.item.or_else(|| params["item"].as_i64()),
                    step: step.as_deref(),
                    shown: shown.as_deref(),
                };
                match store.append_event(&entry) {
                    Ok(seq) => {
                        seen.stop_recorded |= event == "run_stopped";
                        seen.finish_recorded |= event == "run_finished";
                        reply(writer, &id, json!({ "seq": seq }))?
                    }
                    // The error names a kind, never a value. The executor stops the run on it.
                    Err(_) => {
                        if begins_commit {
                            obs.commit_aborted(store);
                        }
                        reply_err(writer, &id, -32000, "the audit trail could not be written")?
                    }
                }
            }
            "approval.request" => {
                let mut card = params["card"].clone();
                if !card.is_object() {
                    reply(writer, &id, json!({ "approved": false }))?;
                    continue;
                }
                card["officer"] = json!(ctx.officer);
                card["run"] = json!(ctx.run);
                // A list run's card says which row it is for.
                if let Some(row) = ctx.item {
                    card["row"] = json!(row);
                }
                obs.approval_asked(store);
                let approved = decide(&card);
                reply(writer, &id, json!({ "approved": approved }))?;
            }
            "recover.request" => {
                let request = &params["request"];
                if !recovery_request_ok(request) {
                    reply_err(writer, &id, -32602, "not a masked recovery request")?;
                    continue;
                }
                match recover(request) {
                    Some(answer) => reply(writer, &id, answer)?,
                    None => reply_err(writer, &id, -32000, "no recovery answer")?,
                }
            }
            _ => reply_err(writer, &id, -32601, "Method not found")?,
        }
    }
    Err("the executor ended before the run finished".into())
}

/// Starts the executor, runs the task through it, and shuts it down. `inputs` stay in memory: they go to
/// the executor over its stdin and nowhere else.
pub fn run_replay(
    bin: &Path,
    store: &mut Store,
    ctx: &RunContext,
    task: &Value,
    inputs: &Value,
    pid: Option<i64>,
    decide: &mut dyn FnMut(&Value) -> bool,
) -> Result<Value, String> {
    let mut hooks = Hooks { on_start: &mut |_| {}, on_progress: &mut |_| {}, stop_grace: STOP_GRACE };
    run_replay_with(bin, store, ctx, task, inputs, pid, decide, &mut hooks)
}

/// The shell's ears and hands on a run: `on_start` is given the handle that pauses, resumes and stops it, as
/// soon as the executor is running; `on_progress` gets each progress notification.
pub struct Hooks<'a> {
    pub on_start: &'a mut dyn FnMut(RunHandle),
    pub on_progress: &'a mut dyn FnMut(&Value),
    pub stop_grace: Duration,
}

/// Records a stop the executor could not record, because the officer stopped the run and the executor had
/// to be ended: `run_stopped` (if the executor had not written it) and `run_finished`. Step ids only.
pub fn record_forced_stop(
    store: &mut Store,
    ctx: &RunContext,
    task: &Value,
    seen: &Observed,
    at: i64,
) -> Result<(), String> {
    let all: Vec<String> = task["steps"]
        .as_array()
        .map(|a| a.iter().filter_map(|s| s["id"].as_str().map(str::to_string)).collect())
        .unwrap_or_default();
    let flying = seen.in_flight.as_deref();
    let not_run: Vec<&str> = all.iter().filter(|s| !seen.done.contains(s) && Some(s.as_str()) != flying).map(String::as_str).collect();
    let list = |v: &[&str]| if v.is_empty() { "none".to_string() } else { v.join(", ") };
    let done: Vec<&str> = seen.done.iter().map(String::as_str).collect();
    let mut shown = format!(
        "Stopped by the officer. The executor did not end the run in time and was ended. Completed: {}. Not run: {}.",
        list(&done),
        list(&not_run)
    );
    // The step in flight may have been acted on: the record does not call it not run.
    if let Some(s) = flying {
        shown.push_str(&format!(" In flight when it was ended: {s} (whether its action was taken is unknown)."));
    }
    let mut rows: Vec<(&str, &str)> = Vec::new();
    if !seen.stop_recorded {
        rows.push(("run_stopped", shown.as_str()));
    }
    if !seen.finish_recorded {
        rows.push(("run_finished", "stopped"));
    }
    for (event, text) in rows {
        let e = AuditEntry {
            at,
            officer: &ctx.officer,
            event,
            task: Some(&ctx.task_ref),
            run: Some(&ctx.run),
            item: None,
            step: None,
            shown: Some(text),
        };
        store.append_event(&e).map_err(|_| "the stop could not be written to the audit trail".to_string())?;
    }
    Ok(())
}

/// `run_replay` with the officer's controls: see `Hooks`.
#[allow(clippy::too_many_arguments)]
pub fn run_replay_with(
    bin: &Path,
    store: &mut Store,
    ctx: &RunContext,
    task: &Value,
    inputs: &Value,
    pid: Option<i64>,
    decide: &mut dyn FnMut(&Value) -> bool,
    hooks: &mut Hooks,
) -> Result<Value, String> {
    run_replay_observed(bin, store, ctx, task, inputs, pid, decide, hooks, &mut NoObserver)
}

/// `run_replay_with`, also telling a list run's `ItemObserver` where the row stands.
#[allow(clippy::too_many_arguments)]
pub fn run_replay_observed(
    bin: &Path,
    store: &mut Store,
    ctx: &RunContext,
    task: &Value,
    inputs: &Value,
    pid: Option<i64>,
    decide: &mut dyn FnMut(&Value) -> bool,
    hooks: &mut Hooks,
    obs: &mut dyn ItemObserver,
) -> Result<Value, String> {
    let mut child = Command::new(bin)
        .arg("serve")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|e| format!("cannot start {}: {}", bin.display(), e.kind()))?;
    let stdin = child.stdin.take().ok_or("no stdin")?;
    let stdout = child.stdout.take().ok_or("no stdout")?;
    let mut writer = SharedWriter::new(stdin);
    let child = Arc::new(Mutex::new(Some(child)));
    let ended = Arc::new(AtomicBool::new(false));
    let handle = RunHandle {
        writer: writer.clone(),
        stop_asked: Arc::new(AtomicBool::new(false)),
        ended: ended.clone(),
        kill: {
            let child = child.clone();
            Arc::new(move || {
                if let Ok(mut g) = child.lock() {
                    if let Some(c) = g.as_mut() {
                        let _ = c.kill();
                    }
                }
            })
        },
        grace: hooks.stop_grace,
    };
    let mut seen = Observed::default();
    let now = || {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs() as i64)
            .unwrap_or(0)
    };
    let result = (|| {
        send_line(&mut writer, start_request(1, task, inputs, pid))?;
        // The controls are handed over only now. A stop pressed before this is sent after `replay.start`
        // (the executor drops a control line that arrives before it), so it still stops the run.
        (hooks.on_start)(handle.clone());
        // A card is never put up once the officer has stopped the run.
        let mut guarded = |card: &Value| if handle.stop_asked() { false } else { decide(card) };
        // Recovery goes to the companion service, if one is set up; otherwise the executor asks the officer.
        let service = crate::chat::HttpService::from_env().ok();
        let mut recover = |r: &Value| service.as_ref().and_then(|s| s.recover(r));
        drive_with(std::io::BufReader::new(stdout), &mut writer, store, ctx, 1, &now, &mut guarded, &mut *hooks.on_progress, &mut seen, obs, &mut recover)
    })();
    // Closing our end tells the executor to exit; an error ends it now.
    let finished = child.lock().ok().and_then(|mut g| g.take());
    writer.close();
    if let Some(mut c) = finished {
        if result.is_err() {
            let _ = c.kill();
        }
        let _ = c.wait();
    }
    ended.store(true, std::sync::atomic::Ordering::SeqCst);
    if result.is_err() && handle.stop_asked() {
        // The officer stopped it and the executor could not say so itself. The trail still says it.
        if let Err(e) = record_forced_stop(store, ctx, task, &seen, now()) {
            return Err(e);
        }
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::Fixed;
    use std::io::Cursor;

    fn store() -> (Store, std::path::PathBuf) {
        let n = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos();
        // A counter as well as the clock: parallel tests can read the same clock tick and share a directory.
        static N: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
        let k = N.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!("companion-approval-{}-{n}-{k}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        (Store::open(&dir.join("s.db"), &Fixed([7u8; 32])).unwrap(), dir)
    }

    fn ctx() -> RunContext {
        RunContext { officer: "officer-1".into(), task_ref: "tsk_x@1".into(), run: "run_1".into(), item: None }
    }

    fn line(v: Value) -> String {
        format!("{v}\n")
    }

    fn replies(w: &[u8]) -> Vec<Value> {
        String::from_utf8_lossy(w).lines().map(|l| serde_json::from_str(l).unwrap()).collect()
    }

    #[test]
    fn officer_names_are_checked() {
        assert!(valid_officer("Test Officer"));
        assert!(!valid_officer(""));
        assert!(!valid_officer("  "));
        assert!(!valid_officer("a\u{1b}[2Kb"));
        assert!(!valid_officer(&"x".repeat(81)));
    }

    #[test]
    fn an_approval_waits_for_the_officer_and_only_a_true_answer_is_a_yes() {
        let (mut s, _d) = store();
        for decision in [true, false] {
            let input = line(json!({"jsonrpc":"2.0","id":"x1","method":"approval.request","params":{"card":{"step":"s5","summary":"Create a staff account for Test Person"}}}))
                + &line(json!({"jsonrpc":"2.0","id":1,"result":{"outcome":"finished"}}));
            let mut out = Vec::new();
            let mut seen = None;
            let r = drive(Cursor::new(input), &mut out, &mut s, &ctx(), 1, &|| 5, &mut |card| {
                seen = Some(card.clone());
                decision
            })
            .unwrap();
            assert_eq!(r["outcome"], "finished");
            let card = seen.unwrap();
            // The officer and run on the card are the shell's.
            assert_eq!(card["officer"], "officer-1");
            assert_eq!(card["run"], "run_1");
            assert_eq!(replies(&out)[0]["result"]["approved"], decision);
        }
    }

    #[test]
    fn the_executor_ending_is_an_error_never_an_approval() {
        let (mut s, _d) = store();
        let input = line(json!({"jsonrpc":"2.0","id":"x1","method":"approval.request","params":{"card":{"step":"s5"}}}));
        let mut out = Vec::new();
        let mut asked = 0;
        let r = drive(Cursor::new(input), &mut out, &mut s, &ctx(), 1, &|| 5, &mut |_| {
            asked += 1;
            false
        });
        assert!(r.is_err());
        assert_eq!(asked, 1);
    }

    #[test]
    fn audit_events_are_appended_with_the_shells_identity_and_the_chain_verifies() {
        let (mut s, _d) = store();
        let mk = |id: &str, ev: &str, extra: Value| {
            let mut p = json!({ "event": ev, "officer": "evil", "task": "other@9", "run": "run_9", "at": 1 });
            for (k, v) in extra.as_object().unwrap() {
                p[k] = v.clone();
            }
            line(json!({"jsonrpc":"2.0","id":id,"method":"audit.append","params":p}))
        };
        let input = mk("x1", "run_started", json!({}))
            + &card_line("s5")
            + &mk("x2", "approved", json!({"step":"s5","shown":"Create a staff account for Test Person (test@example.test)"}))
            + &mk("x3", "run_finished", json!({"shown":"finished"}))
            + &line(json!({"jsonrpc":"2.0","id":1,"result":{"outcome":"finished"}}));
        let mut out = Vec::new();
        drive(Cursor::new(input), &mut out, &mut s, &ctx(), 1, &|| 1_700_000_000, &mut |_| true).unwrap();
        let mut rs = replies(&out);
        assert_eq!(rs.remove(1)["result"]["approved"], true);
        assert_eq!(rs.iter().map(|r| r["result"]["seq"].as_i64().unwrap()).collect::<Vec<_>>(), vec![1, 2, 3]);
        s.verify_audit().unwrap();
        let export = s.export_audit().unwrap();
        let recs: Vec<Value> = export.lines().skip(1).map(|l| serde_json::from_str(l).unwrap()).collect();
        assert_eq!(recs.len(), 3);
        for r in &recs {
            assert_eq!(r["officer"], "officer-1", "the executor cannot name the officer");
            assert_eq!(r["task"], "tsk_x@1");
            assert_eq!(r["run"], "run_1");
            assert_eq!(r["at"], 1_700_000_000);
        }
        assert_eq!(recs[1]["event"], "approved");
        assert!(recs[1]["shown"].as_str().unwrap().contains("Test Person"), "decision 0005: the approved values are kept");
    }

    #[test]
    fn the_executor_cannot_append_the_shells_own_events_or_unknown_ones() {
        let (mut s, _d) = store();
        let input = ["task_saved", "audit_purged", "made_up", ""]
            .iter()
            .enumerate()
            .map(|(i, ev)| line(json!({"jsonrpc":"2.0","id":format!("x{i}"),"method":"audit.append","params":{"event":ev}})))
            .collect::<String>()
            + &line(json!({"jsonrpc":"2.0","id":1,"result":{}}));
        let mut out = Vec::new();
        drive(Cursor::new(input), &mut out, &mut s, &ctx(), 1, &|| 1, &mut |_| false).unwrap();
        assert!(replies(&out).iter().all(|r| r.get("error").is_some()));
        assert_eq!(s.audit_count().unwrap(), 0);
    }

    #[test]
    fn an_unknown_method_is_refused_and_a_refused_run_is_an_error() {
        let (mut s, _d) = store();
        let input = line(json!({"jsonrpc":"2.0","id":"x1","method":"fs.read","params":{}}))
            + &line(json!({"jsonrpc":"2.0","id":1,"error":{"code":-32602,"message":"task not usable"}}));
        let mut out = Vec::new();
        let r = drive(Cursor::new(input), &mut out, &mut s, &ctx(), 1, &|| 1, &mut |_| true);
        assert_eq!(replies(&out)[0]["error"]["code"], -32601);
        assert!(r.unwrap_err().contains("refused"));
    }

    fn drive_recovering(input: String, recover: &mut dyn FnMut(&Value) -> Option<Value>) -> (Vec<Value>, Result<Value, String>) {
        let (mut s, _d) = store();
        let mut out = Vec::new();
        let r = drive_with(Cursor::new(input), &mut out, &mut s, &ctx(), 1, &|| 1, &mut |_| true, &mut |_| {}, &mut Observed::default(), &mut NoObserver, recover);
        (replies(&out), r)
    }

    fn recover_line(request: Value) -> String {
        line(json!({"jsonrpc":"2.0","id":"x1","method":"recover.request","params":{"request":request}}))
            + &line(json!({"jsonrpc":"2.0","id":1,"result":{"outcome":"finished"}}))
    }

    #[test]
    fn a_recovery_request_goes_to_the_service_and_its_answer_back_to_the_executor() {
        let req = json!({"kind":"recover","classification":"official-open","values":"masked","step":{"id":"s2"},"tree":{"ref":"n1"}});
        let mut sent = Vec::new();
        let (out, r) = drive_recovering(recover_line(req.clone()), &mut |b| { sent.push(b.clone()); Some(json!({"answer":"retry"})) });
        assert_eq!(sent, vec![req]);
        assert_eq!(out[0]["id"], "x1");
        assert_eq!(out[0]["result"], json!({"answer":"retry"}));
        assert!(r.is_ok());
    }

    #[test]
    fn no_answer_from_the_service_is_an_error_never_an_answer() {
        let req = json!({"kind":"recover","classification":"official-open","values":"masked","step":{},"tree":{}});
        let (out, _) = drive_recovering(recover_line(req), &mut |_| None);
        assert!(out[0].get("result").is_none());
        assert!(out[0]["error"].is_object());
    }

    #[test]
    fn a_request_that_is_not_a_masked_recovery_request_is_never_sent() {
        // The shell never clears values for a route, so a request with values included is refused here.
        for req in [json!(null), json!("x"), json!({"kind":"chat","values":"masked"}), json!({"kind":"recover","values":"included"}), json!({"kind":"recover"})] {
            let mut called = false;
            let (out, _) = drive_recovering(recover_line(req.clone()), &mut |_| { called = true; Some(json!({"answer":"retry"})) });
            assert!(!called, "{req}");
            assert!(out[0]["error"].is_object(), "{req}");
        }
    }

    #[test]
    fn drive_without_a_service_refuses_recovery() {
        let (mut s, _d) = store();
        let mut out = Vec::new();
        let input = recover_line(json!({"kind":"recover","values":"masked"}));
        drive(Cursor::new(input), &mut out, &mut s, &ctx(), 1, &|| 1, &mut |_| true).unwrap();
        assert!(replies(&out)[0]["error"].is_object());
    }

    #[test]
    fn an_oversize_or_mistyped_field_refuses_the_append_rather_than_dropping_it() {
        let (mut s, _d) = store();
        let big = "x".repeat(MAX_FIELD + 1);
        let input = line(json!({"jsonrpc":"2.0","id":"x1","method":"audit.append","params":{"event":"approved","shown":big}}))
            + &line(json!({"jsonrpc":"2.0","id":"x2","method":"audit.append","params":{"event":"approved","shown":7}}))
            + &line(json!({"jsonrpc":"2.0","id":1,"result":{"outcome":"finished"}}));
        let mut out = Vec::new();
        drive(Cursor::new(input), &mut out, &mut s, &ctx(), 1, &|| 1, &mut |_| true).unwrap();
        let r = replies(&out);
        assert!(r[0]["error"].is_object() && r[1]["error"].is_object());
        assert_eq!(s.audit_count().unwrap(), 0);
    }

    fn card_line(step: &str) -> String {
        line(json!({"jsonrpc":"2.0","id":"c","method":"approval.request","params":{"card":{"step":step}}}))
    }

    fn approved_line(step: &str) -> String {
        line(json!({"jsonrpc":"2.0","id":"a","method":"audit.append","params":{"event":"approved","step":step,"shown":"synthetic"}}))
    }

    #[derive(Default)]
    struct Commits(u32);
    impl ItemObserver for Commits {
        fn commit_begins(&mut self, _store: &mut Store) -> bool {
            self.0 += 1;
            true
        }
    }

    /// Drives `input`, answering the cards with `answers` in order. Returns the replies to the `approved`
    /// records (true: accepted), the records written and how often the row was told a commit begins.
    fn gate(input: String, answers: &[bool]) -> (Vec<bool>, i64, u32) {
        let (mut s, _d) = store();
        let mut out = Vec::new();
        let mut answers = answers.iter();
        let mut commits = Commits::default();
        let input = input + &line(json!({"jsonrpc":"2.0","id":1,"result":{}}));
        drive_with(Cursor::new(input), &mut out, &mut s, &ctx(), 1, &|| 1, &mut |_| *answers.next().unwrap(), &mut |_| {}, &mut Observed::default(), &mut commits, &mut |_| None).unwrap();
        let accepted = replies(&out).iter().filter(|r| r["id"] == "a").map(|r| r.get("error").is_none()).collect();
        (accepted, s.audit_count().unwrap() as i64, commits.0)
    }

    #[test]
    fn approved_after_a_no_is_refused() {
        assert_eq!(gate(card_line("s5") + &approved_line("s5"), &[false]), (vec![false], 0, 0));
    }

    #[test]
    fn approved_with_no_card_first_is_refused() {
        assert_eq!(gate(approved_line("s5"), &[]), (vec![false], 0, 0));
    }

    #[test]
    fn approved_for_another_step_than_the_yes_is_refused() {
        assert_eq!(gate(card_line("s4") + &approved_line("s5"), &[true]), (vec![false], 0, 0));
    }

    #[test]
    fn one_yes_covers_one_approved_record() {
        assert_eq!(gate(card_line("s5") + &approved_line("s5") + &approved_line("s5"), &[true]), (vec![true, false], 1, 1));
    }

    #[test]
    fn a_later_card_replaces_an_unused_yes() {
        let no = card_line("s4") + &card_line("s5") + &approved_line("s4");
        assert_eq!(gate(no, &[true, false]), (vec![false], 0, 0));
        let not_a_card = line(json!({"jsonrpc":"2.0","id":"c2","method":"approval.request","params":{"card":"s4"}}));
        assert_eq!(gate(card_line("s4") + &not_a_card + &approved_line("s4"), &[true]), (vec![false], 0, 0));
    }

    #[test]
    fn officer_names_cannot_carry_hidden_direction_marks() {
        assert!(!valid_officer("Test \u{202E}Officer"));
        assert!(!valid_officer("Test\u{200B}Officer"));
    }
}
