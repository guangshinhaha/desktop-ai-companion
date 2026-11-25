//! The live progress of a replay and the officer's controls over it (PRD C-3, C-4; TD 3.1).
//!
//! The executor sends `run.progress` notifications (`state` started | done | paused | resumed, `step`,
//! `say`, `index`, `total`). `Progress` folds them into what the progress view draws. The shell sends
//! `replay.pause`, `replay.resume` and `replay.stop` back through a `RunHandle`.
//!
//! A stop is a request the executor honours at its next checkpoint (within a fraction of a second, even
//! in the middle of a wait). If it has not ended the run `STOP_GRACE` later, the shell ends the executor
//! process: the executor acts only through accessibility calls, so ending it leaves the target app as it
//! is and no further action can happen. Either way the trail records `run_stopped`.
//! Nothing here logs or prints. Step names and the task's own `say` lines are all it holds.

use serde_json::{json, Value};
use std::io::Write;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

/// How long the shell waits for the executor to end a stopped run before it ends the process.
pub const STOP_GRACE: Duration = Duration::from_secs(2);

/// One writer for the executor's standard input, shared by the run's replies and the officer's controls.
/// A whole line goes out under one lock, so a control message can never land inside a reply.
#[derive(Clone)]
pub struct SharedWriter(Arc<Mutex<Option<Box<dyn Write + Send>>>>);

impl SharedWriter {
    pub fn new<W: Write + Send + 'static>(w: W) -> Self {
        SharedWriter(Arc::new(Mutex::new(Some(Box::new(w)))))
    }

    /// Closes the executor's input, which tells it to exit. Every clone is closed with it.
    pub fn close(&self) {
        if let Ok(mut g) = self.0.lock() {
            *g = None;
        }
    }

    fn with<T>(&self, f: impl FnOnce(&mut dyn Write) -> std::io::Result<T>) -> std::io::Result<T> {
        let mut g = self.0.lock().map_err(|_| std::io::Error::other("poisoned"))?;
        match g.as_mut() {
            Some(w) => f(w.as_mut()),
            None => Err(std::io::Error::new(std::io::ErrorKind::BrokenPipe, "closed")),
        }
    }
}

impl Write for SharedWriter {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.with(|w| w.write(buf))
    }
    fn write_all(&mut self, buf: &[u8]) -> std::io::Result<()> {
        self.with(|w| w.write_all(buf).and_then(|_| w.flush()))
    }
    fn flush(&mut self) -> std::io::Result<()> {
        self.with(|w| w.flush())
    }
}

pub fn control_line(method: &str) -> String {
    format!("{}\n", json!({ "jsonrpc": "2.0", "method": method }))
}

/// The officer's hands on a run in progress. Cheap to clone; every method is safe to call at any time.
#[derive(Clone)]
pub struct RunHandle {
    pub(crate) writer: SharedWriter,
    pub(crate) stop_asked: Arc<AtomicBool>,
    pub(crate) ended: Arc<AtomicBool>,
    pub(crate) kill: Arc<dyn Fn() + Send + Sync>,
    pub(crate) grace: Duration,
}

impl RunHandle {
    fn send(&self, method: &str) -> bool {
        self.writer.clone().write_all(control_line(method).as_bytes()).is_ok()
    }

    /// Holds the run before its next step. A step already waiting for its checks finishes waiting.
    pub fn pause(&self) -> bool {
        !self.stop_asked() && self.send("replay.pause")
    }

    pub fn resume(&self) -> bool {
        !self.stop_asked() && self.send("replay.resume")
    }

    /// Asks the run to stop, and ends the executor if it has not ended the run by `grace`. Safe to call twice.
    pub fn stop(&self) {
        if self.stop_asked.swap(true, Ordering::SeqCst) {
            return;
        }
        let sent = self.send("replay.stop");
        let (ended, kill, grace) = (self.ended.clone(), self.kill.clone(), if sent { self.grace } else { Duration::ZERO });
        // A thread that lives only for the grace period: it does not poll, it sleeps once.
        std::thread::spawn(move || {
            std::thread::sleep(grace);
            if !ended.load(Ordering::SeqCst) {
                kill();
            }
        });
    }

    pub fn stop_asked(&self) -> bool {
        self.stop_asked.load(Ordering::SeqCst)
    }
}

#[derive(Clone, Copy, PartialEq, Debug)]
enum Mark {
    Todo,
    Current,
    Done,
    /// The step the run stopped at: not completed.
    Stopped,
    /// The step the run diverged at or was refused at: not completed.
    Failed,
}

impl Mark {
    fn name(self) -> &'static str {
        match self {
            Mark::Todo => "todo",
            Mark::Current => "current",
            Mark::Done => "done",
            Mark::Stopped => "stopped",
            Mark::Failed => "failed",
        }
    }
}

/// What the progress view shows. `view()` is the only thing the web view receives.
pub struct Progress {
    run: String,
    task: String,
    steps: Vec<(String, String, Mark)>,
    /// running | paused | stopping | finished | stopped | diverged | not_approved | refused | ended
    state: String,
    current: Option<Value>,
    pause_asked: bool,
    report: Option<Value>,
}

const TERMINAL: [&str; 6] = ["finished", "stopped", "diverged", "not_approved", "refused", "ended"];

fn strings(v: &Value) -> Vec<String> {
    v.as_array().map(|a| a.iter().filter_map(|s| s.as_str().map(str::to_string)).collect()).unwrap_or_default()
}

impl Progress {
    /// From the saved task: its name and its steps' ids and `say` lines.
    pub fn new(run: &str, task: &Value) -> Self {
        let steps = task["steps"]
            .as_array()
            .map(|a| {
                a.iter()
                    .map(|s| (s["id"].as_str().unwrap_or("").to_string(), s["say"].as_str().unwrap_or("").to_string(), Mark::Todo))
                    .collect()
            })
            .unwrap_or_default();
        Progress {
            run: run.to_string(),
            task: task["name"].as_str().unwrap_or("").to_string(),
            steps,
            state: "running".into(),
            current: None,
            pause_asked: false,
            report: None,
        }
    }

    pub fn is_over(&self) -> bool {
        TERMINAL.contains(&self.state.as_str())
    }

    /// Folds in the params of one `run.progress` notification. Anything malformed or late is ignored.
    pub fn apply(&mut self, p: &Value) {
        if self.is_over() {
            return;
        }
        let step = p["step"].as_str().unwrap_or("");
        let at = self.steps.iter().position(|s| s.0 == step);
        match p["state"].as_str() {
            Some("started") => {
                if let Some(i) = at {
                    self.steps[i].2 = Mark::Current;
                }
                self.current = Some(json!({
                    "step": step, "say": p["say"].as_str().unwrap_or(""),
                    "index": p["index"].as_i64().unwrap_or(0), "total": p["total"].as_i64().unwrap_or(0),
                }));
            }
            Some("done") => {
                if let Some(i) = at {
                    self.steps[i].2 = Mark::Done;
                }
            }
            Some("paused") => {
                self.pause_asked = false;
                if self.state == "running" {
                    self.state = "paused".into();
                }
            }
            Some("resumed") => {
                self.pause_asked = false;
                if self.state == "paused" {
                    self.state = "running".into();
                }
            }
            _ => {}
        }
    }

    pub fn note_pause_asked(&mut self) {
        if self.state == "running" {
            self.pause_asked = true;
        }
    }

    pub fn note_resume_asked(&mut self) {
        self.pause_asked = false;
        if self.state == "paused" {
            self.state = "running".into();
        }
    }

    /// The officer pressed stop. The view says so until the run ends.
    pub fn note_stop_asked(&mut self) {
        if !self.is_over() {
            self.state = "stopping".into();
            self.pause_asked = false;
        }
    }

    /// The end of the run: the executor's result, or why there is none. Steps done by then stay done;
    /// the rest are reported as not done.
    pub fn finish(&mut self, result: &Result<Value, String>) {
        if self.is_over() {
            return;
        }
        let done_now: Vec<String> = self.steps.iter().filter(|s| s.2 == Mark::Done).map(|s| s.0.clone()).collect();
        let (outcome, completed, not_run, at, mark, extra) = match result {
            Ok(r) => {
                let outcome = match r["outcome"].as_str() {
                    Some(o @ ("finished" | "stopped" | "diverged" | "not_approved" | "refused")) => o,
                    _ => "ended",
                };
                let mark = if outcome == "stopped" { Mark::Stopped } else { Mark::Failed };
                (outcome, strings(&r["completed"]), strings(&r["not_run"]), r["step"].as_str().map(str::to_string), mark, r.clone())
            }
            Err(e) => {
                let not_run: Vec<String> = self.steps.iter().filter(|s| s.2 != Mark::Done).map(|s| s.0.clone()).collect();
                ("ended", done_now.clone(), not_run, None, Mark::Failed, json!({ "reason": e }))
            }
        };
        for s in self.steps.iter_mut() {
            s.2 = if completed.contains(&s.0) {
                Mark::Done
            } else if Some(&s.0) == at.as_ref() && outcome != "finished" {
                mark
            } else {
                Mark::Todo
            };
        }
        self.state = outcome.to_string();
        self.pause_asked = false;
        self.current = None;
        let mut report = json!({ "outcome": outcome, "completed": completed, "not_run": not_run });
        for k in ["step", "reason", "action_done"] {
            if !extra[k].is_null() {
                report[k] = extra[k].clone();
            }
        }
        self.report = Some(report);
    }

    pub fn view(&self) -> Value {
        let state = if self.pause_asked && self.state == "running" { "pausing" } else { self.state.as_str() };
        json!({
            "run": self.run,
            "task": self.task,
            "state": state,
            "over": self.is_over(),
            "current": self.current,
            "steps": self.steps.iter().map(|s| json!({ "id": s.0, "say": s.1, "status": s.2.name() })).collect::<Vec<_>>(),
            "report": self.report,
        })
    }
}

/// The run in progress or the last one finished, as the shell keeps it. Memory only.
pub struct ActiveRun {
    pub progress: Progress,
    /// Set once the executor is started; None before that and after the run ends.
    pub handle: Option<RunHandle>,
    /// Stop pressed before the handle existed. Honoured the moment it does.
    pub stop_wanted: bool,
}

impl ActiveRun {
    pub fn new(progress: Progress) -> Self {
        ActiveRun { progress, handle: None, stop_wanted: false }
    }

    pub fn is_live(&self) -> bool {
        !self.progress.is_over()
    }

    pub fn pause(&mut self) {
        if let Some(h) = &self.handle {
            if h.pause() {
                self.progress.note_pause_asked();
            }
        }
    }

    pub fn resume(&mut self) {
        if let Some(h) = &self.handle {
            if h.resume() {
                self.progress.note_resume_asked();
            }
        }
    }

    pub fn stop(&mut self) {
        if !self.is_live() {
            return;
        }
        self.progress.note_stop_asked();
        match &self.handle {
            Some(h) => h.stop(),
            None => self.stop_wanted = true,
        }
    }

    pub fn attach(&mut self, h: RunHandle) {
        if self.stop_wanted {
            h.stop();
        }
        self.handle = Some(h);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn task() -> Value {
        json!({ "name": "Provision a staff account", "steps": [
            { "id": "s1", "say": "Type into Full name" },
            { "id": "s2", "say": "Type into Email" },
            { "id": "s3", "say": "Press Create account" } ] })
    }

    fn ev(state: &str, step: &str, i: i64) -> Value {
        json!({ "state": state, "step": step, "say": format!("say {step}"), "index": i, "total": 3 })
    }

    fn marks(p: &Progress) -> Vec<String> {
        p.view()["steps"].as_array().unwrap().iter().map(|s| s["status"].as_str().unwrap().to_string()).collect()
    }

    #[test]
    fn the_view_shows_the_current_say_line_and_what_is_done_and_left() {
        let mut p = Progress::new("run_1", &task());
        assert_eq!(marks(&p), ["todo", "todo", "todo"]);
        p.apply(&ev("started", "s1", 1));
        p.apply(&ev("done", "s1", 1));
        p.apply(&ev("started", "s2", 2));
        let v = p.view();
        assert_eq!(v["current"]["say"], "say s2");
        assert_eq!(v["current"]["index"], 2);
        assert_eq!(v["state"], "running");
        assert_eq!(marks(&p), ["done", "current", "todo"]);
        assert_eq!(v["steps"][2]["say"], "Press Create account");
    }

    #[test]
    fn pause_shows_pausing_until_the_executor_holds_then_paused_then_running() {
        let mut p = Progress::new("r", &task());
        p.note_pause_asked();
        assert_eq!(p.view()["state"], "pausing");
        p.apply(&ev("paused", "s2", 2));
        assert_eq!(p.view()["state"], "paused");
        p.apply(&ev("resumed", "s2", 2));
        assert_eq!(p.view()["state"], "running");
    }

    #[test]
    fn a_stopped_run_reports_what_completed_and_what_did_not() {
        let mut p = Progress::new("r", &task());
        p.apply(&ev("started", "s1", 1));
        p.apply(&ev("done", "s1", 1));
        p.note_stop_asked();
        assert_eq!(p.view()["state"], "stopping");
        p.finish(&Ok(json!({ "outcome": "stopped", "step": "s2", "action_done": false, "completed": ["s1"], "not_run": ["s2", "s3"] })));
        let v = p.view();
        assert_eq!(v["state"], "stopped");
        assert_eq!(v["over"], true);
        assert_eq!(v["report"]["completed"], json!(["s1"]));
        assert_eq!(v["report"]["not_run"], json!(["s2", "s3"]));
        assert_eq!(marks(&p), ["done", "stopped", "todo"]);
        // A late event changes nothing.
        p.apply(&ev("done", "s2", 2));
        assert_eq!(marks(&p), ["done", "stopped", "todo"]);
    }

    #[test]
    fn an_executor_that_vanished_is_reported_as_ended_with_what_was_seen_done() {
        let mut p = Progress::new("r", &task());
        p.apply(&ev("done", "s1", 1));
        p.finish(&Err("the executor ended before the run finished".into()));
        let v = p.view();
        assert_eq!(v["state"], "ended");
        assert_eq!(v["report"]["completed"], json!(["s1"]));
        assert_eq!(v["report"]["not_run"], json!(["s2", "s3"]));
    }

    #[test]
    fn a_divergence_marks_its_step_failed_and_carries_the_reason() {
        let mut p = Progress::new("r", &task());
        p.finish(&Ok(json!({ "outcome": "diverged", "step": "s1", "reason": "element not found within 30 s", "completed": [], "not_run": ["s1", "s2", "s3"] })));
        assert_eq!(marks(&p), ["failed", "todo", "todo"]);
        assert_eq!(p.view()["report"]["reason"], "element not found within 30 s");
    }

    #[test]
    fn stop_before_the_executor_exists_is_remembered_and_stop_after_the_end_does_nothing() {
        let mut r = ActiveRun::new(Progress::new("r", &task()));
        r.stop();
        assert!(r.stop_wanted);
        assert_eq!(r.progress.view()["state"], "stopping");
        r.progress.finish(&Ok(json!({ "outcome": "stopped", "step": "s1", "completed": [], "not_run": ["s1"] })));
        r.stop();
        assert_eq!(r.progress.view()["state"], "stopped");
    }

    #[test]
    fn a_whole_line_goes_out_at_once() {
        let buf = Arc::new(Mutex::new(Vec::<u8>::new()));
        struct B(Arc<Mutex<Vec<u8>>>);
        impl Write for B {
            fn write(&mut self, b: &[u8]) -> std::io::Result<usize> {
                self.0.lock().unwrap().extend_from_slice(b);
                Ok(b.len())
            }
            fn flush(&mut self) -> std::io::Result<()> {
                Ok(())
            }
        }
        let mut w = SharedWriter::new(B(buf.clone()));
        w.write_all(control_line("replay.stop").as_bytes()).unwrap();
        let text = String::from_utf8(buf.lock().unwrap().clone()).unwrap();
        assert_eq!(text, "{\"jsonrpc\":\"2.0\",\"method\":\"replay.stop\"}\n");
    }
}
