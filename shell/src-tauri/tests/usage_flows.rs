//! The usage log wired into the real flows (#35): the shell's own replay and list-run code, with a shell
//! script as the executor. Synthetic data only. No window, no network, no keychain.

use companion_shell::approval::{Hooks, NoObserver, RunContext};
use companion_shell::listrun::{self, Answer, Loaded, Outcome, RunParams};
use companion_shell::progress::RunHandle;
use companion_shell::store::{Fixed, Store};
use companion_shell::usage::{counted_replay, Event, Tally, UsageLog};
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::time::Duration;

/// Synthetic values that go through the flows. None may reach the log, the export or `stats`.
const SEEDS: [&str; 7] = [
    "Zebra-Quokka-7731",   // an officer's name
    "S1234567A",           // an input value
    "aisha@example.test",  // a spreadsheet cell
    "Seeded Column Name",  // a spreadsheet column
    "Seeded Task Name",    // a task's name
    "Press Seeded Button", // a step: screen content
    "seeded-screen-text",  // what the executor says it saw
];

/// The inputs hold `MODE:<m>`: `refuse`, `diverge`, `stop` and `die` end that way. Anything else asks for
/// approval and reports what the officer said.
const FAKE: &str = r#"#!/bin/bash
read -r start
case "$start" in
  *MODE:die*) exit 1;;
  *MODE:refuse*) echo '{"jsonrpc":"2.0","id":1,"result":{"outcome":"refused","reason":"seeded-screen-text"}}';;
  *MODE:diverge*) echo '{"jsonrpc":"2.0","id":1,"result":{"outcome":"diverged","step":"s1","reason":"seeded-screen-text"}}';;
  *MODE:stop*) echo '{"jsonrpc":"2.0","id":1,"result":{"outcome":"stopped","step":"s1"}}';;
  *)
    echo '{"jsonrpc":"2.0","id":"x1","method":"approval.request","params":{"card":{"step":"s1","summary":"seeded-screen-text"}}}'
    read -r answer
    case "$answer" in
      *'"approved":true'*)
        echo '{"jsonrpc":"2.0","id":"x2","method":"audit.append","params":{"event":"approved","step":"s1","shown":"seeded-screen-text"}}'
        read -r recorded
        echo '{"jsonrpc":"2.0","id":1,"result":{"outcome":"finished","completed":["s1"]}}';;
      *) echo '{"jsonrpc":"2.0","id":1,"result":{"outcome":"not_approved","step":"s1"}}';;
    esac;;
esac
"#;

struct Flow {
    dir: PathBuf,
    bin: PathBuf,
    /// The usage log has a folder to itself, so every file in it can be searched.
    log_dir: PathBuf,
}

impl Flow {
    fn new(name: &str) -> Flow {
        let n = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos();
        let dir = std::env::temp_dir().join(format!("companion-usageflow-{name}-{}-{n}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let bin = dir.join("fake-executor.sh");
        std::fs::write(&bin, FAKE).unwrap();
        std::fs::set_permissions(&bin, std::fs::Permissions::from_mode(0o755)).unwrap();
        let log_dir = dir.join("log");
        Flow { dir, bin, log_dir }
    }

    fn tally(&self) -> Tally {
        Tally::at(self.log_dir.join("usage.db"))
    }

    fn store(&self) -> Store {
        Store::open(&self.dir.join("s.db"), &Fixed([4u8; 32])).unwrap()
    }

    /// One replay, as `replay_start` runs it. Returns the report and whether the shell was given the
    /// run's controls.
    fn replay(&self, tally: &Tally, bin: &Path, mode: &str) -> (Result<Value, String>, bool) {
        let mut store = self.store();
        let ctx = RunContext { officer: SEEDS[0].into(), task_ref: "tsk_u@1".into(), run: "run_1".into(), item: None };
        let inputs = json!({ "nric": SEEDS[1], "mode": format!("MODE:{mode}") });
        let mut attached = false;
        let r = {
            let mut on_start = |_h: RunHandle| attached = true;
            let mut on_progress = |_p: &Value| {};
            let mut hooks = Hooks { on_start: &mut on_start, on_progress: &mut on_progress, stop_grace: Duration::from_secs(5) };
            counted_replay(tally, bin, &mut store, &ctx, &task(), &inputs, None, &mut |_| true, &mut hooks, &mut NoObserver)
        };
        (r, attached)
    }

    fn new_list_run(&self, n: i64) {
        let mut s = self.store();
        s.save_task("tsk_u", SEEDS[4], &task().to_string(), 1).unwrap();
        s.start_run("run_l", "tsk_u", 1, "{}", 1).unwrap();
        s.create_run_items("run_l", n).unwrap();
    }

    /// A list run over rows with these modes, as `spawn_listrun` runs it. The officer rejects the rows
    /// numbered in `reject`.
    fn list(&self, tally: &Tally, modes: &[&str], reject: &[i64]) -> Outcome {
        let mut store = self.store();
        let rows: Vec<Value> =
            modes.iter().map(|m| json!({ "email": SEEDS[2], SEEDS[3]: SEEDS[1], "mode": format!("MODE:{m}") })).collect();
        let never = || false;
        let p = RunParams { run: "run_l", task_ref: "tsk_u@1", officer: SEEDS[0], rows: &rows, from: None, stop: &never, usage: tally };
        listrun::run_rows(&mut store, &p, &mut |store, ctx, inputs, obs| {
            let row = ctx.item.unwrap();
            let mut on_start = |_h: RunHandle| {};
            let mut on_progress = |_p: &Value| {};
            let mut hooks = Hooks { on_start: &mut on_start, on_progress: &mut on_progress, stop_grace: Duration::from_secs(5) };
            counted_replay(tally, &self.bin, store, ctx, &task(), inputs, None, &mut |_| !reject.contains(&row), &mut hooks, obs)
        })
        .unwrap()
    }

    /// Counts over all days (a test may cross midnight).
    fn counts(&self) -> BTreeMap<String, i64> {
        let log = UsageLog::open(&self.log_dir.join("usage.db")).unwrap();
        let v: Value = serde_json::from_str(&log.export_json().unwrap()).unwrap();
        let mut out = BTreeMap::new();
        for day in v["days"].as_array().unwrap() {
            for (k, n) in day["counts"].as_object().unwrap() {
                *out.entry(k.clone()).or_insert(0) += n.as_i64().unwrap();
            }
        }
        out
    }
}

fn task() -> Value {
    json!({ "name": SEEDS[4], "steps": [{ "id": "s1", "say": SEEDS[5] }] })
}

fn loaded(resumed: bool) -> Loaded {
    Loaded { run: "run_l".into(), rows: vec![json!({ "email": SEEDS[2] })], resumed }
}

fn want(pairs: &[(&str, i64)]) -> BTreeMap<String, i64> {
    pairs.iter().map(|(k, n)| (k.to_string(), *n)).collect()
}

fn now() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs() as i64
}

#[test]
fn a_finished_replay_shows_as_a_replay_return() {
    let f = Flow::new("return");
    let (r, attached) = f.replay(&f.tally(), &f.bin, "finish");
    assert_eq!(r.unwrap()["outcome"], "finished");
    assert!(attached, "the shell still gets the run's controls");
    assert_eq!(f.counts(), want(&[("replay_started", 1), ("replay_finished", 1)]));
    let stats = UsageLog::open(&f.log_dir.join("usage.db")).unwrap().stats(now(), 1).unwrap();
    let replay = stats.lines().find(|l| l.starts_with("replay returns")).unwrap();
    assert!(replay.ends_with(":1"), "{replay}");
}

#[test]
fn only_a_finished_replay_counts_as_finished() {
    for mode in ["refuse", "diverge", "stop", "die"] {
        let f = Flow::new(mode);
        let _ = f.replay(&f.tally(), &f.bin, mode);
        assert_eq!(f.counts(), want(&[("replay_started", 1)]), "{mode}");
    }
}

#[test]
fn an_executor_that_cannot_start_counts_nothing() {
    let f = Flow::new("nospawn");
    let (r, _) = f.replay(&f.tally(), &f.dir.join("not-there"), "finish");
    assert!(r.is_err());
    assert_eq!(f.counts(), want(&[]));
}

#[test]
fn a_list_run_counts_each_replayed_row_and_only_rows_a_replay_finished() {
    let f = Flow::new("list");
    let tally = f.tally();
    let modes = ["finish", "finish", "diverge"];
    f.new_list_run(3);
    tally.list_run_loaded(&loaded(false));
    // Row 1 finishes, the officer rejects row 2, row 3 diverges and stops the run.
    assert_eq!(f.list(&tally, &modes, &[2]), Outcome::Stopped { item: 3, state: "diverged".into() });
    let after_first = want(&[("list_run", 1), ("replay_started", 3), ("replay_finished", 1), ("row_done", 1)]);
    assert_eq!(f.counts(), after_first);

    // Picked up again while blocked: nothing runs, nothing is counted.
    tally.list_run_loaded(&loaded(true));
    assert!(matches!(f.list(&tally, &modes, &[]), Outcome::Blocked { .. }));
    assert_eq!(f.counts(), after_first);

    // The officer did row 3 by hand: it is done, but no replay did it.
    let mut s = f.store();
    listrun::officer_answer(&mut s, "run_l", 3, Answer::FixedByHand, "Test Officer", 5).unwrap();
    drop(s);
    assert_eq!(f.list(&tally, &modes, &[]), Outcome::Finished);
    assert_eq!(f.counts(), after_first);
}

#[test]
fn a_resumed_list_run_counts_only_what_it_replays_and_is_not_a_second_list_run() {
    let f = Flow::new("resume");
    let tally = f.tally();
    f.new_list_run(2);
    tally.list_run_loaded(&loaded(false));
    assert_eq!(f.list(&tally, &["finish", "diverge"], &[]), Outcome::Stopped { item: 2, state: "diverged".into() });
    let mut s = f.store();
    listrun::officer_answer(&mut s, "run_l", 2, Answer::Retry, "Test Officer", 5).unwrap();
    drop(s);
    tally.list_run_loaded(&loaded(true));
    assert_eq!(f.list(&tally, &["finish", "finish"], &[]), Outcome::Finished);
    // Row 1 is not run or counted again. Row 2 was replayed twice and finished once.
    assert_eq!(f.counts(), want(&[("list_run", 1), ("replay_started", 3), ("replay_finished", 2), ("row_done", 2)]));
}

#[test]
fn seeded_values_pushed_through_the_flows_never_reach_the_log() {
    let f = Flow::new("seeded");
    let tally = f.tally();
    for mode in ["finish", "refuse", "diverge", "die"] {
        let _ = f.replay(&tally, &f.bin, mode);
    }
    f.new_list_run(2);
    tally.list_run_loaded(&loaded(false));
    f.list(&tally, &["finish", "diverge"], &[]);
    // Not an empty log: the flows above were counted.
    assert_eq!(f.counts(), want(&[("list_run", 1), ("replay_started", 6), ("replay_finished", 2), ("row_done", 1)]));
    // The values did go through: the executor's text is in the (encrypted) audit trail.
    assert!(f.store().export_audit().unwrap().contains("seeded-screen-text"));

    let log = UsageLog::open(&f.log_dir.join("usage.db")).unwrap();
    let mut places = vec![
        ("export".to_string(), log.export_json().unwrap().into_bytes()),
        ("stats".to_string(), log.stats(now(), 4).unwrap().into_bytes()),
    ];
    for entry in std::fs::read_dir(&f.log_dir).unwrap() {
        let p = entry.unwrap().path();
        places.push((p.display().to_string(), std::fs::read(&p).unwrap()));
    }
    for (name, bytes) in places {
        for s in SEEDS.iter().chain(["tsk_u", "run_l", "run_1"].iter()) {
            assert!(!bytes.windows(s.len()).any(|w| w == s.as_bytes()), "{s} found in {name}");
        }
    }
}

#[test]
fn a_log_that_cannot_be_written_never_affects_a_run() {
    let f = Flow::new("broken");
    // The log's path is a folder: it cannot be opened.
    std::fs::create_dir_all(f.log_dir.join("usage.db")).unwrap();
    let tally = f.tally();
    let (r, attached) = f.replay(&tally, &f.bin, "finish");
    assert_eq!(r.unwrap()["outcome"], "finished");
    assert!(attached);
    f.new_list_run(2);
    tally.list_run_loaded(&loaded(false));
    assert_eq!(f.list(&tally, &["finish", "finish"], &[]), Outcome::Finished);
    let states: Vec<String> = f.store().run_items("run_l").unwrap().into_iter().map(|r| r.1).collect();
    assert_eq!(states, ["done", "done"]);
}

#[test]
fn a_locked_log_does_not_stall_a_run() {
    let f = Flow::new("locked");
    let path = f.log_dir.join("usage.db");
    drop(UsageLog::open(&path).unwrap());
    let holder = rusqlite::Connection::open(&path).unwrap();
    holder.execute_batch("BEGIN EXCLUSIVE").unwrap();
    let (r, _) = f.replay(&f.tally(), &f.bin, "finish");
    assert_eq!(r.unwrap()["outcome"], "finished");
    // The writes alone, so a slow process launch is not timed. At SQLite's default wait of 5 s these two
    // take 10 s.
    let tally = f.tally();
    let t = std::time::Instant::now();
    tally.note(Event::ReplayStarted);
    tally.note(Event::ReplayFinished);
    assert!(t.elapsed() < Duration::from_secs(4), "{:?}", t.elapsed());
}
