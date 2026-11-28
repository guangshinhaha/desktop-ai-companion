//! Review tests (#26). Synthetic data only. The store key is injected (never the login keychain).
//! Saved tasks are checked by the real `scripts/validate-task`.

use companion_shell::recording::{self, Phase};
use companion_shell::review::{check_task, iso_utc, Draft};
use companion_shell::store::{Fixed, Store};
use rusqlite::Connection;
use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{mpsc, Arc, Mutex, Once};
use std::time::Duration;

const SECRET: &str = "Zebra-Quokka-7731"; // synthetic stand-in for a typed value
const NOW: i64 = 1_795_000_000;

fn tmp(name: &str) -> PathBuf {
    let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos();
    let d = std::env::temp_dir().join(format!("companion-review-{name}-{}-{nanos}", std::process::id()));
    std::fs::create_dir_all(&d).unwrap();
    d
}

fn open(dir: &Path) -> Store {
    Store::open(&dir.join("store.db"), &Fixed([3u8; 32])).unwrap()
}

/// What the recorder writes: descriptors and input names, a press it cannot classify (unknown),
/// and a button it recognises as consequential.
fn recorded() -> Value {
    json!({
      "format": "task/v1", "id": "tsk_ab12cd34", "version": 1, "name": "Recorded task",
      "author": "officer", "created": "2026-11-24T10:00:00+08:00", "platform": "macos",
      "apps": [{ "key": "chrome", "bundle": "com.google.Chrome" }],
      "preconditions": [{ "app": "chrome", "check": { "kind": "exists",
          "target": { "role": "AXSecureTextField", "label": "Password" } } }],
      "inputs": [
        { "name": "full_name", "type": "text", "source": "ask" },
        { "name": "email", "type": "email", "source": "ask" }
      ],
      "steps": [
        { "id": "s1", "say": "Type into Full name", "app": "chrome", "action": "set_value",
          "target": { "role": "AXTextField", "label": "Full name" },
          "value": { "input": "full_name" },
          "expect": [{ "kind": "value_equals", "input": "full_name" }], "consequential": "no" },
        { "id": "s2", "say": "Type into Email", "app": "chrome", "action": "set_value",
          "target": { "role": "AXTextField", "label": "Email" },
          "value": { "input": "email" },
          "expect": [{ "kind": "value_equals", "input": "email" }], "consequential": "no" },
        { "id": "s3", "say": "Press Next", "app": "chrome", "action": "press",
          "target": { "role": "AXButton", "label": "Next" },
          "expect": [{ "kind": "exists", "target": { "role": "AXButton", "label": "Next" } }],
          "consequential": "unknown",
          "approval": { "summary": "Press Next with {full_name}, {email}", "undoable": false } },
        { "id": "s4", "say": "Press Create account", "app": "chrome", "action": "press",
          "target": { "role": "AXButton", "label": "Create account" },
          "expect": [{ "kind": "exists", "target": { "role": "AXButton", "label": "Create account" } }],
          "consequential": "yes",
          "approval": { "summary": "Press Create account with {full_name}, {email}", "undoable": false } }
      ]
    })
}

fn draft() -> Draft {
    Draft::from_json(&recorded().to_string(), None).unwrap()
}

fn edit(d: &mut Draft, e: Value) -> Result<(), String> {
    d.apply(&e)
}

/// `scripts/validate-task` runs `npm ci` when ajv is missing. Tests run in parallel, and
/// concurrent installs into one folder break each other, so the first caller installs for all.
fn install_validator(root: &Path) {
    static INSTALL: Once = Once::new();
    INSTALL.call_once(|| {
        if !root.join("schema/node_modules/ajv").exists() {
            let ok = Command::new("npm")
                .args(["ci", "--no-audit", "--no-fund"])
                .current_dir(root.join("schema"))
                .status()
                .is_ok_and(|s| s.success());
            assert!(ok, "npm ci in schema/ failed");
        }
    });
}

fn validate(body: &str, dir: &Path) -> (bool, String) {
    let f = dir.join("check.json");
    std::fs::write(&f, body).unwrap();
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    install_validator(&root);
    let out = Command::new(root.join("scripts/validate-task")).arg(&f).output().unwrap();
    let text = format!("{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr));
    (out.status.success(), text)
}

fn step<'a>(v: &'a Value, id: &str) -> &'a Value {
    v["steps"].as_array().unwrap().iter().find(|s| s["id"] == id).unwrap()
}

/// Every byte of every file under `dir` (the store, its journal, the drafts folder).
fn all_bytes(dir: &Path) -> Vec<(PathBuf, Vec<u8>)> {
    let mut out = vec![];
    let mut stack = vec![dir.to_path_buf()];
    while let Some(p) = stack.pop() {
        for e in std::fs::read_dir(&p).unwrap() {
            let path = e.unwrap().path();
            if path.is_dir() {
                stack.push(path);
            } else {
                let b = std::fs::read(&path).unwrap();
                out.push((path, b));
            }
        }
    }
    out
}

fn contains(hay: &[u8], needle: &str) -> bool {
    hay.windows(needle.len()).any(|w| w == needle.as_bytes())
}

/// Every text value of every table, decrypted: what the store holds, not just its file bytes.
fn decrypted_dump(dir: &Path) -> String {
    let c = Connection::open(dir.join("store.db")).unwrap();
    c.pragma_update(None, "key", format!("x'{}'", "03".repeat(32))).unwrap();
    let tables: Vec<String> = c
        .prepare("SELECT name FROM sqlite_master WHERE type='table'")
        .unwrap()
        .query_map([], |r| r.get(0))
        .unwrap()
        .map(|r| r.unwrap())
        .collect();
    let mut all = String::new();
    for t in tables {
        let mut st = c.prepare(&format!("SELECT * FROM \"{t}\"")).unwrap();
        let n = st.column_count();
        let mut rows = st.query([]).unwrap();
        while let Some(r) = rows.next().unwrap() {
            for i in 0..n {
                let v: rusqlite::types::Value = r.get(i).unwrap();
                all.push_str(&format!("{v:?}\n"));
            }
        }
    }
    all
}

// ---- what the officer sees -------------------------------------------------------------

#[test]
fn the_view_shows_say_lines_inputs_unknown_and_consequential_steps() {
    let v = draft().view();
    let say: Vec<&str> = v["steps"].as_array().unwrap().iter().map(|s| s["say"].as_str().unwrap()).collect();
    assert_eq!(say, ["Type into Full name", "Type into Email", "Press Next", "Press Create account"]);
    let names: Vec<&str> = v["inputs"].as_array().unwrap().iter().map(|i| i["name"].as_str().unwrap()).collect();
    assert_eq!(names, ["full_name", "email"]);
    assert_eq!(step(&v, "s3")["flag"], "unknown");
    assert_eq!(step(&v, "s4")["flag"], "consequential");
    assert_eq!(step(&v, "s1")["flag"], "none");
    assert_eq!((v["unknown_count"].as_i64(), v["consequential_count"].as_i64()), (Some(1), Some(1)));
    assert_eq!(v["apps"][0]["bundle"], "com.google.Chrome");
    assert_eq!(v["preconditions"].as_array().unwrap().len(), 1);
    assert_eq!(v["problems"].as_array().unwrap().len(), 0);
}

#[test]
fn something_that_is_not_a_recorded_task_is_refused() {
    assert!(Draft::from_json("{\"format\":\"other\"}", None).is_err());
    assert!(Draft::from_json("not json", None).is_err());
}

// ---- edits -----------------------------------------------------------------------------

#[test]
fn renaming_an_input_updates_every_use_and_stays_valid() {
    let mut d = draft();
    edit(&mut d, json!({"op":"rename_input","from":"full_name","to":"staff_name"})).unwrap();
    let v = d.view();
    assert_eq!(v["inputs"][0]["name"], "staff_name");
    assert_eq!(v["inputs"][0]["steps"], json!(["s1"]));
    assert!(v["problems"].as_array().unwrap().is_empty());
    for bad in ["Has Caps", "1abc", "", "email"] {
        assert!(edit(&mut d, json!({"op":"rename_input","from":"staff_name","to":bad})).is_err(), "{bad}");
    }
    assert!(edit(&mut d, json!({"op":"rename_input","from":"nope","to":"x"})).is_err());
}

#[test]
fn deleting_a_step_drops_inputs_nothing_uses_and_their_mentions() {
    let mut d = draft();
    edit(&mut d, json!({"op":"delete_step","step":"s2"})).unwrap();
    let v = d.view();
    let names: Vec<&str> = v["inputs"].as_array().unwrap().iter().map(|i| i["name"].as_str().unwrap()).collect();
    assert_eq!(names, ["full_name"]);
    assert!(v["problems"].as_array().unwrap().is_empty(), "{}", v["problems"]);
    assert!(edit(&mut d, json!({"op":"delete_step","step":"s2"})).is_err());
    // The last steps cannot all be deleted: that would be an empty task.
    edit(&mut d, json!({"op":"delete_step","step":"s1"})).unwrap();
    edit(&mut d, json!({"op":"delete_step","step":"s3"})).unwrap();
    assert!(edit(&mut d, json!({"op":"delete_step","step":"s4"})).is_err());
}

#[test]
fn marking_consequential_yes_or_no_keeps_the_approval_rule() {
    let mut d = draft();
    edit(&mut d, json!({"op":"set_consequential","step":"s1","value":"yes"})).unwrap();
    assert!(step(&d.view(), "s1")["flag"] == "consequential");
    edit(&mut d, json!({"op":"set_consequential","step":"s4","value":"no"})).unwrap();
    assert!(d.view()["problems"].as_array().unwrap().is_empty());
    // The officer cannot set a step back to unknown, or to anything else.
    assert!(edit(&mut d, json!({"op":"set_consequential","step":"s1","value":"unknown"})).is_err());
    assert!(edit(&mut d, json!({"op":"set_consequential","step":"s1","value":"maybe"})).is_err());
}

#[test]
fn a_constant_needs_the_no_personal_data_tick_in_the_backend() {
    let mut d = draft();
    for e in [
        json!({"op":"mark_constant","step":"s1","value":SECRET}),
        json!({"op":"mark_constant","step":"s1","value":SECRET,"confirmed_no_personal_data":false}),
        json!({"op":"mark_constant","step":"s1","value":SECRET,"confirmed_no_personal_data":"true"}),
        json!({"op":"mark_constant","step":"s1","value":SECRET,"confirmed_no_personal_data":1}),
    ] {
        assert!(edit(&mut d, e).is_err());
    }
    let after = d.view();
    assert_eq!(step(&after, "s1")["is_constant"], false);
    assert!(!after.to_string().contains(SECRET), "a refused constant must not appear");
    edit(&mut d, json!({"op":"mark_constant","step":"s1","value":SECRET,"confirmed_no_personal_data":true})).unwrap();
    assert_eq!(step(&d.view(), "s1")["is_constant"], true);
    // Only typed or chosen values can be constants.
    assert!(edit(&mut d, json!({"op":"mark_constant","step":"s4","value":"x","confirmed_no_personal_data":true})).is_err());
    // Empty or oversized values are refused.
    assert!(edit(&mut d, json!({"op":"mark_constant","step":"s2","value":"","confirmed_no_personal_data":true})).is_err());
    let big = "x".repeat(501);
    assert!(edit(&mut d, json!({"op":"mark_constant","step":"s2","value":big,"confirmed_no_personal_data":true})).is_err());
}

#[test]
fn a_task_edited_outside_the_api_with_an_unconfirmed_constant_cannot_be_saved() {
    let mut t = recorded();
    t["steps"][0]["value"] = json!({"constant": SECRET, "confirmed_no_personal_data": false});
    assert!(!check_task(&t).is_empty());
    t["steps"][0]["value"] = json!(SECRET); // a bare literal
    assert!(!check_task(&t).is_empty());
    t["steps"][0]["value"] = json!({"constant": SECRET});
    assert!(!check_task(&t).is_empty());
    let d = Draft::from_json(&t.to_string(), None).unwrap();
    let dir = tmp("unconfirmed");
    let mut s = open(&dir);
    let err = d.save(&mut s, "officer-1", NOW).unwrap_err();
    assert!(!err.contains(SECRET), "errors never carry a value");
    assert!(s.task_versions("tsk_ab12cd34").unwrap().is_empty());
}

// ---- save ------------------------------------------------------------------------------

#[test]
fn saving_writes_the_next_version_with_author_date_app_and_preconditions_and_validates() {
    let dir = tmp("save");
    let mut s = open(&dir);
    let mut d = draft();
    edit(&mut d, json!({"op":"rename_input","from":"full_name","to":"staff_name"})).unwrap();
    edit(&mut d, json!({"op":"mark_constant","step":"s2","value":SECRET,"confirmed_no_personal_data":true})).unwrap();
    let (id, v1) = d.save(&mut s, "officer-1", NOW).unwrap();
    assert_eq!((id.as_str(), v1), ("tsk_ab12cd34", 1));
    let body = s.task_body("tsk_ab12cd34", 1).unwrap().unwrap();
    let (ok, text) = validate(&body, &dir);
    assert!(ok, "{text}");
    let t: Value = serde_json::from_str(&body).unwrap();
    assert_eq!(t["author"], "officer-1");
    assert_eq!(t["created"], iso_utc(NOW));
    assert_eq!(t["version"], 1);
    assert_eq!(t["apps"][0]["bundle"], "com.google.Chrome");
    assert_eq!(t["preconditions"].as_array().unwrap().len(), 1);

    // A second review of the same task id is version 2; version 1 is kept.
    let (_, v2) = draft().save(&mut s, "officer-2", NOW + 60).unwrap();
    assert_eq!(v2, 2);
    assert_eq!(s.task_versions("tsk_ab12cd34").unwrap(), vec![1, 2]);
    let t2: Value = serde_json::from_str(&s.task_body("tsk_ab12cd34", 2).unwrap().unwrap()).unwrap();
    assert_eq!((t2["version"].as_i64(), t2["author"].as_str()), (Some(2), Some("officer-2")));
    assert!(validate(&s.task_body("tsk_ab12cd34", 2).unwrap().unwrap(), &dir).0);

    // Each save left a chained audit record that names the task and holds no value.
    assert_eq!(s.audit_count().unwrap(), 2);
    s.verify_audit().unwrap();
    let dump = decrypted_dump(&dir);
    assert!(dump.contains("task_saved") && dump.contains("tsk_ab12cd34@2"));
}

#[test]
fn a_blank_author_or_an_unready_task_saves_nothing() {
    let dir = tmp("refuse");
    let mut s = open(&dir);
    assert!(draft().save(&mut s, "   ", NOW).is_err());
    let mut t = recorded();
    t["steps"][0]["expect"] = json!([]);
    assert!(Draft::from_json(&t.to_string(), None).unwrap().save(&mut s, "o", NOW).is_err());
    assert!(s.task_versions("tsk_ab12cd34").unwrap().is_empty());
    assert_eq!(s.audit_count().unwrap(), 0);
}

#[test]
fn a_step_left_unknown_is_unknown_in_the_saved_task_after_every_other_edit() {
    let dir = tmp("unknown");
    let mut s = open(&dir);
    let mut d = draft();
    edit(&mut d, json!({"op":"rename_input","from":"email","to":"work_email"})).unwrap();
    edit(&mut d, json!({"op":"set_consequential","step":"s1","value":"yes"})).unwrap();
    edit(&mut d, json!({"op":"delete_step","step":"s2"})).unwrap();
    edit(&mut d, json!({"op":"mark_constant","step":"s1","value":"fixed","confirmed_no_personal_data":true})).unwrap();
    d.save(&mut s, "officer-1", NOW).unwrap();
    let body = s.task_body("tsk_ab12cd34", 1).unwrap().unwrap();
    let t: Value = serde_json::from_str(&body).unwrap();
    assert_eq!(step(&t, "s3")["consequential"], "unknown");
    assert!(step(&t, "s3")["approval"].is_object(), "unknown keeps its approval so replay can ask");
    assert!(validate(&body, &dir).0);
}

// ---- discard ---------------------------------------------------------------------------

#[test]
fn discarding_leaves_no_typed_value_in_the_store_or_any_app_folder() {
    let app = tmp("discard"); // stands in for the app-support folder: store, drafts
    let drafts = app.join("drafts");
    std::fs::create_dir_all(&drafts).unwrap();
    let source = drafts.join("draft-1.json");
    std::fs::write(&source, recorded().to_string()).unwrap();

    // A store with unrelated history, so the search runs over real tables.
    let mut s = open(&app);
    draft().save(&mut s, "officer-1", NOW).unwrap();

    let mut d = Draft::from_file(&source).unwrap();
    edit(&mut d, json!({"op":"mark_constant","step":"s1","value":SECRET,"confirmed_no_personal_data":true})).unwrap();
    edit(&mut d, json!({"op":"mark_constant","step":"s2","value":format!("{SECRET}-2"),"confirmed_no_personal_data":true})).unwrap();
    assert!(d.view().to_string().contains(SECRET), "the officer sees what they typed while reviewing");

    // Before discard: still nothing on disk. The draft is memory only.
    for (p, b) in all_bytes(&app) {
        assert!(!contains(&b, SECRET), "{} holds the value before save", p.display());
    }
    d.discard();

    assert!(!source.exists(), "the recorder's file is deleted");
    drop(s);
    for (p, b) in all_bytes(&app) {
        assert!(!contains(&b, SECRET), "{} holds a typed value after discard", p.display());
    }
    let dump = decrypted_dump(&app);
    assert!(!dump.contains(SECRET), "a store row holds a typed value after discard");
    assert!(!dump.contains("Zebra"));
}

#[test]
fn a_failed_save_keeps_nothing_in_the_store_and_the_value_only_after_a_good_save() {
    let app = tmp("failsave");
    let mut s = open(&app);
    let mut d = draft();
    edit(&mut d, json!({"op":"mark_constant","step":"s1","value":SECRET,"confirmed_no_personal_data":true})).unwrap();
    assert!(d.clone_for_retry().save(&mut s, "", NOW).is_err());
    assert!(!decrypted_dump(&app).contains(SECRET));
    d.save(&mut s, "officer-1", NOW).unwrap();
    // A saved constant is part of the task by the officer's choice, and only in the tasks table.
    assert!(decrypted_dump(&app).contains(SECRET));
    // Encrypted at rest: the raw file does not show it.
    for (p, b) in all_bytes(&app) {
        assert!(!contains(&b, SECRET), "{} shows a value in plain bytes", p.display());
    }
}

// ---- recording start and stop ----------------------------------------------------------

/// A stand-in executor: on SIGINT it writes the recorded task to --out and exits 0, as the real
/// one does at stop. SIGUSR1 does the same and stands in for Stop recording in the REC menu,
/// where the shell sends nothing and the recorder ends by itself.
fn fake_executor(dir: &Path, task: &str, write: bool) -> PathBuf {
    let bin = dir.join("fake-executor");
    let payload = dir.join("payload.json");
    std::fs::write(&payload, task).unwrap();
    let body = format!(
        "#!/bin/sh\nout=\"\"\nwhile [ $# -gt 0 ]; do [ \"$1\" = \"--out\" ] && out=\"$2\"; shift; done\n\
         trap '{} exit 0' INT USR1\necho $$ > \"{}/ready\"\n\
         while :; do sleep 1 & wait $!; done\n",
        if write {
            format!("cp \"{}\" \"$out\";", payload.display())
        } else {
            "exit 1;".to_string()
        },
        dir.display()
    );
    std::fs::write(&bin, body).unwrap();
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(&bin, std::fs::Permissions::from_mode(0o755)).unwrap();
    bin
}

/// The stand-in installs its stop handler, then touches `ready`. Waits for that, bounded.
/// The stand-in writes `ready` with a shell builtin: macOS /bin/sh can drop a SIGINT that arrives
/// while it waits for a command such as `touch` or `mv` (#92). The newline says the write is whole.
fn wait_ready(dir: &Path) {
    let t = std::time::Instant::now();
    while !std::fs::read_to_string(dir.join("ready")).is_ok_and(|s| s.ends_with('\n')) {
        assert!(t.elapsed() < BOUND, "stand-in executor never became ready");
        std::thread::yield_now();
    }
}

/// What the stand-in's exit produced, with a bound so a watcher that never fires fails the test.
type Exit = mpsc::Receiver<Result<PathBuf, String>>;
const BOUND: Duration = Duration::from_secs(30);

fn start(bin: &Path, drafts: &Path) -> (recording::Recording, Exit) {
    let (tx, rx) = mpsc::channel();
    let rec = recording::start(bin, "com.google.Chrome", drafts, move |r| {
        let _ = tx.send(r);
    })
    .unwrap();
    (rec, rx)
}

/// Stop recording in the REC menu: the recorder ends on its own, with no call from the shell.
fn choose_stop_in_rec_menu(dir: &Path) {
    wait_ready(dir);
    let pid = std::fs::read_to_string(dir.join("ready")).unwrap();
    assert!(Command::new("/bin/kill").args(["-USR1", pid.trim()]).status().unwrap().success());
}

/// The same call `record_start` makes. The receiver fires when the shell has what to show.
fn begin(phase: &Arc<Mutex<Phase>>, bin: &Path, drafts: &Path) -> Result<mpsc::Receiver<()>, String> {
    let (tx, rx) = mpsc::channel();
    recording::begin(phase, bin, "com.google.Chrome", drafts, move || {
        let _ = tx.send(());
    })?;
    Ok(rx)
}

fn drafts_in(dir: &Path) -> usize {
    std::fs::read_dir(dir).unwrap().count()
}

#[test]
fn stopping_a_recording_yields_the_file_the_review_opens() {
    let dir = tmp("rec");
    let bin = fake_executor(&dir, &recorded().to_string(), true);
    let drafts = dir.join("drafts");
    let (rec, exit) = start(&bin, &drafts);
    wait_ready(&dir);
    rec.stopper().stop().unwrap();
    let path = exit.recv_timeout(BOUND).unwrap().unwrap();
    assert!(path.starts_with(&drafts));
    let d = Draft::from_file(&path).unwrap();
    assert_eq!(d.view()["steps"].as_array().unwrap().len(), 4);
    d.discard();
    assert!(!path.exists());
}

#[test]
fn stop_in_the_rec_menu_puts_the_recording_under_review_with_no_call_from_the_shell() {
    let dir = tmp("rec-menu");
    let bin = fake_executor(&dir, &recorded().to_string(), true);
    let drafts = dir.join("drafts");
    let phase = Arc::new(Mutex::new(Phase::default()));
    let finished = begin(&phase, &bin, &drafts).unwrap();
    choose_stop_in_rec_menu(&dir);
    finished.recv_timeout(BOUND).expect("the shell did not notice that the recorder finished");
    let p = phase.lock().unwrap();
    let Phase::Review(d) = &*p else { panic!("no draft under review") };
    assert_eq!(d.view()["steps"].as_array().unwrap().len(), 4);
}

#[test]
fn no_new_recording_starts_while_one_is_going_stopping_or_under_review() {
    let dir = tmp("rec-busy");
    let bin = fake_executor(&dir, &recorded().to_string(), true);
    let drafts = dir.join("drafts");
    let phase = Arc::new(Mutex::new(Phase::default()));
    let finished = begin(&phase, &bin, &drafts).unwrap();
    assert!(begin(&phase, &bin, &drafts).unwrap_err().contains("already recording"));
    choose_stop_in_rec_menu(&dir);
    finished.recv_timeout(BOUND).unwrap();
    assert!(begin(&phase, &bin, &drafts).unwrap_err().contains("under review"));
    // The refused start swept nothing: the draft under review still has its file.
    assert_eq!(drafts_in(&drafts), 1);
}

#[test]
fn a_recorder_that_fails_or_writes_nothing_usable_says_so_and_a_new_recording_can_start() {
    // (name, what the recorder writes, whether it writes, what the officer is told)
    for (name, task, write, told) in [
        ("rec-fail", "{}", false, "was not saved"),
        ("rec-bad", "not json", true, "not a recorded task"),
    ] {
        let dir = tmp(name);
        let bin = fake_executor(&dir, task, write);
        let drafts = dir.join("drafts");
        let phase = Arc::new(Mutex::new(Phase::default()));
        let finished = begin(&phase, &bin, &drafts).unwrap();
        choose_stop_in_rec_menu(&dir);
        finished.recv_timeout(BOUND).unwrap();
        match &*phase.lock().unwrap() {
            Phase::Failed(why) => assert!(why.contains(told), "{name}: {why}"),
            _ => panic!("{name}: the failure was not kept for the officer"),
        }
        assert_eq!(drafts_in(&drafts), 0, "{name}: a refused recording does not linger");
        std::fs::remove_file(dir.join("ready")).unwrap();
        let again = begin(&phase, &bin, &drafts).unwrap();
        choose_stop_in_rec_menu(&dir);
        again.recv_timeout(BOUND).unwrap();
    }
}

#[test]
fn a_recorder_that_exits_with_an_error_after_writing_leaves_no_file() {
    let dir = tmp("rec-crash");
    let bin = dir.join("fake-crash");
    std::fs::write(&bin, "#!/bin/sh\nfor a; do out=\"$a\"; done\necho '{' > \"$out\"\nexit 1\n").unwrap();
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(&bin, std::fs::Permissions::from_mode(0o755)).unwrap();
    let drafts = dir.join("drafts");
    let (_rec, exit) = start(&bin, &drafts);
    assert!(exit.recv_timeout(BOUND).unwrap().unwrap_err().contains("was not saved"));
    assert_eq!(drafts_in(&drafts), 0);
}

#[test]
fn a_recorder_that_ignores_the_stop_is_ended_and_leaves_no_file() {
    let dir = tmp("rec-stuck");
    let bin = dir.join("fake-stuck");
    std::fs::write(
        &bin,
        format!("#!/bin/sh\ntrap '' INT\necho > \"{}/ready\"\nexec sleep 600\n", dir.display()),
    )
    .unwrap();
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(&bin, std::fs::Permissions::from_mode(0o755)).unwrap();
    let (rec, exit) = start(&bin, &dir.join("drafts"));
    wait_ready(&dir);
    let stopper = rec.stopper();
    stopper.stop_within(Duration::from_millis(200)).unwrap();
    assert!(exit.recv_timeout(BOUND).unwrap().unwrap_err().contains("did not stop in time"));
    // Stopping a recorder that has already gone signals nothing.
    stopper.stop().unwrap();
}

#[test]
fn a_recording_with_no_file_reports_failure_and_a_bad_bundle_never_starts() {
    let dir = tmp("rec-none");
    let bin = fake_executor(&dir, "{}", false);
    let (rec, exit) = start(&bin, &dir.join("drafts"));
    wait_ready(&dir);
    rec.stopper().stop().unwrap();
    assert!(exit.recv_timeout(BOUND).unwrap().is_err());
    for bad in ["", "--out", "a b", "x;rm", "../x"] {
        assert!(recording::start(&bin, bad, &dir.join("drafts"), |_| {}).is_err(), "{bad}");
    }
}

#[test]
fn drafts_folder_is_private() {
    let dir = tmp("rec-mode");
    let bin = fake_executor(&dir, &recorded().to_string(), true);
    let drafts = dir.join("drafts");
    let (rec, exit) = start(&bin, &drafts);
    use std::os::unix::fs::PermissionsExt;
    assert_eq!(std::fs::metadata(&drafts).unwrap().permissions().mode() & 0o777, 0o700);
    wait_ready(&dir);
    rec.stopper().stop().unwrap();
    let _ = exit.recv_timeout(BOUND);
}

#[test]
fn iso_dates_are_right() {
    assert_eq!(iso_utc(0), "1970-01-01T00:00:00Z");
    assert_eq!(iso_utc(1_709_251_199), "2024-02-29T23:59:59Z");
    assert_eq!(iso_utc(1_795_000_000), "2026-11-18T11:06:40Z");
}

#[test]
fn a_constant_value_is_never_copied_into_a_summary_even_after_its_step_is_deleted() {
    let dir = tmp("summary-leak");
    let mut d = draft();
    edit(&mut d, json!({"op":"mark_constant","step":"s1","value":SECRET,"confirmed_no_personal_data":true})).unwrap();
    // The view has no approval field, so read the summaries from a task saved while s1 still exists.
    // Its own store: that body rightly holds the constant in s1.
    let mut before = open(&tmp("summary-leak-before"));
    let (id, v) = d.clone_for_retry().save(&mut before, "officer", NOW).unwrap();
    let t: Value = serde_json::from_str(&before.task_body(&id, v).unwrap().unwrap()).unwrap();
    assert_eq!(step(&t, "s1")["value"]["constant"], SECRET);
    for s in ["s3", "s4"] {
        let summary = step(&t, s)["approval"]["summary"].as_str().unwrap();
        assert!(!summary.contains(SECRET), "{s}: not copied while the step exists");
    }
    edit(&mut d, json!({"op":"delete_step","step":"s1"})).unwrap();
    let mut store = open(&dir);
    let (id, v) = d.save(&mut store, "officer", NOW).unwrap();
    let body = store.task_body(&id, v).unwrap().unwrap();
    assert!(!body.contains(SECRET), "the value left with its step");
    assert!(validate(&body, &dir).0);
    assert!(!decrypted_dump(&dir).contains(SECRET));
}

fn saved_valid(t: Value, dir: &Path) -> Result<bool, String> {
    let mut s = open(dir);
    let d = Draft::from_json(&t.to_string(), None).unwrap();
    d.save(&mut s, "officer", NOW)?;
    Ok(validate(&s.task_body("tsk_ab12cd34", 1).unwrap().unwrap(), dir).0)
}

#[test]
fn a_summary_never_ends_up_empty() {
    let dir = tmp("empty-summary");
    let mut t = recorded();
    t["steps"][2]["approval"]["summary"] = json!("{full_name}");
    let mut d = Draft::from_json(&t.to_string(), None).unwrap();
    edit(&mut d, json!({"op":"delete_step","step":"s1"})).unwrap();
    edit(&mut d, json!({"op":"delete_step","step":"s2"})).unwrap();
    let mut s = open(&dir);
    d.save(&mut s, "officer", NOW).unwrap();
    assert!(validate(&s.task_body("tsk_ab12cd34", 1).unwrap().unwrap(), &dir).0);
    // A blank constant is refused outright.
    let mut d = draft();
    assert!(edit(&mut d, json!({"op":"mark_constant","step":"s1","value":"   ","confirmed_no_personal_data":true})).is_err());
}

#[test]
fn a_hand_made_recording_that_breaks_the_format_is_refused_at_save() {
    let dir = tmp("tampered");
    let cases: Vec<(&str, Box<dyn Fn(&mut Value)>)> = vec![
        ("task id", Box::new(|t| t["id"] = json!("../evil"))),
        ("step id", Box::new(|t| t["steps"][0]["id"] = json!("zzz"))),
        ("action", Box::new(|t| t["steps"][0]["action"] = json!("rm_rf"))),
        ("press with a value", Box::new(|t| t["steps"][2]["value"] = json!({"input":"email"}))),
        ("extra field", Box::new(|t| t["extra"] = json!(1))),
        ("platform", Box::new(|t| t["platform"] = json!("linux"))),
        ("summary not a string", Box::new(|t| t["steps"][2]["approval"]["summary"] = json!(5))),
        ("bad input name", Box::new(|t| t["inputs"][0]["name"] = json!("Bad Name"))),
        ("unknown without an approval", Box::new(|t| { t["steps"][2].as_object_mut().unwrap().remove("approval"); })),
        ("yes without an approval", Box::new(|t| { t["steps"][3].as_object_mut().unwrap().remove("approval"); })),
    ];
    for (what, f) in cases {
        let mut t = recorded();
        f(&mut t);
        assert!(saved_valid(t, &dir).is_err(), "{what} was saved");
    }
    assert_eq!(saved_valid(recorded(), &dir), Ok(true));
}

#[test]
fn a_constant_for_a_choice_input_must_be_one_of_its_choices() {
    let mut t = recorded();
    t["inputs"][0] = json!({"name":"full_name","type":"choice","source":"ask","choices":["a","b"]});
    let mut d = Draft::from_json(&t.to_string(), None).unwrap();
    assert!(edit(&mut d, json!({"op":"mark_constant","step":"s1","value":"zzz","confirmed_no_personal_data":true})).is_err());
    edit(&mut d, json!({"op":"mark_constant","step":"s1","value":"a","confirmed_no_personal_data":true})).unwrap();
}

#[test]
fn drafts_left_over_from_an_earlier_run_are_removed_at_launch() {
    let dir = tmp("launch-sweep");
    let drafts = dir.join("drafts");
    std::fs::create_dir_all(&drafts).unwrap();
    for f in ["draft-1-2.json", "draft-3-4.json", "keep.txt"] {
        std::fs::write(drafts.join(f), "{}").unwrap();
    }
    recording::sweep_at_launch(Some(drafts.clone()));
    let left: Vec<_> = std::fs::read_dir(&drafts).unwrap().map(|e| e.unwrap().file_name()).collect();
    assert_eq!(left, ["keep.txt"]);
    // A first launch has no drafts folder, and a launch with no home folder has no path.
    let absent = dir.join("absent");
    recording::sweep_at_launch(Some(absent.clone()));
    assert!(!absent.exists());
    recording::sweep_at_launch(None);
    // A drafts path that is a link must not make launch delete files in the folder it points to.
    let elsewhere = dir.join("elsewhere");
    std::fs::create_dir_all(&elsewhere).unwrap();
    std::fs::write(elsewhere.join("draft-mine.json"), "{}").unwrap();
    let linked = dir.join("linked-drafts");
    std::os::unix::fs::symlink(&elsewhere, &linked).unwrap();
    recording::sweep_at_launch(Some(linked));
    assert!(elsewhere.join("draft-mine.json").exists(), "launch deleted through a linked drafts folder");
}

#[test]
fn old_drafts_are_swept_at_start_and_an_existing_folder_is_made_private() {
    let dir = tmp("sweep");
    let drafts = dir.join("drafts");
    std::fs::create_dir_all(&drafts).unwrap();
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(&drafts, std::fs::Permissions::from_mode(0o755)).unwrap();
    std::fs::write(drafts.join("draft-old.json"), "{}").unwrap();
    std::fs::write(drafts.join("keep.txt"), "x").unwrap();
    let bin = fake_executor(&dir, &recorded().to_string(), true);
    let (rec, exit) = start(&bin, &drafts);
    assert!(!drafts.join("draft-old.json").exists(), "leftover drafts are swept at the next start");
    assert!(drafts.join("keep.txt").exists(), "only draft files are touched");
    assert_eq!(std::fs::metadata(&drafts).unwrap().permissions().mode() & 0o777, 0o700);
    wait_ready(&dir);
    rec.stopper().stop().unwrap();
    let _ = exit.recv_timeout(BOUND);
}
