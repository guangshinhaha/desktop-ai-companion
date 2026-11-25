//! Runs a replay through the executor and holds the approval gate on the shell's side (TD 7.1, 7.2).
//!
//! The executor is started with `serve` and given `replay.start`. While it runs it sends two kinds of
//! request and waits for each answer:
//!   `approval.request {card}`  -> the officer decides; the answer is `{approved: bool}`.
//!   `audit.append {event, step?, shown?, item?}` -> written through the store's API; the answer is `{seq}`.
//! The officer, the task reference, the run id and the time are the shell's: the executor cannot set them.
//! No answer is never a yes: if the executor ends, or the officer's window closes, the run stops.
//! Nothing here logs, prints or puts a value in an error.

use crate::store::{AuditEntry, Store};
use serde_json::{json, Value};
use std::io::{BufRead, Write};
use std::path::Path;
use std::process::{Command, Stdio};

/// Events the executor may append. The shell's own events (`task_saved` and the rest) are not among them.
pub const EXECUTOR_EVENTS: [&str; 6] =
    ["run_started", "approved", "rejected", "step_diverged", "recovered", "run_finished"];

/// Longest `shown` or `step` text taken from the executor.
const MAX_FIELD: usize = 16 * 1024;

pub struct RunContext {
    pub officer: String,
    /// `<task id>@<version>`
    pub task_ref: String,
    pub run: String,
}

/// An officer name the shell will put in the trail: 1 to 80 characters, no control characters.
pub fn valid_officer(s: &str) -> bool {
    let n = s.chars().count();
    let hidden = |c: char| matches!(c, '\u{061C}' | '\u{200B}'..='\u{200F}' | '\u{202A}'..='\u{202E}' | '\u{2060}'..='\u{2069}' | '\u{FEFF}');
    (1..=80).contains(&n) && !s.chars().any(|c| c.is_control() || hidden(c)) && !s.trim().is_empty()
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

fn reply(w: &mut impl Write, id: &Value, result: Value) -> Result<(), String> {
    let line = json!({ "jsonrpc": "2.0", "id": id, "result": result }).to_string();
    writeln!(w, "{line}").and_then(|_| w.flush()).map_err(|_| "the executor is gone".to_string())
}

fn reply_err(w: &mut impl Write, id: &Value, code: i64, msg: &str) -> Result<(), String> {
    let line = json!({ "jsonrpc": "2.0", "id": id, "error": { "code": code, "message": msg } }).to_string();
    writeln!(w, "{line}").and_then(|_| w.flush()).map_err(|_| "the executor is gone".to_string())
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
                let entry = AuditEntry {
                    at: now(),
                    officer: &ctx.officer,
                    event,
                    task: Some(&ctx.task_ref),
                    run: Some(&ctx.run),
                    item: params["item"].as_i64(),
                    step: step.as_deref(),
                    shown: shown.as_deref(),
                };
                match store.append_event(&entry) {
                    Ok(seq) => reply(writer, &id, json!({ "seq": seq }))?,
                    // The error names a kind, never a value. The executor stops the run on it.
                    Err(_) => reply_err(writer, &id, -32000, "the audit trail could not be written")?,
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
                let approved = decide(&card);
                reply(writer, &id, json!({ "approved": approved }))?;
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
    let mut child = Command::new(bin)
        .arg("serve")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|e| format!("cannot start {}: {}", bin.display(), e.kind()))?;
    let mut stdin = child.stdin.take().ok_or("no stdin")?;
    let stdout = child.stdout.take().ok_or("no stdout")?;
    let now = || {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs() as i64)
            .unwrap_or(0)
    };
    let result = (|| {
        writeln!(stdin, "{}", start_request(1, task, inputs, pid)).map_err(|_| "the executor is gone".to_string())?;
        stdin.flush().map_err(|_| "the executor is gone".to_string())?;
        drive(std::io::BufReader::new(stdout), &mut stdin, store, ctx, 1, &now, decide)
    })();
    drop(stdin);
    if result.is_err() {
        let _ = child.kill();
    }
    let _ = child.wait();
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::Fixed;
    use std::io::Cursor;

    fn store() -> (Store, std::path::PathBuf) {
        let n = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos();
        let dir = std::env::temp_dir().join(format!("companion-approval-{}-{n}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        (Store::open(&dir.join("s.db"), &Fixed([7u8; 32])).unwrap(), dir)
    }

    fn ctx() -> RunContext {
        RunContext { officer: "officer-1".into(), task_ref: "tsk_x@1".into(), run: "run_1".into() }
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
            + &mk("x2", "approved", json!({"step":"s5","shown":"Create a staff account for Test Person (test@example.test)"}))
            + &mk("x3", "run_finished", json!({"shown":"finished"}))
            + &line(json!({"jsonrpc":"2.0","id":1,"result":{"outcome":"finished"}}));
        let mut out = Vec::new();
        drive(Cursor::new(input), &mut out, &mut s, &ctx(), 1, &|| 1_700_000_000, &mut |_| false).unwrap();
        let rs = replies(&out);
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

    #[test]
    fn officer_names_cannot_carry_hidden_direction_marks() {
        assert!(!valid_officer("Test \u{202E}Officer"));
        assert!(!valid_officer("Test\u{200B}Officer"));
    }
}
