//! Pause, resume and stop of a run, end to end through the real plumbing: a child process stands in for the
//! executor (a shell script), so what is tested is the shell's side: the control lines it sends, the
//! progress it passes on, the record of the stop, and ending an executor that does not stop.
//! Synthetic data only. No window, no network.

use companion_shell::approval::{run_replay_with, Hooks, RunContext};
use companion_shell::progress::{Progress, RunHandle};
use companion_shell::store::{Fixed, Store};
use serde_json::{json, Value};
use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

fn tmp(name: &str) -> PathBuf {
    let n = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos();
    let d = std::env::temp_dir().join(format!("companion-stop-{name}-{}-{n}", std::process::id()));
    std::fs::create_dir_all(&d).unwrap();
    d
}

fn task() -> Value {
    json!({ "name": "Provision a staff account", "steps": [
        { "id": "s1", "say": "Type into Full name" }, { "id": "s2", "say": "Press Create account" } ] })
}

fn ctx() -> RunContext {
    RunContext { officer: "Test Officer".into(), task_ref: "tsk_x@1".into(), run: "run_1".into(), item: None }
}

fn script(dir: &std::path::Path, body: &str) -> PathBuf {
    let p = dir.join("fake-executor.sh");
    std::fs::write(&p, format!("#!/bin/bash\n{body}\n")).unwrap();
    std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o755)).unwrap();
    p
}

const STARTED: &str = r#"echo '{"jsonrpc":"2.0","method":"run.progress","params":{"state":"started","step":"s1","say":"Type into Full name","index":1,"total":2}}'"#;

/// An executor that honours a stop the way the real one does: it records `run_stopped` and
/// `run_finished` through the shell, then answers `replay.start` with outcome `stopped`.
fn obedient(dir: &std::path::Path, log: &std::path::Path) -> PathBuf {
    script(
        dir,
        &format!(
            r#"read -r start
{STARTED}
while read -r line; do
  echo "$line" >> '{log}'
  case "$line" in
    *replay.stop*)
      echo '{{"jsonrpc":"2.0","id":"x1","method":"audit.append","params":{{"event":"run_stopped","step":"s1","shown":"Stopped by the officer at step s1. Completed: none. Not run: s1, s2."}}}}'
      read -r reply
      echo '{{"jsonrpc":"2.0","id":"x2","method":"audit.append","params":{{"event":"run_finished","shown":"stopped"}}}}'
      read -r reply
      echo '{{"jsonrpc":"2.0","id":1,"result":{{"outcome":"stopped","step":"s1","action_done":false,"completed":[],"not_run":["s1","s2"]}}}}'
      exit 0;;
  esac
done"#,
            log = log.display()
        ),
    )
}

fn open(dir: &std::path::Path) -> Store {
    Store::open(&dir.join("s.db"), &Fixed([7u8; 32])).unwrap()
}

fn events(s: &Store) -> Vec<String> {
    s.export_audit().unwrap().lines().skip(1).map(|l| serde_json::from_str::<Value>(l).unwrap()["event"].as_str().unwrap().to_string()).collect()
}

#[test]
fn pause_resume_and_stop_reach_the_executor_as_whole_lines_and_the_stop_is_recorded() {
    let dir = tmp("obedient");
    let log = dir.join("received.log");
    let bin = obedient(&dir, &log);
    let mut store = open(&dir);
    let mut progress = Progress::new("run_1", &task());

    let handle: Arc<Mutex<Option<RunHandle>>> = Arc::default();
    let h2 = handle.clone();
    let stopped_at: Arc<Mutex<Option<Instant>>> = Arc::default();
    let (h3, s3) = (handle.clone(), stopped_at.clone());
    let mut on_start = move |h: RunHandle| *h2.lock().unwrap() = Some(h);
    let mut seen_progress = Vec::new();
    let mut on_progress = |p: &Value| {
        seen_progress.push(p.clone());
        progress.apply(p);
        // The first progress line is the cue: the officer pauses, resumes, then stops.
        if p["state"] == "started" {
            let h = h3.lock().unwrap().clone().unwrap();
            assert!(h.pause());
            assert!(h.resume());
            *s3.lock().unwrap() = Some(Instant::now());
            h.stop();
            h.stop(); // pressing twice is the same as once
        }
    };
    let mut hooks = Hooks { on_start: &mut on_start, on_progress: &mut on_progress, stop_grace: Duration::from_secs(5) };
    let r = run_replay_with(&bin, &mut store, &ctx(), &task(), &json!({}), None, &mut |_| false, &mut hooks).unwrap();
    let took = stopped_at.lock().unwrap().unwrap().elapsed();

    assert_eq!(r["outcome"], "stopped");
    assert_eq!(r["not_run"], json!(["s1", "s2"]));
    assert!(took < Duration::from_millis(500), "stop to report took {took:?}");
    // The executor received exactly the three control lines, in order, and no stray reply to a notification.
    let received: Vec<String> = std::fs::read_to_string(&log).unwrap().lines().map(|l| serde_json::from_str::<Value>(l).unwrap()["method"].as_str().unwrap().to_string()).collect();
    assert_eq!(received, ["replay.pause", "replay.resume", "replay.stop"]);
    // The stop is in the trail, written by the shell under the officer's name.
    assert_eq!(events(&store), ["run_stopped", "run_finished"]);
    let export = store.export_audit().unwrap();
    assert!(export.contains("Test Officer") && export.contains("Stopped by the officer at step s1"));
    store.verify_audit().unwrap();
    assert_eq!(seen_progress.len(), 1);

    progress.finish(&Ok(r));
    let v = progress.view();
    assert_eq!(v["state"], "stopped");
    assert_eq!(v["report"]["not_run"], json!(["s1", "s2"]));
}

#[test]
fn an_executor_that_does_not_stop_is_ended_and_the_shell_records_the_stop() {
    let dir = tmp("stuck");
    // Starts, reports a step, then ignores everything (a hung accessibility call).
    let bin = script(&dir, &format!("read -r start\n{STARTED}\nexec sleep 30"));
    let mut store = open(&dir);
    let handle: Arc<Mutex<Option<RunHandle>>> = Arc::default();
    let h2 = handle.clone();
    let h3 = handle.clone();
    let mut on_start = move |h: RunHandle| *h2.lock().unwrap() = Some(h);
    let mut on_progress = |p: &Value| {
        if p["state"] == "done" {
            return;
        }
        h3.lock().unwrap().clone().unwrap().stop();
    };
    let mut hooks = Hooks { on_start: &mut on_start, on_progress: &mut on_progress, stop_grace: Duration::from_millis(200) };
    let t = Instant::now();
    let r = run_replay_with(&bin, &mut store, &ctx(), &task(), &json!({}), None, &mut |_| false, &mut hooks);
    assert!(r.is_err(), "no result came from an executor that was ended");
    assert!(t.elapsed() < Duration::from_secs(5), "ended after the grace period, not after its 30 s sleep");
    assert_eq!(events(&store), ["run_stopped", "run_finished"]);
    let export = store.export_audit().unwrap();
    assert!(export.contains("was ended") && export.contains("Not run: s2."), "{export}");
    // The step that was running may have been acted on: it is not called "not run".
    assert!(export.contains("In flight when it was ended: s1 (whether its action was taken is unknown)"), "{export}");
    store.verify_audit().unwrap();
}

#[test]
fn a_forced_stop_names_the_list_row_and_the_step_in_flight() {
    let dir = tmp("stuck-row");
    let bin = script(&dir, &format!("read -r start\n{STARTED}\nexec sleep 30"));
    let mut store = open(&dir);
    let handle: Arc<Mutex<Option<RunHandle>>> = Arc::default();
    let (h2, h3) = (handle.clone(), handle.clone());
    let mut on_start = move |h: RunHandle| *h2.lock().unwrap() = Some(h);
    let mut on_progress = |_p: &Value| h3.lock().unwrap().clone().unwrap().stop();
    let mut hooks = Hooks { on_start: &mut on_start, on_progress: &mut on_progress, stop_grace: Duration::from_millis(200) };
    let row = RunContext { item: Some(2), ..ctx() };
    assert!(run_replay_with(&bin, &mut store, &row, &task(), &json!({}), None, &mut |_| false, &mut hooks).is_err());
    let records: Vec<Value> = store.export_audit().unwrap().lines().skip(1).map(|l| serde_json::from_str(l).unwrap()).collect();
    let named: Vec<(&str, Option<i64>, Option<&str>)> =
        records.iter().map(|r| (r["event"].as_str().unwrap(), r["item"].as_i64(), r["step"].as_str())).collect();
    assert_eq!(named, [("run_stopped", Some(2), Some("s1")), ("run_finished", Some(2), Some("s1"))]);
    store.verify_audit().unwrap();
}

#[test]
fn a_run_nobody_stopped_that_loses_its_executor_writes_no_stop() {
    let dir = tmp("died");
    let bin = script(&dir, "read -r start\nexit 1");
    let mut store = open(&dir);
    let mut on_start = |_h: RunHandle| {};
    let mut on_progress = |_p: &Value| {};
    let mut hooks = Hooks { on_start: &mut on_start, on_progress: &mut on_progress, stop_grace: Duration::from_millis(100) };
    assert!(run_replay_with(&bin, &mut store, &ctx(), &task(), &json!({}), None, &mut |_| false, &mut hooks).is_err());
    assert!(events(&store).is_empty(), "only a stop the officer asked for is written as a stop");
}

#[test]
fn a_stop_after_the_run_ended_does_nothing() {
    let dir = tmp("late");
    let bin = script(&dir, r#"read -r start
echo '{"jsonrpc":"2.0","id":1,"result":{"outcome":"finished","completed":["s1","s2"],"not_run":[]}}'"#);
    let mut store = open(&dir);
    let handle: Arc<Mutex<Option<RunHandle>>> = Arc::default();
    let h2 = handle.clone();
    let mut on_start = move |h: RunHandle| *h2.lock().unwrap() = Some(h);
    let mut on_progress = |_p: &Value| {};
    let mut hooks = Hooks { on_start: &mut on_start, on_progress: &mut on_progress, stop_grace: Duration::from_millis(100) };
    let r = run_replay_with(&bin, &mut store, &ctx(), &task(), &json!({}), None, &mut |_| false, &mut hooks).unwrap();
    assert_eq!(r["outcome"], "finished");
    let h = handle.lock().unwrap().clone().unwrap();
    h.stop(); // the pipe is closed: nothing is sent, nothing breaks
    std::thread::sleep(Duration::from_millis(250));
    assert!(events(&store).is_empty());
}

#[test]
fn a_stop_pressed_before_the_run_starts_is_sent_after_replay_start_not_before() {
    let dir = tmp("early");
    let log = dir.join("received.log");
    // Logs the first two lines it is sent, in order, then ends the run as stopped.
    let bin = script(
        &dir,
        &format!(
            r#"read -r a
echo "$a" | sed 's/.*"method":"\([a-z.]*\)".*/\1/' >> '{log}'
read -r b
echo "$b" | sed 's/.*"method":"\([a-z.]*\)".*/\1/' >> '{log}'
echo '{{"jsonrpc":"2.0","id":1,"result":{{"outcome":"stopped","step":"s1","action_done":false,"completed":[],"not_run":["s1","s2"]}}}}'"#,
            log = log.display()
        ),
    );
    let mut store = open(&dir);
    // The officer pressed Stop before the executor existed: the shell hands over the controls and the stop goes out at once.
    let mut on_start = |h: RunHandle| h.stop();
    let mut on_progress = |_p: &Value| {};
    let mut hooks = Hooks { on_start: &mut on_start, on_progress: &mut on_progress, stop_grace: Duration::from_secs(5) };
    run_replay_with(&bin, &mut store, &ctx(), &task(), &json!({}), None, &mut |_| false, &mut hooks).unwrap();
    let order: Vec<String> = std::fs::read_to_string(&log).unwrap().lines().map(str::to_string).collect();
    assert_eq!(order, ["replay.start", "replay.stop"]);
}

#[test]
fn no_card_is_put_up_once_the_officer_has_stopped_the_run() {
    let dir = tmp("stale");
    let reply = dir.join("reply.log");
    // Reports a step, waits a moment, then asks for approval as if the stop had not reached it.
    let bin = script(
        &dir,
        &format!(
            r#"read -r start
{STARTED}
sleep 0.3
echo '{{"jsonrpc":"2.0","id":"x1","method":"approval.request","params":{{"card":{{"step":"s2"}}}}}}'
while read -r line; do
  case "$line" in
    *'"id":"x1"'*) echo "$line" > '{reply}'
      echo '{{"jsonrpc":"2.0","id":1,"result":{{"outcome":"stopped","step":"s1","action_done":false,"completed":[],"not_run":["s1","s2"]}}}}'
      exit 0;;
  esac
done"#,
            reply = reply.display()
        ),
    );
    let mut store = open(&dir);
    let handle: Arc<Mutex<Option<RunHandle>>> = Arc::default();
    let h2 = handle.clone();
    let h3 = handle.clone();
    let mut on_start = move |h: RunHandle| *h2.lock().unwrap() = Some(h);
    let mut on_progress = |_p: &Value| h3.lock().unwrap().clone().unwrap().stop();
    let mut hooks = Hooks { on_start: &mut on_start, on_progress: &mut on_progress, stop_grace: Duration::from_secs(5) };
    let mut asked = false;
    run_replay_with(&bin, &mut store, &ctx(), &task(), &json!({}), None, &mut |_| { asked = true; true }, &mut hooks).unwrap();
    assert!(!asked, "the officer was never asked: the run was stopped");
    assert!(std::fs::read_to_string(&reply).unwrap().contains(r#""approved":false"#));
}

#[test]
fn approved_after_the_stop_guard_declined_is_refused_and_writes_nothing() {
    let dir = tmp("stale-approved");
    let reply = dir.join("reply.log");
    // Asks for approval after the stop, then records `approved` although the shell said no.
    let bin = script(
        &dir,
        &format!(
            r#"read -r start
{STARTED}
sleep 0.3
echo '{{"jsonrpc":"2.0","id":"x1","method":"approval.request","params":{{"card":{{"step":"s2"}}}}}}'
while read -r line; do
  case "$line" in
    *'"id":"x1"'*) echo '{{"jsonrpc":"2.0","id":"x2","method":"audit.append","params":{{"event":"approved","step":"s2","shown":"synthetic"}}}}';;
    *'"id":"x2"'*) echo "$line" > '{reply}'
      echo '{{"jsonrpc":"2.0","id":1,"result":{{"outcome":"stopped","step":"s2","action_done":false,"completed":[],"not_run":["s2"]}}}}'
      exit 0;;
  esac
done"#,
            reply = reply.display()
        ),
    );
    let mut store = open(&dir);
    let handle: Arc<Mutex<Option<RunHandle>>> = Arc::default();
    let (h2, h3) = (handle.clone(), handle.clone());
    let mut on_start = move |h: RunHandle| *h2.lock().unwrap() = Some(h);
    let mut on_progress = |_p: &Value| h3.lock().unwrap().clone().unwrap().stop();
    let mut hooks = Hooks { on_start: &mut on_start, on_progress: &mut on_progress, stop_grace: Duration::from_secs(5) };
    // The officer would say yes; the guard answers no without asking.
    run_replay_with(&bin, &mut store, &ctx(), &task(), &json!({}), None, &mut |_| true, &mut hooks).unwrap();
    assert!(std::fs::read_to_string(&reply).unwrap().contains(r#""error""#));
    assert!(events(&store).is_empty());
}
