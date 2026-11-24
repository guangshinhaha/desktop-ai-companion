//! The audit trail end to end: the store writes it, `executor-macos verify-audit` checks it.
//! Synthetic data only. Keys are injected, so no test touches the login keychain.
//! The verifier tests need the built executor; scripts/test.sh sets EXECUTOR_BIN.

use companion_shell::executor::verify_audit;
use companion_shell::store::{AuditEntry, Fixed, Store, StoreError};
use rusqlite::Connection;
use std::path::{Path, PathBuf};

fn tmp(name: &str) -> PathBuf {
    let n = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos();
    let d = std::env::temp_dir().join(format!("companion-audit-{name}-{}-{n}", std::process::id()));
    std::fs::create_dir_all(&d).unwrap();
    d
}

fn open(dir: &Path) -> Store {
    Store::open(&dir.join("s.db"), &Fixed([3u8; 32])).unwrap()
}

fn side(dir: &Path) -> Connection {
    let c = Connection::open(dir.join("s.db")).unwrap();
    c.pragma_update(None, "key", format!("x'{}'", "03".repeat(32))).unwrap();
    c
}

/// A run as the shell records it: start, an approval with values, a rejection, a divergence, the end.
/// Includes an empty string and absent fields, to cover absent versus empty in the hash.
fn trail(s: &mut Store) {
    let e = |at, event, step, shown, item| AuditEntry {
        at,
        officer: "Test Officer",
        event,
        task: Some("tsk_x@1"),
        run: Some("run_1"),
        item,
        step,
        shown,
    };
    s.append_event(&e(1_700_000_000, "run_started", None, None, None)).unwrap();
    s.append_event(&e(
        1_700_000_005,
        "approved",
        Some("s5"),
        Some("Create a staff account for Test Person (test.person@example.test), Teacher at Test School"),
        Some(1),
    ))
    .unwrap();
    s.append_event(&e(1_700_000_009, "rejected", Some("s9"), Some("Päivä \u{4e2d}\u{6587} \"quoted\" \\ line\nbreak"), None)).unwrap();
    s.append_event(&e(1_700_000_010, "step_diverged", Some("s9"), Some(""), None)).unwrap();
    s.append_event(&e(1_700_000_011, "run_finished", None, Some("diverged"), None)).unwrap();
}

fn executor() -> Option<PathBuf> {
    let p = std::env::var_os("EXECUTOR_BIN").map(PathBuf::from);
    if p.is_none() {
        eprintln!("EXECUTOR_BIN not set: skipping (scripts/test.sh sets it)");
    }
    p
}

/// The Swift verifier's fixture is made by this store, so the two hash implementations are tied together.
/// Set REGEN_AUDIT_FIXTURE=1 to rewrite it after an intended change.
#[test]
fn the_swift_fixture_is_exactly_what_the_store_exports() {
    let d = tmp("fixture");
    let mut s = open(&d);
    trail(&mut s);
    let export = s.export_audit().unwrap();
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../executor-macos/Tests/fixtures/audit/trail.jsonl");
    if std::env::var_os("REGEN_AUDIT_FIXTURE").is_some() {
        std::fs::write(&path, &export).unwrap();
    }
    assert_eq!(std::fs::read_to_string(&path).unwrap(), export);
}

#[test]
fn an_untouched_trail_passes_the_executors_verifier() {
    let Some(bin) = executor() else { return };
    let d = tmp("ok");
    let mut s = open(&d);
    trail(&mut s);
    s.verify_audit().unwrap();
    let out = verify_audit(&bin, &s.export_audit().unwrap()).unwrap();
    assert!(out.contains("intact: 5 records"), "{out}");
}

#[test]
fn an_edited_record_fails_and_names_its_sequence_number() {
    let Some(bin) = executor() else { return };
    let d = tmp("edit");
    let mut s = open(&d);
    trail(&mut s);
    // Someone with the key drops the trigger and changes what was approved.
    let c = side(&d);
    c.execute_batch("DROP TRIGGER audit_no_update").unwrap();
    c.execute("UPDATE audit SET shown = 'Create a staff account for Someone Else' WHERE seq = 2", []).unwrap();
    assert!(matches!(s.verify_audit(), Err(StoreError::Chain(2))));
    let err = verify_audit(&bin, &s.export_audit().unwrap()).unwrap_err();
    assert!(err.contains("broken at seq 2"), "{err}");
    // Editing the officer, the event, the time, the run, the step or the item is caught the same way.
    for (col, val) in [("officer", "'Other Officer'"), ("event", "'approved'"), ("at", "5"), ("run", "'run_2'"), ("step", "NULL"), ("item", "9")] {
        let d = tmp("edit2");
        let mut s = open(&d);
        trail(&mut s);
        let c = side(&d);
        c.execute_batch("DROP TRIGGER audit_no_update").unwrap();
        c.execute(&format!("UPDATE audit SET {col} = {val} WHERE seq = 3"), []).unwrap();
        let err = verify_audit(&bin, &s.export_audit().unwrap()).unwrap_err();
        assert!(err.contains("broken at seq 3"), "{col}: {err}");
    }
}

#[test]
fn a_removed_record_fails_and_names_its_sequence_number() {
    let Some(bin) = executor() else { return };
    for (seq, name) in [(3, "rm-mid"), (5, "rm-tail"), (1, "rm-first")] {
        let d = tmp(name);
        let mut s = open(&d);
        trail(&mut s);
        let c = side(&d);
        c.execute_batch("DROP TRIGGER audit_no_delete").unwrap();
        c.execute("DELETE FROM audit WHERE seq = ?1", [seq]).unwrap();
        assert!(matches!(s.verify_audit(), Err(StoreError::Chain(n)) if n == seq));
        let err = verify_audit(&bin, &s.export_audit().unwrap()).unwrap_err();
        assert!(err.contains(&format!("broken at seq {seq}")), "{name}: {err}");
    }
}

#[test]
fn an_export_edited_after_the_fact_fails_too() {
    let Some(bin) = executor() else { return };
    let d = tmp("export");
    let mut s = open(&d);
    trail(&mut s);
    let export = s.export_audit().unwrap();
    let edited = export.replacen("Test Person", "Test Persoo", 1);
    assert_ne!(edited, export);
    let err = verify_audit(&bin, &edited).unwrap_err();
    assert!(err.contains("broken at seq 2"), "{err}");
    let lines: Vec<&str> = export.lines().collect();
    let without_second = [lines[0], lines[1], lines[3], lines[4], lines[5]].join("\n");
    assert!(verify_audit(&bin, &without_second).unwrap_err().contains("broken at seq 2"));
}

#[test]
fn append_event_refuses_names_outside_the_td_list() {
    let d = tmp("names");
    let mut s = open(&d);
    let mk = |event| AuditEntry { at: 1, officer: "o", event, task: None, run: None, item: None, step: None, shown: None };
    for bad in ["", "approve", "Approved", "audit_purged", "made_up"] {
        assert!(s.append_event(&mk(bad)).is_err(), "{bad}");
    }
    for ok in [
        "run_started",
        "approved",
        "rejected",
        "step_diverged",
        "recovered",
        "run_finished",
        "item_marked_by_officer",
        "task_saved",
        "task_shared",
        "task_handed_over",
    ] {
        s.append_event(&mk(ok)).unwrap();
    }
    assert_eq!(s.audit_count().unwrap(), 10);
    s.verify_audit().unwrap();
}
