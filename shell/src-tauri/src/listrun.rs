//! List runs (E-5, TD 6): one task, once per spreadsheet row, the officer approving each row.
//!
//! The guarantee is that no row is submitted twice, even across a crash. It rests on four things:
//!  1. `committing` is written to `run_items` before the `approved` audit record, and the executor acts only
//!     after that record is written (see `approval::ItemObserver`).
//!  2. A row found `committing` when a run is picked up again is `unknown`, never retried by the shell.
//!  3. A run will not continue while any row is `unknown` or `diverged`. Only the officer clears one, by saying
//!     what happened in the target app, and that is written to the audit trail.
//!  4. Every state change is a compare-and-set in the store, so a second caller cannot take a row the first holds.
//!
//! Values are not stored. The run keeps the file's fingerprint, row count and column mapping; a resume must be
//! given the same file again. Nothing here logs or prints a value.

use crate::approval::{ItemObserver, RunContext};
use crate::csv::{self, Csv, InputSpec};
use crate::store::{AuditEntry, Store};
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
pub fn source_json(file_sha256: &str, rows: usize, mapping: &BTreeMap<String, String>) -> String {
    json!({ "kind": "csv", "sha256": file_sha256, "rows": rows, "map": mapping }).to_string()
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

/// Makes a run safe to pick up again after a crash or a stop: a row that was `committing` becomes `unknown`
/// (the action may or may not have happened); a row that had only started or was waiting for the officer
/// becomes `pending` (nothing was done for it). Returns the rows made `unknown`.
pub fn recover(store: &mut Store, run: &str) -> Result<Vec<i64>, String> {
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
}

impl ItemObserver for Row<'_> {
    fn approval_asked(&mut self, store: &mut Store) {
        // A second approval in the same row finds it `committing` already: no change.
        let _ = store.move_run_item(self.run, self.item, &[RUNNING], AWAITING, None);
    }
    fn commit_begins(&mut self, store: &mut Store) -> bool {
        match store.item_state(self.run, self.item) {
            Ok(Some(s)) if s == COMMITTING => true,
            Ok(Some(s)) if s == AWAITING => {
                store.move_run_item(self.run, self.item, &[AWAITING], COMMITTING, None).unwrap_or(false)
            }
            _ => false,
        }
    }
    fn commit_aborted(&mut self, store: &mut Store) {
        // The record was refused, so the executor will not act. Only a row that this call moved goes back.
        let _ = store.move_run_item(self.run, self.item, &[COMMITTING], AWAITING, None);
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
}

/// Runs the pending rows in order. Stops at the first `diverged` or `unknown` row.
pub fn run_rows(store: &mut Store, p: &RunParams, replay: &mut ReplayRow) -> Result<Outcome, String> {
    let info = db(store.run_info(p.run))?.ok_or("no such run")?;
    if info.3 == "ended" {
        return Err("the officer ended this run".into());
    }
    if db(store.run_items(p.run))?.len() != p.rows.len() {
        return Err("the file no longer matches the run".into());
    }
    recover(store, p.run)?;

    // Retry: the chosen row, if it diverged or was rejected, is asked again.
    if let Some(from) = p.from {
        db(store.move_run_item(p.run, from, &[DIVERGED, REJECTED], PENDING, None))?;
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
        let result = replay(store, &ctx, inputs, &mut Row { run: p.run, item });
        let end = settle(store, p.run, item, &result)?;
        if let Some(state) = end {
            let _ = store.finish_run(p.run, "stopped");
            return Ok(Outcome::Stopped { item, state });
        }
    }
    let items = db(store.run_items(p.run))?;
    let open = items.iter().any(|(_, s, _)| s == PENDING);
    let _ = store.finish_run(p.run, if open { "stopped" } else { "finished" });
    Ok(if open { Outcome::Ended } else { Outcome::Finished })
}

/// Writes the row's end state from what the replay reported. Returns the state to stop on, if any.
///
/// The rule that prevents a second submit: a row that reached `committing` is `done` only when the executor
/// says the run finished. Every other end, including an executor that died, is `unknown`.
fn settle(store: &mut Store, run: &str, item: i64, result: &Result<Value, String>) -> Result<Option<String>, String> {
    let outcome = result.as_ref().ok().and_then(|r| r["outcome"].as_str());
    let committed = |store: &Store| db(store.item_state(run, item)).map(|s| s.as_deref() == Some(COMMITTING));
    match outcome {
        Some("finished") => {
            if db(store.move_run_item(run, item, &[COMMITTING, RUNNING], DONE, None))? {
                return Ok(None);
            }
            // Finished, but the row is in a state the shell did not expect: do not call it done.
            db(store.move_run_item(run, item, &[AWAITING, PENDING], UNKNOWN, None))?;
            Ok(Some(UNKNOWN.into()))
        }
        Some("not_approved") => {
            if committed(store)? {
                db(store.move_run_item(run, item, &[COMMITTING], UNKNOWN, None))?;
                return Ok(Some(UNKNOWN.into()));
            }
            // The officer said no to this row. Nothing was done for it; the run goes on to the next.
            db(store.move_run_item(run, item, &[AWAITING, RUNNING], REJECTED, None))?;
            Ok(None)
        }
        // Diverged, refused, the executor ended, an unreadable reply: nothing proves what happened.
        _ => {
            if committed(store)? {
                db(store.move_run_item(run, item, &[COMMITTING], UNKNOWN, None))?;
                return Ok(Some(UNKNOWN.into()));
            }
            let step = result.as_ref().ok().and_then(|r| r["step"].as_str()).map(String::from);
            db(store.move_run_item(run, item, &[RUNNING, AWAITING], DIVERGED, step.as_deref()))?;
            Ok(Some(DIVERGED.into()))
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
}

impl Answer {
    fn allowed(self) -> (&'static [&'static str], &'static str, &'static str) {
        match self {
            Answer::MarkDone => (&[UNKNOWN], DONE, "marked done after checking the app"),
            Answer::MarkNotDone => (&[UNKNOWN], PENDING, "marked not done after checking the app"),
            Answer::Skip => (&[DIVERGED], SKIPPED, "skipped"),
            Answer::FixedByHand => (&[DIVERGED], DONE, "done by hand"),
        }
    }
}

/// Records the officer's answer for a row, then applies it. The trail is written first: if it cannot be, the
/// row is left as it was. The record names the row and the answer, never a value.
pub fn officer_answer(store: &mut Store, run: &str, item: i64, answer: Answer, officer: &str, now: i64) -> Result<(), String> {
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

/// The run's recorded source, checked against the file the officer gave again. A different file is refused,
/// because the row numbers would no longer mean the same people.
pub fn check_same_file(source: &str, file_sha256: &str) -> Result<BTreeMap<String, String>, String> {
    let v: Value = serde_json::from_str(source).map_err(|_| "this run was not started from a file".to_string())?;
    if v["kind"] != "csv" {
        return Err("this run was not started from a file".into());
    }
    if v["sha256"].as_str() != Some(file_sha256) {
        return Err("this is not the file the run started with, so it cannot continue".into());
    }
    Ok(v["map"]
        .as_object()
        .map(|m| m.iter().filter_map(|(k, c)| c.as_str().map(|c| (k.clone(), c.to_string()))).collect())
        .unwrap_or_default())
}

/// Convenience for the command layer: parse the file, build every row, and fingerprint it.
pub fn prepare(
    path: &std::path::Path,
    inputs: &[InputSpec],
    mapping: &BTreeMap<String, String>,
) -> Result<(Csv, Vec<Value>, String), String> {
    let (csv, bytes) = csv::read_file(path)?;
    let rows = csv::build_rows(&csv, mapping, inputs)?;
    Ok((csv, rows, csv::fingerprint(&bytes)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::Fixed;

    fn store() -> Store {
        let n = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos();
        let dir = std::env::temp_dir().join(format!("companion-listrun-{}-{n}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let mut s = Store::open(&dir.join("s.db"), &Fixed([9u8; 32])).unwrap();
        s.save_task("tsk_x", "Test task", "{}", 1).unwrap();
        s.start_run("run_1", "tsk_x", 1, "{}", 1).unwrap();
        s.create_run_items("run_1", 3).unwrap();
        s
    }

    fn states(s: &Store) -> Vec<String> {
        s.run_items("run_1").unwrap().into_iter().map(|r| r.1).collect()
    }

    fn params<'a>(rows: &'a [Value], stop: &'a dyn Fn() -> bool) -> RunParams<'a> {
        RunParams { run: "run_1", task_ref: "tsk_x@1", officer: "Test Officer", rows, from: None, stop }
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
        let mut o = Row { run: "run_1", item: 1 };
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
        let src = source_json("aaaa", 3, &[("n".to_string(), "Name".to_string())].into());
        assert_eq!(check_same_file(&src, "aaaa").unwrap()["n"], "Name");
        assert!(check_same_file(&src, "bbbb").is_err());
        assert!(check_same_file("shell", "aaaa").is_err());
    }
}
