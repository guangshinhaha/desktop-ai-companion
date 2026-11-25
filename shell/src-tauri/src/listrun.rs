//! List runs (E-5, TD 6): one task, once per spreadsheet row, the officer approving each row.
//!
//! The guarantee is that no row is submitted twice, even across a crash. It rests on these:
//!  1. `committing` is written to `run_items` before the `approved` audit record, and the executor acts only
//!     after that record is written (see `approval::ItemObserver`). Only a row that is `awaiting_approval` can
//!     become `committing`, and only once: a second observer finds it `committing` and is refused.
//!  2. A row found `committing` when a run is picked up again is `unknown`, never retried by the shell. So is a
//!     row whose later step failed after an earlier step of the same row acted.
//!  3. A run will not continue while any row is `unknown` or `diverged`. Only the officer clears one, by saying
//!     what happened in the target app, and that is written to the audit trail.
//!  4. Every state change is a compare-and-set in the store, so a second caller cannot take a row the first holds.
//!  5. A run is held by one lease at a time (`Store::lease_run`). Picking a run up (`recover`), running it and
//!     the officer's answers all need the lease, so a crashed run is recovered only once its holder is gone and
//!     a live run's rows are never reset under it.
//!  6. Starting a run over a file and task that already have an unfinished run picks that run up instead of
//!     starting a second one.
//!
//! No cell value is stored in `runs` or `run_items`: the run keeps the file's fingerprint, row count, whether
//! the first row is a header, and the column mapping, and a resume must be given the same file again. A value
//! does reach the audit trail, in the `shown` text of an `approved` record (decision 0005), because the officer
//! approved that text. Nothing here logs or prints a value.

use crate::approval::{ItemObserver, RunContext};
use crate::csv::{self, InputSpec};
use crate::store::{AuditEntry, Store};
use crate::usage::{Event, Tally};
use serde_json::{json, Value};
use std::collections::BTreeMap;

pub const PENDING: &str = "pending";
pub const RUNNING: &str = "running";
pub const AWAITING: &str = "awaiting_approval";
pub const COMMITTING: &str = "committing";
pub const DONE: &str = "done";
pub const DIVERGED: &str = "diverged";
pub const REJECTED: &str = "rejected";
pub const UNKNOWN: &str = "unknown";
/// The officer chose to leave a diverged row out. Not in TD 6's diagram: the officer's answer to a divergence.
pub const SKIPPED: &str = "skipped";

/// Where a run's rows came from, kept in `runs.inputs_source`. Column names only, no values.
pub fn source_json(file_sha256: &str, rows: usize, header: bool, mapping: &BTreeMap<String, String>) -> String {
    json!({ "kind": "csv", "sha256": file_sha256, "rows": rows, "header": header, "map": mapping }).to_string()
}

#[derive(Debug, PartialEq)]
pub enum Outcome {
    /// Every row is done, skipped or rejected.
    Finished,
    /// The run stopped at `item`, which is `diverged` or `unknown`.
    Stopped { item: i64, state: String },
    /// The run did not start: these rows are `unknown` or `diverged` and the officer must settle them first.
    Blocked { unknown: Vec<i64>, diverged: Vec<i64> },
    /// The officer ended the run, or asked it to stop between rows.
    Ended,
}

fn db<T>(r: crate::store::Result<T>) -> Result<T, String> {
    r.map_err(|e| e.to_string())
}

fn now_secs() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0)
}

/// Makes a run safe to pick up again after a crash or a stop: a row that was `committing` becomes `unknown`
/// (the action may or may not have happened); a row that had only started or was waiting for the officer
/// becomes `pending` (nothing was done for it). Returns the rows made `unknown`.
///
/// Only the lease holder may call this: a row of a live run is not the caller's to reset.
fn recover(store: &mut Store, run: &str) -> Result<Vec<i64>, String> {
    let mut made_unknown = Vec::new();
    for (item, state, _) in db(store.run_items(run))? {
        match state.as_str() {
            COMMITTING => {
                if db(store.move_run_item(run, item, &[COMMITTING], UNKNOWN, None))? {
                    made_unknown.push(item);
                }
            }
            RUNNING | AWAITING => {
                db(store.move_run_item(run, item, &[RUNNING, AWAITING], PENDING, None))?;
            }
            _ => {}
        }
    }
    Ok(made_unknown)
}

/// Moves one row's state as the replay of that row goes. Held for one row.
struct Row<'a> {
    run: &'a str,
    item: i64,
    /// This observer moved the row to `committing`: the row is its own. A row already `committing` that this
    /// observer did not move belongs to someone else, or to a crashed run, and is refused.
    began: bool,
    /// The latest `commit_begins` was the one that moved the row (its first approval), not a later step's.
    fresh: bool,
}

impl<'a> Row<'a> {
    fn new(run: &'a str, item: i64) -> Self {
        Row { run, item, began: false, fresh: false }
    }
}

impl ItemObserver for Row<'_> {
    fn approval_asked(&mut self, store: &mut Store) {
        // A later step's approval finds the row `committing` already: no change.
        let _ = store.move_run_item(self.run, self.item, &[RUNNING], AWAITING, None);
    }
    fn commit_begins(&mut self, store: &mut Store) -> bool {
        self.fresh = false;
        if self.began {
            // A later consequential step of the same row, after an earlier one acted. Still ours, still committing.
            return matches!(store.item_state(self.run, self.item), Ok(Some(s)) if s == COMMITTING);
        }
        // The one way in: awaiting_approval to committing, in one statement. Anything else is refused,
        // including a row that is already committing.
        let moved = store.move_run_item(self.run, self.item, &[AWAITING], COMMITTING, None).unwrap_or(false);
        self.began = moved;
        self.fresh = moved;
        moved
    }
    fn commit_aborted(&mut self, store: &mut Store) {
        // The record was refused, so the executor will not act on THIS approval. If it is the row's first
        // approval nothing has been done for the row and it goes back. If an earlier step already acted, the
        // row stays `committing` and ends `unknown`: it must never look retryable.
        if self.fresh {
            let _ = store.move_run_item(self.run, self.item, &[COMMITTING], AWAITING, None);
            self.began = false;
            self.fresh = false;
        }
    }
}

/// Runs one row through `replay`. It is given the row's observer and returns the executor's report, or an
/// error when the executor ended or refused.
pub type ReplayRow<'a> = dyn FnMut(&mut Store, &RunContext, &Value, &mut dyn ItemObserver) -> Result<Value, String> + 'a;

pub struct RunParams<'a> {
    pub run: &'a str,
    pub task_ref: &'a str,
    pub officer: &'a str,
    /// One inputs object per row, in row order (`csv::build_rows`).
    pub rows: &'a [Value],
    /// Start at this row. Rows before it that are still pending stay pending.
    pub from: Option<i64>,
    /// Checked before each row. True ends the run between rows; a row already started is finished or stopped first.
    pub stop: &'a dyn Fn() -> bool,
    /// Where finished rows are counted (#35).
    pub usage: &'a Tally,
}

/// Runs the pending rows in order. Stops at the first `diverged` or `unknown` row.
pub fn run_rows(store: &mut Store, p: &RunParams, replay: &mut ReplayRow) -> Result<Outcome, String> {
    // The lease first: nothing below may touch a row while another holder has this run.
    let _lease = db(store.lease_run(p.run))?.ok_or("this run is already going, here or in another window")?;
    let info = db(store.run_info(p.run))?.ok_or("no such run")?;
    if info.3 == "ended" {
        return Err("the officer ended this run".into());
    }
    if db(store.run_items(p.run))?.len() != p.rows.len() {
        return Err("the file no longer matches the run".into());
    }
    recover(store, p.run)?;

    // Retry: a diverged or rejected row the officer chose to start from is asked again, on the record.
    if let Some(from) = p.from {
        if matches!(db(store.item_state(p.run, from))?.as_deref(), Some(DIVERGED) | Some(REJECTED)) {
            answer_locked(store, p.run, from, Answer::Retry, p.officer, now_secs())?;
        }
    }
    let items = db(store.run_items(p.run))?;
    let pick = |want: &str| items.iter().filter(|(_, s, _)| s == want).map(|(i, _, _)| *i).collect::<Vec<_>>();
    let (unknown, diverged) = (pick(UNKNOWN), pick(DIVERGED));
    if !unknown.is_empty() || !diverged.is_empty() {
        let _ = store.finish_run(p.run, "stopped");
        return Ok(Outcome::Blocked { unknown, diverged });
    }
    let _ = store.finish_run(p.run, "running");

    let start = p.from.unwrap_or(1);
    for (item, _, _) in items.iter().filter(|(i, s, _)| *i >= start && s == PENDING) {
        let item = *item;
        if (p.stop)() {
            let _ = store.finish_run(p.run, "stopped");
            return Ok(Outcome::Ended);
        }
        // The claim. If another caller holds the row, this one does not touch it.
        if !db(store.move_run_item(p.run, item, &[PENDING], RUNNING, None))? {
            continue;
        }
        let ctx = RunContext { officer: p.officer.into(), task_ref: p.task_ref.into(), run: p.run.into(), item: Some(item) };
        let inputs = &p.rows[(item - 1) as usize];
        let result = replay(store, &ctx, inputs, &mut Row::new(p.run, item));
        match settle(store, p.run, item, &result)? {
            Settled::Done => p.usage.note(Event::RowDone),
            Settled::Next => {}
            Settled::Halt(state) => {
                let _ = store.finish_run(p.run, "stopped");
                return Ok(Outcome::Stopped { item, state });
            }
            Settled::Paused => {
                let _ = store.finish_run(p.run, "stopped");
                return Ok(Outcome::Ended);
            }
        }
    }
    let items = db(store.run_items(p.run))?;
    let open = items.iter().any(|(_, s, _)| s == PENDING);
    let _ = store.finish_run(p.run, if open { "stopped" } else { "finished" });
    Ok(if open { Outcome::Ended } else { Outcome::Finished })
}

enum Settled {
    /// The replay finished the row. The run goes on.
    Done,
    /// The row ended without being done. The run goes on.
    Next,
    /// The row ended `unknown` or `diverged`: the run stops here until the officer answers.
    Halt(String),
    /// The officer stopped the row before anything was done: it is `pending` again and the run stops.
    Paused,
}

/// Writes the row's end state from what the replay reported.
///
/// The rule that prevents a second submit: a row that reached `committing` is `done` only when the executor
/// says the run finished. Every other end, including an executor that died, is `unknown`.
fn settle(store: &mut Store, run: &str, item: i64, result: &Result<Value, String>) -> Result<Settled, String> {
    let outcome = result.as_ref().ok().and_then(|r| r["outcome"].as_str());
    let committed = |store: &Store| db(store.item_state(run, item)).map(|s| s.as_deref() == Some(COMMITTING));
    match outcome {
        Some("finished") => {
            if db(store.move_run_item(run, item, &[COMMITTING, RUNNING], DONE, None))? {
                return Ok(Settled::Done);
            }
            // Finished, but the row is in a state the shell did not expect: do not call it done.
            db(store.move_run_item(run, item, &[AWAITING, PENDING], UNKNOWN, None))?;
            Ok(Settled::Halt(UNKNOWN.into()))
        }
        Some("not_approved") => {
            if committed(store)? {
                db(store.move_run_item(run, item, &[COMMITTING], UNKNOWN, None))?;
                return Ok(Settled::Halt(UNKNOWN.into()));
            }
            // The officer said no to this row. Nothing was done for it; the run goes on to the next.
            db(store.move_run_item(run, item, &[AWAITING, RUNNING], REJECTED, None))?;
            Ok(Settled::Next)
        }
        // The officer pressed Stop. Before the commit nothing was done, so the row is as if never started.
        Some("stopped") if !committed(store)? => {
            db(store.move_run_item(run, item, &[RUNNING, AWAITING], PENDING, None))?;
            Ok(Settled::Paused)
        }
        // Diverged, stopped after the commit, refused, the executor ended, an unreadable reply: nothing
        // proves what happened.
        _ => {
            if committed(store)? {
                db(store.move_run_item(run, item, &[COMMITTING], UNKNOWN, None))?;
                return Ok(Settled::Halt(UNKNOWN.into()));
            }
            let step = result.as_ref().ok().and_then(|r| r["step"].as_str()).map(String::from);
            db(store.move_run_item(run, item, &[RUNNING, AWAITING], DIVERGED, step.as_deref()))?;
            Ok(Settled::Halt(DIVERGED.into()))
        }
    }
}

// MARK: the officer's answers

/// What the officer may do with a row that stopped the run.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Answer {
    /// `unknown`: they checked the app and the row was created. It will not be run again.
    MarkDone,
    /// `unknown`: they checked and it was not created. It goes back to `pending`.
    MarkNotDone,
    /// `diverged`: leave this row out.
    Skip,
    /// `diverged`: they did this row by hand.
    FixedByHand,
    /// `diverged` or `rejected`: nothing was done for the row, ask for it again.
    Retry,
}

impl Answer {
    fn allowed(self) -> (&'static [&'static str], &'static str, &'static str) {
        match self {
            Answer::MarkDone => (&[UNKNOWN], DONE, "marked done after checking the app"),
            Answer::MarkNotDone => (&[UNKNOWN], PENDING, "marked not done after checking the app"),
            Answer::Skip => (&[DIVERGED], SKIPPED, "skipped"),
            Answer::FixedByHand => (&[DIVERGED], DONE, "done by hand"),
            Answer::Retry => (&[DIVERGED, REJECTED], PENDING, "asked again"),
        }
    }
}

/// Records the officer's answer for a row, then applies it. The trail is written first: if it cannot be, the
/// row is left as it was. The record names the row and the answer, never a value.
pub fn officer_answer(store: &mut Store, run: &str, item: i64, answer: Answer, officer: &str, now: i64) -> Result<(), String> {
    let _lease = db(store.lease_run(run))?.ok_or("the run is going; stop it before answering")?;
    answer_locked(store, run, item, answer, officer, now)
}

/// `officer_answer` for a caller that already holds the run's lease.
fn answer_locked(store: &mut Store, run: &str, item: i64, answer: Answer, officer: &str, now: i64) -> Result<(), String> {
    if !crate::approval::valid_officer(officer) {
        return Err("the officer's name is needed".into());
    }
    let (from, to, words) = answer.allowed();
    let info = db(store.run_info(run))?.ok_or("no such run")?;
    let state = db(store.item_state(run, item))?.ok_or("no such row")?;
    if !from.contains(&state.as_str()) {
        return Err(format!("row {item} is {state}; this answer is for a row that is {}", from.join(" or ")));
    }
    let task_ref = format!("{}@{}", info.0, info.1);
    let shown = format!("Row {item} {words}");
    db(store.append_event(&AuditEntry {
        at: now,
        officer,
        event: "item_marked_by_officer",
        task: Some(&task_ref),
        run: Some(run),
        item: Some(item),
        step: None,
        shown: Some(&shown),
    }))?;
    if !db(store.move_run_item(run, item, from, to, None))? {
        return Err(format!("row {item} changed while this was being recorded; look at the run again"));
    }
    Ok(())
}

/// Ends the run. Rows still pending stay pending; nothing more will run. Recorded as `run_finished`.
pub fn end_run(store: &mut Store, run: &str, officer: &str, now: i64) -> Result<(), String> {
    let _lease = db(store.lease_run(run))?.ok_or("the run is going; stop it before ending it")?;
    if !crate::approval::valid_officer(officer) {
        return Err("the officer's name is needed".into());
    }
    let info = db(store.run_info(run))?.ok_or("no such run")?;
    let task_ref = format!("{}@{}", info.0, info.1);
    db(store.append_event(&AuditEntry {
        at: now,
        officer,
        event: "run_finished",
        task: Some(&task_ref),
        run: Some(run),
        item: None,
        step: None,
        shown: Some("ended by the officer"),
    }))?;
    db(store.finish_run(run, "ended"))
}

/// A run's rows for the screen: row number and state. No values.
pub fn status(store: &Store, run: &str) -> Result<Value, String> {
    let info = db(store.run_info(run))?.ok_or("no such run")?;
    let items: Vec<Value> = db(store.run_items(run))?
        .into_iter()
        .map(|(i, s, step)| json!({ "item": i, "state": s, "last_step": step }))
        .collect();
    Ok(json!({ "run": run, "status": info.3, "items": items }))
}

/// What a run remembers about its file.
#[derive(Debug, PartialEq)]
pub struct Source {
    pub sha256: String,
    /// The officer said the first row holds column names.
    pub header: bool,
    pub map: BTreeMap<String, String>,
}

fn parse_source(source: &str) -> Option<Source> {
    let v: Value = serde_json::from_str(source).ok()?;
    if v["kind"] != "csv" {
        return None;
    }
    Some(Source {
        sha256: v["sha256"].as_str()?.to_string(),
        header: v["header"].as_bool().unwrap_or(true),
        map: v["map"]
            .as_object()
            .map(|m| m.iter().filter_map(|(k, c)| c.as_str().map(|c| (k.clone(), c.to_string()))).collect())
            .unwrap_or_default(),
    })
}

/// The run's recorded source, checked against the file the officer gave again. A different file is refused,
/// because the row numbers would no longer mean the same people.
pub fn check_same_file(source: &str, file_sha256: &str) -> Result<Source, String> {
    let src = parse_source(source).ok_or("this run was not started from a file")?;
    if src.sha256 != file_sha256 {
        return Err("this is not the file the run started with, so it cannot continue".into());
    }
    Ok(src)
}

/// What the command layer needs to run: the run, one inputs object per row, and whether it was picked up.
#[derive(Debug)]
pub struct Loaded {
    pub run: String,
    pub rows: Vec<Value>,
    pub resumed: bool,
}

/// The rows of an existing run, built from the file the officer gives again. The bytes that are hashed are the
/// bytes that are parsed.
pub fn rows_for_run(store: &Store, run: &str, path: &std::path::Path, inputs: &[InputSpec]) -> Result<Vec<Value>, String> {
    let info = db(store.run_info(run))?.ok_or("no such run")?;
    let bytes = csv::read_bytes(path)?;
    let src = check_same_file(&info.2, &csv::fingerprint(&bytes))?;
    let file = csv::parse(&bytes, src.header)?;
    let rows = csv::build_rows(&file, &src.map, inputs)?;
    if db(store.run_items(run))?.len() != rows.len() {
        return Err("the file no longer matches the run".into());
    }
    Ok(rows)
}

/// Starts a run over the file, unless an unfinished run for this task over this same file exists: then that
/// run is picked up, with the mapping it started with, and nothing new is created. Rows already done are
/// never asked again, and a row that was `committing` shows as `unknown` for the officer.
#[allow(clippy::too_many_arguments)]
pub fn start_or_resume(
    store: &mut Store,
    path: &std::path::Path,
    task_id: &str,
    version: i64,
    inputs: &[InputSpec],
    header: bool,
    mapping: &BTreeMap<String, String>,
    new_run: &str,
    now: i64,
) -> Result<Loaded, String> {
    let bytes = csv::read_bytes(path)?;
    let sha = csv::fingerprint(&bytes);
    for (id, t, v, _, source) in db(store.open_list_runs())? {
        if t == task_id && v == version && parse_source(&source).is_some_and(|s| s.sha256 == sha) {
            let rows = rows_for_run(store, &id, path, inputs)?;
            return Ok(Loaded { run: id, rows, resumed: true });
        }
    }
    let file = csv::parse(&bytes, header)?;
    let rows = csv::build_rows(&file, mapping, inputs)?;
    db(store.start_run(new_run, task_id, version, &source_json(&sha, rows.len(), header, mapping), now))?;
    db(store.create_run_items(new_run, rows.len() as i64))?;
    Ok(Loaded { run: new_run.to_string(), rows, resumed: false })
}

/// Unfinished list runs for the screen: which, for what task, and how far. No values.
pub fn unfinished(store: &Store) -> Result<Value, String> {
    let mut out = Vec::new();
    for (id, task, version, status, _) in db(store.open_list_runs())? {
        let mut counts: BTreeMap<String, i64> = BTreeMap::new();
        for (_, s, _) in db(store.run_items(&id))? {
            *counts.entry(s).or_default() += 1;
        }
        out.push(json!({ "run": id, "task": task, "version": version, "status": status, "counts": counts }));
    }
    Ok(Value::Array(out))
}

/// After a crash or a restart, a run nobody holds still shows rows as they were when it died. With the lease
/// (no one else has the run) they are recovered, so the screen shows `unknown` rows where the officer must
/// answer. Does nothing for a run that is going.
pub fn settle_orphans(store: &mut Store, run: &str) -> Result<(), String> {
    if let Some(_lease) = db(store.lease_run(run))? {
        recover(store, run)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::Fixed;

    static NO_LOG: Tally = Tally::off();

    fn store() -> Store {
        store_with_path().0
    }

    fn store_with_path() -> (Store, std::path::PathBuf) {
        let n = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos();
        let dir = std::env::temp_dir().join(format!("companion-listrun-{}-{n}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let mut s = Store::open(&dir.join("s.db"), &Fixed([9u8; 32])).unwrap();
        s.save_task("tsk_x", "Test task", "{}", 1).unwrap();
        s.start_run("run_1", "tsk_x", 1, "{}", 1).unwrap();
        s.create_run_items("run_1", 3).unwrap();
        (s, dir.join("s.db"))
    }

    fn states(s: &Store) -> Vec<String> {
        s.run_items("run_1").unwrap().into_iter().map(|r| r.1).collect()
    }

    fn params<'a>(rows: &'a [Value], stop: &'a dyn Fn() -> bool) -> RunParams<'a> {
        RunParams { run: "run_1", task_ref: "tsk_x@1", officer: "Test Officer", rows, from: None, stop, usage: &NO_LOG }
    }

    #[test]
    fn a_crash_while_committing_leaves_the_row_unknown_and_blocks_the_run() {
        let mut s = store();
        let rows = vec![json!({}); 3];
        let never = || false;
        let mut seen = 0;
        let out = run_rows(&mut s, &params(&rows, &never), &mut |st, ctx, _, obs| {
            seen += 1;
            assert_eq!(ctx.item, Some(seen));
            obs.approval_asked(st);
            assert!(obs.commit_begins(st));
            if seen == 2 {
                return Err("the executor ended before the run finished".into());
            }
            Ok(json!({"outcome":"finished"}))
        })
        .unwrap();
        assert_eq!(out, Outcome::Stopped { item: 2, state: UNKNOWN.into() });
        assert_eq!(states(&s), ["done", "unknown", "pending"]);

        // Resume: nothing runs while a row is unknown.
        let mut ran = false;
        let out = run_rows(&mut s, &params(&rows, &never), &mut |_, _, _, _| {
            ran = true;
            Ok(json!({"outcome":"finished"}))
        })
        .unwrap();
        assert_eq!(out, Outcome::Blocked { unknown: vec![2], diverged: vec![] });
        assert!(!ran);
    }

    #[test]
    fn a_run_picked_up_after_the_shell_died_treats_committing_as_unknown_and_waiting_rows_as_pending() {
        let s = &mut store();
        s.move_run_item("run_1", 1, &[PENDING], COMMITTING, None).unwrap();
        s.move_run_item("run_1", 2, &[PENDING], AWAITING, None).unwrap();
        assert_eq!(recover(s, "run_1").unwrap(), vec![1]);
        assert_eq!(states(s), ["unknown", "pending", "pending"]);
    }

    #[test]
    fn the_officer_marks_unknown_rows_and_every_answer_is_audited() {
        let mut s = store();
        s.move_run_item("run_1", 1, &[PENDING], UNKNOWN, None).unwrap();
        s.move_run_item("run_1", 2, &[PENDING], UNKNOWN, None).unwrap();
        // Wrong answer for the state is refused and writes nothing.
        assert!(officer_answer(&mut s, "run_1", 3, Answer::MarkDone, "Test Officer", 5).is_err());
        assert!(officer_answer(&mut s, "run_1", 1, Answer::Skip, "Test Officer", 5).is_err());
        assert!(officer_answer(&mut s, "run_1", 1, Answer::MarkDone, "", 5).is_err());
        assert_eq!(s.audit_count().unwrap(), 0);
        officer_answer(&mut s, "run_1", 1, Answer::MarkDone, "Test Officer", 5).unwrap();
        officer_answer(&mut s, "run_1", 2, Answer::MarkNotDone, "Test Officer", 6).unwrap();
        assert_eq!(states(&s), ["done", "pending", "pending"]);
        assert_eq!(s.audit_count().unwrap(), 2);
        s.verify_audit().unwrap();
        // The same row cannot be answered twice.
        assert!(officer_answer(&mut s, "run_1", 1, Answer::MarkDone, "Test Officer", 7).is_err());
    }

    #[test]
    fn a_divergence_stops_the_run_and_the_officer_can_skip_fix_by_hand_or_retry() {
        let mut s = store();
        let rows = vec![json!({}); 3];
        let never = || false;
        let out = run_rows(&mut s, &params(&rows, &never), &mut |_, _, _, _| {
            Ok(json!({"outcome":"diverged","step":"s2","reason":"x"}))
        })
        .unwrap();
        assert_eq!(out, Outcome::Stopped { item: 1, state: DIVERGED.into() });
        assert_eq!(states(&s), ["diverged", "pending", "pending"]);
        assert_eq!(s.run_items("run_1").unwrap()[0].2.as_deref(), Some("s2"));

        // Not answered: blocked.
        let out = run_rows(&mut s, &params(&rows, &never), &mut |_, _, _, _| panic!("must not run")).unwrap();
        assert_eq!(out, Outcome::Blocked { unknown: vec![], diverged: vec![1] });

        officer_answer(&mut s, "run_1", 1, Answer::Skip, "Test Officer", 1).unwrap();
        // Resume from row 3: row 2 stays pending and is not run.
        let mut ran = vec![];
        let mut p = params(&rows, &never);
        p.from = Some(3);
        let out = run_rows(&mut s, &p, &mut |_, ctx, _, _| {
            ran.push(ctx.item.unwrap());
            Ok(json!({"outcome":"finished"}))
        })
        .unwrap();
        assert_eq!(ran, vec![3]);
        assert_eq!(out, Outcome::Ended);
        assert_eq!(states(&s), ["skipped", "pending", "done"]);
    }

    #[test]
    fn retrying_a_diverged_row_asks_again_from_that_row() {
        let mut s = store();
        let rows = vec![json!({}); 3];
        let never = || false;
        run_rows(&mut s, &params(&rows, &never), &mut |_, _, _, _| Ok(json!({"outcome":"diverged"}))).unwrap();
        let mut p = params(&rows, &never);
        p.from = Some(1);
        let out = run_rows(&mut s, &p, &mut |_, _, _, _| Ok(json!({"outcome":"finished"}))).unwrap();
        assert_eq!(out, Outcome::Finished);
        assert_eq!(states(&s), ["done", "done", "done"]);
    }

    #[test]
    fn a_rejected_row_is_not_run_and_the_run_goes_on() {
        let mut s = store();
        let rows = vec![json!({}); 3];
        let never = || false;
        let out = run_rows(&mut s, &params(&rows, &never), &mut |st, ctx, _, obs| {
            obs.approval_asked(st);
            if ctx.item == Some(2) {
                return Ok(json!({"outcome":"not_approved","step":"s9"}));
            }
            assert!(obs.commit_begins(st));
            Ok(json!({"outcome":"finished"}))
        })
        .unwrap();
        assert_eq!(out, Outcome::Finished);
        assert_eq!(states(&s), ["done", "rejected", "done"]);
    }

    #[test]
    fn divergence_after_the_commit_began_is_unknown_not_diverged() {
        let mut s = store();
        let rows = vec![json!({}); 3];
        let never = || false;
        let out = run_rows(&mut s, &params(&rows, &never), &mut |st, _, _, obs| {
            obs.approval_asked(st);
            obs.commit_begins(st);
            Ok(json!({"outcome":"diverged","step":"s9"}))
        })
        .unwrap();
        assert_eq!(out, Outcome::Stopped { item: 1, state: UNKNOWN.into() });
    }

    #[test]
    fn a_refused_commit_record_puts_the_row_back_and_a_second_commit_cannot_start_from_pending() {
        let mut s = store();
        s.move_run_item("run_1", 1, &[PENDING], RUNNING, None).unwrap();
        let mut o = Row::new("run_1", 1);
        // A commit with no approval asked first is refused.
        assert!(!o.commit_begins(&mut s));
        o.approval_asked(&mut s);
        assert!(o.commit_begins(&mut s));
        o.commit_aborted(&mut s);
        assert_eq!(states(&s)[0], AWAITING);
    }

    #[test]
    fn the_stop_flag_ends_the_run_between_rows() {
        let mut s = store();
        let rows = vec![json!({}); 3];
        let flag = std::cell::Cell::new(false);
        let stop = || flag.get();
        let out = run_rows(&mut s, &params(&rows, &stop), &mut |_, _, _, _| {
            flag.set(true);
            Ok(json!({"outcome":"finished"}))
        })
        .unwrap();
        assert_eq!(out, Outcome::Ended);
        assert_eq!(states(&s), ["done", "pending", "pending"]);
    }

    #[test]
    fn a_different_file_cannot_resume_the_run() {
        let src = source_json("aaaa", 3, true, &[("n".to_string(), "Name".to_string())].into());
        assert_eq!(check_same_file(&src, "aaaa").unwrap().map["n"], "Name");
        assert!(check_same_file(&src, "bbbb").is_err());
        assert!(check_same_file("shell", "aaaa").is_err());
    }

    #[test]
    fn only_an_awaiting_row_can_begin_to_commit_and_only_one_observer_gets_it() {
        let mut s = store();
        s.move_run_item("run_1", 1, &[PENDING], RUNNING, None).unwrap();
        let mut a = Row::new("run_1", 1);
        let mut b = Row::new("run_1", 1);
        // Not asked yet: refused.
        assert!(!a.commit_begins(&mut s));
        a.approval_asked(&mut s);
        assert!(a.commit_begins(&mut s));
        // A second observer on the same row finds it committing and is refused, whether or not it asked.
        b.approval_asked(&mut s);
        assert!(!b.commit_begins(&mut s));
        // The first may take a later step of its own row.
        assert!(a.commit_begins(&mut s));
        assert_eq!(states(&s)[0], COMMITTING);
    }

    #[test]
    fn a_later_step_whose_record_fails_after_an_earlier_step_acted_leaves_the_row_unknown() {
        let mut s = store();
        let rows = vec![json!({}); 3];
        let never = || false;
        let out = run_rows(&mut s, &params(&rows, &never), &mut |st, _, _, obs| {
            obs.approval_asked(st);
            assert!(obs.commit_begins(st)); // step 1 approved, recorded, acted on
            obs.approval_asked(st);
            assert!(obs.commit_begins(st)); // step 2 approved ...
            obs.commit_aborted(st); // ... but its record could not be written
            Ok(json!({"outcome":"diverged","step":"s2"}))
        })
        .unwrap();
        assert_eq!(out, Outcome::Stopped { item: 1, state: UNKNOWN.into() });
        assert_eq!(states(&s)[0], UNKNOWN);
        // It is not retryable from the screen: a retry is for diverged or rejected rows only.
        assert!(officer_answer(&mut s, "run_1", 1, Answer::Retry, "Test Officer", 1).is_err());
        let mut p = params(&rows, &never);
        p.from = Some(1);
        let out = run_rows(&mut s, &p, &mut |_, _, _, _| panic!("must not run")).unwrap();
        assert_eq!(out, Outcome::Blocked { unknown: vec![1], diverged: vec![] });
    }

    #[test]
    fn a_first_step_whose_record_fails_goes_back_and_ends_diverged_with_nothing_done() {
        let mut s = store();
        let rows = vec![json!({}); 1];
        let never = || false;
        let out = run_rows(&mut s, &params(&rows[..1], &never), &mut |st, _, _, obs| {
            obs.approval_asked(st);
            assert!(obs.commit_begins(st));
            obs.commit_aborted(st);
            Ok(json!({"outcome":"diverged","step":"s1"}))
        });
        // (the run has 3 rows, so the call is refused before it starts: use the matching rows)
        assert!(out.is_err());
        let rows = vec![json!({}); 3];
        let out = run_rows(&mut s, &params(&rows, &never), &mut |st, _, _, obs| {
            obs.approval_asked(st);
            assert!(obs.commit_begins(st));
            obs.commit_aborted(st);
            Ok(json!({"outcome":"diverged","step":"s1"}))
        })
        .unwrap();
        assert_eq!(out, Outcome::Stopped { item: 1, state: DIVERGED.into() });
    }

    #[test]
    fn a_run_held_by_a_live_holder_is_not_recovered_run_or_answered_by_another_store() {
        let (mut a, db_dir) = store_with_path();
        // A second store on the same database, as a second window or process would have.
        let rows = vec![json!({}); 3];
        let never = || false;
        let mut b = Store::open(&db_dir, &Fixed([9u8; 32])).unwrap();
        let mut inner = None;
        let out = run_rows(&mut a, &params(&rows, &never), &mut |st, _, _, obs| {
            if inner.is_none() {
                obs.approval_asked(st);
                assert!(obs.commit_begins(st)); // row 1 is committing, held by a
                // b tries to take the run while a is live: refused, and row 1 is not reset.
                inner = Some((
                    run_rows(&mut b, &params(&rows, &never), &mut |_, _, _, _| panic!("b must not run")).is_err(),
                    settle_orphans(&mut b, "run_1").is_ok(),
                    officer_answer(&mut b, "run_1", 1, Answer::MarkDone, "Test Officer", 1).is_err(),
                    b.item_state("run_1", 1).unwrap(),
                ));
            }
            Ok(json!({"outcome":"finished"}))
        })
        .unwrap();
        assert_eq!(out, Outcome::Finished);
        let (refused, orphans_ok, answer_refused, seen) = inner.unwrap();
        assert!(refused && orphans_ok && answer_refused);
        assert_eq!(seen.as_deref(), Some(COMMITTING), "the live run's row was left alone");
        // Once a is done, b may take the run.
        assert!(b.lease_run("run_1").unwrap().is_some());
    }

    #[test]
    fn a_stop_before_the_commit_puts_the_row_back_and_a_stop_after_it_leaves_it_unknown() {
        let mut s = store();
        let rows = vec![json!({}); 3];
        let never = || false;
        let out = run_rows(&mut s, &params(&rows, &never), &mut |st, _, _, obs| {
            obs.approval_asked(st);
            Ok(json!({"outcome":"stopped","step":"s1"}))
        })
        .unwrap();
        assert_eq!(out, Outcome::Ended);
        assert_eq!(states(&s), ["pending", "pending", "pending"]);
        let out = run_rows(&mut s, &params(&rows, &never), &mut |st, _, _, obs| {
            obs.approval_asked(st);
            assert!(obs.commit_begins(st));
            Ok(json!({"outcome":"stopped","step":"s1"}))
        })
        .unwrap();
        assert_eq!(out, Outcome::Stopped { item: 1, state: UNKNOWN.into() });
    }

    #[test]
    fn retrying_a_diverged_or_rejected_row_is_an_audited_officer_act() {
        let mut s = store();
        let rows = vec![json!({}); 3];
        let never = || false;
        run_rows(&mut s, &params(&rows, &never), &mut |_, _, _, _| Ok(json!({"outcome":"diverged"}))).unwrap();
        let mut p = params(&rows, &never);
        p.from = Some(1);
        run_rows(&mut s, &p, &mut |_, _, _, _| Ok(json!({"outcome":"finished"}))).unwrap();
        let export = s.export_audit().unwrap();
        let marks: Vec<Value> = export.lines().skip(1).map(|l| serde_json::from_str::<Value>(l).unwrap()).filter(|r| r["event"] == "item_marked_by_officer").collect();
        assert_eq!(marks.len(), 1);
        assert_eq!(marks[0]["item"], 1);
        assert_eq!(marks[0]["shown"], "Row 1 asked again");
        s.verify_audit().unwrap();
    }
}
