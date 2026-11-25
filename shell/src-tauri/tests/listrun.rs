//! List runs end to end (#33, TD 6): the real store, the real `drive` loop and a fake executor process that
//! creates accounts in the real stand-in app. "Created exactly once" is judged from the stand-in's own log
//! (`GET /api/accounts`), not from anything the shell says.
//!
//! Needs `node` (the stand-in and the fake executor are Node). Synthetic data only; no network beyond 127.0.0.1.

use companion_shell::approval::{run_replay_observed, Hooks};
use std::time::Duration;
use companion_shell::csv;
use companion_shell::listrun::{self, Answer, Outcome, RunParams};
use companion_shell::store::{Fixed, Store};
use companion_shell::usage::Tally;
use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};

const OFFICER: &str = "Test Officer";

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn fake_executor() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/fake-executor.mjs")
}

fn tmp(name: &str) -> PathBuf {
    let n = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos();
    let d = std::env::temp_dir().join(format!("companion-listrun-{name}-{}-{n}", std::process::id()));
    std::fs::create_dir_all(&d).unwrap();
    d
}

/// The stand-in app on a free local port. Killed when dropped.
struct StandIn {
    child: Child,
    url: String,
}

impl StandIn {
    fn start() -> Option<StandIn> {
        if Command::new("node").arg("-v").stdout(Stdio::null()).status().is_err() {
            assert!(std::env::var_os("CI").is_none(), "node is required in CI");
            eprintln!("node not found: skipping");
            return None;
        }
        let port = std::net::TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port();
        let child = Command::new("node")
            .arg(root().join("stand-in/provisioning/server.ts"))
            .env("PORT", port.to_string())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        let s = StandIn { child, url: format!("http://127.0.0.1:{port}") };
        for _ in 0..100 {
            if ureq::get(&format!("{}/healthz", s.url)).call().is_ok() {
                return Some(s);
            }
            std::thread::sleep(std::time::Duration::from_millis(100));
        }
        panic!("the stand-in did not start");
    }

    /// The emails the stand-in has created, in creation order.
    fn log(&self) -> Vec<String> {
        let text = ureq::get(&format!("{}/api/accounts", self.url)).call().unwrap().body_mut().read_to_string().unwrap();
        let v: Value = serde_json::from_str(&text).unwrap();
        v["accounts"].as_array().unwrap().iter().map(|a| a["email"].as_str().unwrap().to_string()).collect()
    }
}

impl Drop for StandIn {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

struct Fixture {
    dir: PathBuf,
    csv_path: PathBuf,
    emails: Vec<String>,
    rows: Vec<Value>,
    inputs: Vec<csv::InputSpec>,
    map: std::collections::BTreeMap<String, String>,
}

fn task_body() -> Value {
    json!({
        "name": "Provision a staff account (stand-in)",
        "inputs": [
            {"name":"full_name","type":"text"}, {"name":"email","type":"text"},
            {"name":"school","type":"text"}, {"name":"role","type":"text"}
        ]
    })
}

/// The stand-in's own synthetic staff list, mapped by name, and a run with every row pending.
fn fixture(name: &str) -> Fixture {
    let dir = tmp(name);
    let csv_path = root().join("stand-in/data/staff.csv");
    let task = task_body();
    let inputs = csv::task_inputs(&task);
    let (file, _) = csv::read_file(&csv_path, true).unwrap();
    let map = csv::suggest_mapping(&file.header, &inputs);
    let emails = file.rows.iter().map(|r| r[1].clone()).collect();
    let mut s = open(&dir);
    s.save_task("tsk_p", "Provision", &task.to_string(), 1).unwrap();
    let loaded = listrun::start_or_resume(&mut s, &csv_path, "tsk_p", 1, &inputs, true, &map, "run_1", 1).unwrap();
    assert!(!loaded.resumed);
    Fixture { dir, csv_path, emails, rows: loaded.rows, inputs, map }
}

fn open(dir: &Path) -> Store {
    Store::open(&dir.join("s.db"), &Fixed([5u8; 32])).unwrap()
}

fn states(s: &Store) -> Vec<String> {
    s.run_items("run_1").unwrap().into_iter().map(|r| r.1).collect()
}

struct Go<'a> {
    fx: &'a Fixture,
    url: &'a str,
    fake: Value,
    from: Option<i64>,
    /// Rows the "officer" rejects.
    reject: Vec<i64>,
}

impl<'a> Go<'a> {
    fn new(fx: &'a Fixture, url: &'a str) -> Self {
        Go { fx, url, fake: json!({}), from: None, reject: vec![] }
    }

    /// Opens the store afresh, as a restarted shell would, and runs. Returns the outcome and how many
    /// approval cards the officer was shown.
    fn run(&self) -> (Outcome, usize) {
        let mut s = open(&self.fx.dir);
        let mut task = task_body();
        let mut fake = self.fake.clone();
        fake["url"] = json!(self.url);
        task["fake"] = fake;
        let never = || false;
        let p = RunParams { run: "run_1", task_ref: "tsk_p@1", officer: OFFICER, rows: &self.fx.rows, from: self.from, stop: &never, usage: &Tally::off() };
        let mut asked = 0;
        let mut current = 0i64;
        let out = listrun::run_rows(&mut s, &p, &mut |store, ctx, inputs, obs| {
            current = ctx.item.unwrap();
            let mut hooks = Hooks { on_start: &mut |_| {}, on_progress: &mut |_| {}, stop_grace: Duration::from_secs(5) };
            run_replay_observed(&fake_executor(), store, ctx, &task, inputs, None, &mut |_card| {
                asked += 1;
                !self.reject.contains(&current)
            }, &mut hooks, obs)
        })
        .unwrap();
        (out, asked)
    }
}

fn once_each(log: &[String], want: &[String]) {
    let mut got = log.to_vec();
    got.sort();
    let mut want = want.to_vec();
    want.sort();
    assert_eq!(got, want, "every row exactly once, judged from the stand-in's log");
}

#[test]
fn killing_the_executor_while_committing_leaves_the_row_unknown_and_blocks_the_run() {
    let Some(app) = StandIn::start() else { return };
    let fx = fixture("kill-committing");
    let mut go = Go::new(&fx, &app.url);
    go.fake = json!({"die": "before_post", "die_on": "Chen Wei Example"});

    let (out, asked) = go.run();
    assert_eq!(out, Outcome::Stopped { item: 3, state: "unknown".into() });
    assert_eq!(asked, 3, "each row's consequential step went through the approval card");
    assert_eq!(states(&open(&fx.dir)), ["done", "done", "unknown", "pending", "pending"]);
    assert_eq!(app.log(), &fx.emails[..2], "the killed row was not created");

    // After a restart the run will not continue, and nothing is created.
    go.fake = json!({});
    let (out, asked) = go.run();
    assert_eq!(out, Outcome::Blocked { unknown: vec![3], diverged: vec![] });
    assert_eq!(asked, 0);
    assert_eq!(app.log().len(), 2);

    // The officer checks the app (row 3 is not in it) and says so. Then the rest runs, once each.
    let mut s = open(&fx.dir);
    listrun::officer_answer(&mut s, "run_1", 3, Answer::MarkNotDone, OFFICER, 10).unwrap();
    drop(s);
    let (out, asked) = go.run();
    assert_eq!(out, Outcome::Finished);
    assert_eq!(asked, 3);
    once_each(&app.log(), &fx.emails);
    assert_eq!(states(&open(&fx.dir)), ["done"; 5]);
}

#[test]
fn a_row_created_just_before_the_crash_is_marked_done_and_never_created_twice() {
    let Some(app) = StandIn::start() else { return };
    let fx = fixture("kill-after-post");
    let mut go = Go::new(&fx, &app.url);
    go.fake = json!({"die": "after_post", "die_on": "Chen Wei Example"});

    let (out, _) = go.run();
    assert_eq!(out, Outcome::Stopped { item: 3, state: "unknown".into() });
    // The stand-in shows it WAS created, though the shell never heard back.
    assert_eq!(app.log().len(), 3);

    go.fake = json!({});
    let mut s = open(&fx.dir);
    listrun::officer_answer(&mut s, "run_1", 3, Answer::MarkDone, OFFICER, 10).unwrap();
    drop(s);
    let (out, asked) = go.run();
    assert_eq!(out, Outcome::Finished);
    assert_eq!(asked, 2, "rows 4 and 5 only");
    once_each(&app.log(), &fx.emails);
}

#[test]
fn a_run_with_no_crash_creates_each_row_once_and_running_it_again_creates_nothing() {
    let Some(app) = StandIn::start() else { return };
    let fx = fixture("clean");
    let go = Go::new(&fx, &app.url);
    assert_eq!(go.run().0, Outcome::Finished);
    once_each(&app.log(), &fx.emails);
    assert_eq!(go.run().0, Outcome::Finished);
    assert_eq!(app.log().len(), fx.emails.len());
}

#[test]
fn a_divergence_stops_at_that_row_and_the_officer_can_skip_it_then_resume_from_a_chosen_row() {
    let Some(app) = StandIn::start() else { return };
    let fx = fixture("diverge");
    let mut go = Go::new(&fx, &app.url);
    go.fake = json!({"diverge_on": "Bella Koh Example"});

    let (out, _) = go.run();
    assert_eq!(out, Outcome::Stopped { item: 2, state: "diverged".into() });
    assert_eq!(app.log(), &fx.emails[..1]);
    let (out, asked) = go.run();
    assert_eq!(out, Outcome::Blocked { unknown: vec![], diverged: vec![2] });
    assert_eq!(asked, 0);

    let mut s = open(&fx.dir);
    listrun::officer_answer(&mut s, "run_1", 2, Answer::Skip, OFFICER, 10).unwrap();
    drop(s);
    // Resume from row 4: row 3 stays pending and is not run.
    go.from = Some(4);
    let (out, _) = go.run();
    assert_eq!(out, Outcome::Ended);
    assert_eq!(states(&open(&fx.dir)), ["done", "skipped", "pending", "done", "done"]);
    let want = vec![fx.emails[0].clone(), fx.emails[3].clone(), fx.emails[4].clone()];
    once_each(&app.log(), &want);
}

#[test]
fn a_rejected_row_is_not_created_and_the_others_go_on() {
    let Some(app) = StandIn::start() else { return };
    let fx = fixture("reject");
    let mut go = Go::new(&fx, &app.url);
    go.reject = vec![2];
    let (out, asked) = go.run();
    assert_eq!(out, Outcome::Finished);
    assert_eq!(asked, 5);
    assert_eq!(states(&open(&fx.dir)), ["done", "rejected", "done", "done", "done"]);
    assert_eq!(app.log().len(), 4);
    assert!(!app.log().contains(&fx.emails[1]));
}

#[test]
fn the_trail_carries_the_row_numbers_and_the_officers_marks_and_verifies() {
    let Some(app) = StandIn::start() else { return };
    let fx = fixture("trail");
    let mut go = Go::new(&fx, &app.url);
    go.fake = json!({"die": "before_post", "die_on": "Bella Koh Example"});
    go.run();
    let mut s = open(&fx.dir);
    listrun::officer_answer(&mut s, "run_1", 2, Answer::MarkNotDone, OFFICER, 10).unwrap();
    s.verify_audit().unwrap();
    let export = s.export_audit().unwrap();
    let recs: Vec<Value> = export.lines().skip(1).map(|l| serde_json::from_str(l).unwrap()).collect();
    let approved: Vec<i64> = recs.iter().filter(|r| r["event"] == "approved").map(|r| r["item"].as_i64().unwrap()).collect();
    assert_eq!(approved, vec![1, 2], "approved is written for row 2 before the executor dies");
    let mark = recs.iter().find(|r| r["event"] == "item_marked_by_officer").unwrap();
    assert_eq!(mark["item"], 2);
    assert_eq!(mark["officer"], OFFICER);
    assert!(!mark["shown"].as_str().unwrap().contains("@"), "the mark names a row, not a person");
}

/// Makes the store refuse the `approved` record for one step, as a full disk or a locked file would.
fn refuse_approved_for(dir: &Path, step: &str) {
    let conn = rusqlite::Connection::open(dir.join("s.db")).unwrap();
    conn.pragma_update(None, "key", format!("x'{}'", "05".repeat(32))).unwrap();
    conn.execute_batch(&format!(
        "CREATE TRIGGER inject_refuse BEFORE INSERT ON audit WHEN NEW.event = 'approved' AND NEW.step = '{step}'
         BEGIN SELECT RAISE(ABORT, 'injected'); END;"
    ))
    .unwrap();
}

#[test]
fn a_second_consequential_step_whose_record_fails_leaves_the_row_unknown_never_retryable() {
    let Some(app) = StandIn::start() else { return };
    let fx = fixture("two-step");
    refuse_approved_for(&fx.dir, "s5");
    let mut go = Go::new(&fx, &app.url);
    go.fake = json!({"steps": 2});

    let (out, asked) = go.run();
    // Step 1 acted (the account was created), then step 2's record was refused: row 1 may be half done.
    assert_eq!(out, Outcome::Stopped { item: 1, state: "unknown".into() });
    assert_eq!(asked, 2);
    assert_eq!(app.log(), &fx.emails[..1], "step 1 happened once, step 2 never");
    assert_eq!(states(&open(&fx.dir))[0], "unknown");

    // Run again: it is blocked and does nothing. The row never goes back to being asked.
    let (out, asked) = go.run();
    assert_eq!(out, Outcome::Blocked { unknown: vec![1], diverged: vec![] });
    assert_eq!(asked, 0);
    assert_eq!(app.log().len(), 1);
}

#[test]
fn after_a_restart_the_same_file_picks_the_run_up_and_does_not_ask_done_rows_again() {
    let Some(app) = StandIn::start() else { return };
    let fx = fixture("restart");
    let mut go = Go::new(&fx, &app.url);
    go.fake = json!({"die": "before_post", "die_on": "Chen Wei Example"});
    let (out, _) = go.run();
    assert_eq!(out, Outcome::Stopped { item: 3, state: "unknown".into() });

    // The shell restarts: a fresh store, the page's memory of the run id gone. The same file and task.
    let mut s = open(&fx.dir);
    let listed = listrun::unfinished(&s).unwrap();
    assert_eq!(listed.as_array().unwrap().len(), 1, "the unfinished run can be found");
    assert_eq!(listed[0]["run"], "run_1");
    let again = listrun::start_or_resume(&mut s, &fx.csv_path, "tsk_p", 1, &fx.inputs, true, &fx.map, "run_2", 99).unwrap();
    assert!(again.resumed);
    assert_eq!(again.run, "run_1", "no second run over the same file");
    assert_eq!(listrun::unfinished(&s).unwrap().as_array().unwrap().len(), 1);
    assert_eq!(again.rows, fx.rows);
    drop(s);

    go.fake = json!({});
    let (out, asked) = go.run();
    assert_eq!(out, Outcome::Blocked { unknown: vec![3], diverged: vec![] });
    assert_eq!(asked, 0, "rows 1 and 2 are done and are not asked again");
    let mut s = open(&fx.dir);
    listrun::officer_answer(&mut s, "run_1", 3, Answer::MarkNotDone, OFFICER, 10).unwrap();
    drop(s);
    let (out, asked) = go.run();
    assert_eq!(out, Outcome::Finished);
    assert_eq!(asked, 3);
    once_each(&app.log(), &fx.emails);
}

/// run_1 stops at row 2 with row 1 created: an unfinished run (#108).
fn stop_at_row_two(fx: &Fixture, url: &str) {
    let mut go = Go::new(fx, url);
    go.fake = json!({"diverge_on": "Bella Koh Example"});
    assert_eq!(go.run().0, Outcome::Stopped { item: 2, state: "diverged".into() });
}

/// What an officer's second Start does: if the shell lets it make a run, that run is run.
fn start_again_and_run(fx: &Fixture, url: &str, csv_path: &Path, version: i64) -> Result<listrun::Loaded, String> {
    let mut s = open(&fx.dir);
    let loaded = listrun::start_or_resume(&mut s, csv_path, "tsk_p", version, &fx.inputs, true, &fx.map, "run_2", 99)?;
    let never = || false;
    let task_ref = format!("tsk_p@{version}");
    let p = RunParams { run: &loaded.run, task_ref: &task_ref, officer: OFFICER, rows: &loaded.rows, from: None, stop: &never, usage: &Tally::off() };
    let mut task = task_body();
    task["fake"] = json!({"url": url});
    listrun::run_rows(&mut s, &p, &mut |store, ctx, inputs, obs| {
        let mut hooks = Hooks { on_start: &mut |_| {}, on_progress: &mut |_| {}, stop_grace: Duration::from_secs(5) };
        run_replay_observed(&fake_executor(), store, ctx, &task, inputs, None, &mut |_| true, &mut hooks, obs)
    })?;
    Ok(loaded)
}

/// The refusal names the open run, writes nothing and creates nothing: row 1 is in the app exactly once.
fn assert_refused_once(fx: &Fixture, app: &StandIn, again: Result<listrun::Loaded, String>) {
    assert_eq!(app.log(), &fx.emails[..1], "row 1 was created exactly once, judged from the stand-in's log");
    let why = again.expect_err("a second run must be refused");
    assert!(why.contains("run_1 (version 1)") && why.contains("end it"), "{why}");
    assert!(!why.contains('@') && !why.contains("Example"), "no cell value: {why}");
    let s = open(&fx.dir);
    assert!(s.run_info("run_2").unwrap().is_none());
    assert!(s.run_items("run_2").unwrap().is_empty());
}

#[test]
fn a_new_task_version_cannot_start_a_second_run_while_one_is_unfinished() {
    let Some(app) = StandIn::start() else { return };
    let fx = fixture("new-version");
    stop_at_row_two(&fx, &app.url);
    assert_eq!(open(&fx.dir).save_task("tsk_p", "Provision", &task_body().to_string(), 2).unwrap(), 2);
    let again = start_again_and_run(&fx, &app.url, &fx.csv_path, 2);
    assert_refused_once(&fx, &app, again);
}

#[test]
fn the_same_rows_saved_with_crlf_cannot_start_a_second_run_while_one_is_unfinished() {
    let Some(app) = StandIn::start() else { return };
    let fx = fixture("crlf");
    stop_at_row_two(&fx, &app.url);
    let text = std::fs::read_to_string(&fx.csv_path).unwrap();
    assert!(!text.contains('\r'));
    let resaved = fx.dir.join("resaved.csv");
    std::fs::write(&resaved, text.replace('\n', "\r\n")).unwrap();
    let again = start_again_and_run(&fx, &app.url, &resaved, 1);
    assert_refused_once(&fx, &app, again);
}

#[test]
fn ending_the_unfinished_run_allows_a_new_one() {
    let fx = fixture("end-then-start");
    open(&fx.dir).save_task("tsk_p", "Provision", &task_body().to_string(), 2).unwrap();
    let mut s = open(&fx.dir);
    listrun::end_run(&mut s, "run_1", OFFICER, 10).unwrap();
    let again = listrun::start_or_resume(&mut s, &fx.csv_path, "tsk_p", 2, &fx.inputs, true, &fx.map, "run_2", 99).unwrap();
    assert!(!again.resumed);
    assert_eq!(again.run, "run_2");
    assert_eq!(s.run_items("run_2").unwrap().len(), fx.rows.len());
}

#[test]
fn a_crash_mid_commit_shows_the_row_unknown_to_the_officer_after_restart() {
    let fx = fixture("crash-mid-commit");
    // The shell died with row 2 committing and row 3 waiting on the officer: nothing recorded the end.
    let s = open(&fx.dir);
    for (item, to) in [(1, "done"), (2, "committing"), (3, "awaiting_approval")] {
        s.set_run_item("run_1", item, to, None).unwrap();
    }
    drop(s);
    let mut s = open(&fx.dir);
    listrun::settle_orphans(&mut s, "run_1").unwrap();
    assert_eq!(states(&s), ["done", "unknown", "pending", "pending", "pending"]);
}

#[test]
fn two_stores_cannot_run_the_same_run_and_the_second_does_not_touch_its_rows() {
    let fx = fixture("two-stores");
    let a = open(&fx.dir);
    let _lease = a.lease_run("run_1").unwrap().expect("the first holder gets the run");
    let mut b = open(&fx.dir);
    let never = || false;
    let p = RunParams { run: "run_1", task_ref: "tsk_p@1", officer: OFFICER, rows: &fx.rows, from: None, stop: &never, usage: &Tally::off() };
    let mut called = false;
    let r = listrun::run_rows(&mut b, &p, &mut |_, _, _, _| {
        called = true;
        Err("must not be called".into())
    });
    assert!(r.is_err(), "refused while another holds the run");
    assert!(!called);
    // A live run's rows are not reset by a second reader.
    a.move_run_item("run_1", 1, &["pending"], "committing", None).unwrap();
    listrun::settle_orphans(&mut b, "run_1").unwrap();
    assert_eq!(states(&b)[0], "committing");
    // The row can be claimed by only one: awaiting to committing is a single compare-and-set.
    a.move_run_item("run_1", 2, &["pending"], "awaiting_approval", None).unwrap();
    assert!(a.move_run_item("run_1", 2, &["awaiting_approval"], "committing", None).unwrap());
    assert!(!b.move_run_item("run_1", 2, &["awaiting_approval"], "committing", None).unwrap());
}

#[test]
fn a_rejection_record_carries_no_values() {
    let Some(app) = StandIn::start() else { return };
    let fx = fixture("reject-record");
    let mut go = Go::new(&fx, &app.url);
    go.reject = vec![1];
    go.run();
    let export = open(&fx.dir).export_audit().unwrap();
    let rej = export.lines().skip(1).map(|l| serde_json::from_str::<Value>(l).unwrap()).find(|r| r["event"] == "rejected").unwrap();
    assert!(!rej["shown"].as_str().unwrap_or("").contains('@'));
}
