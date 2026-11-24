//! Store tests. Synthetic data only. Keys are injected or a throwaway keychain item that is
//! deleted at the end of the test. Tests that attack the audit table use a second plain
//! connection (same key, no recursive_triggers) so they do not depend on the Store's own setup.

use companion_shell::store::*;
use rusqlite::Connection;
use std::path::{Path, PathBuf};

const DAY: i64 = 86_400;

fn tmp(name: &str) -> PathBuf {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let d = std::env::temp_dir().join(format!("companion-store-{name}-{}-{nanos}", std::process::id()));
    std::fs::create_dir_all(&d).unwrap();
    d
}

fn open(dir: &Path) -> Store {
    Store::open(&dir.join("s.db"), &Fixed([1u8; 32])).unwrap()
}

/// A separate connection to the same file, as code that bypasses `Store` would have.
fn side(dir: &Path) -> Connection {
    let c = Connection::open(dir.join("s.db")).unwrap();
    c.pragma_update(None, "key", format!("x'{}'", "01".repeat(32))).unwrap();
    c
}

fn entry(at: i64, shown: &str) -> AuditEntry<'_> {
    AuditEntry {
        at,
        officer: "officer-1",
        event: "approved",
        task: Some("tsk_x@1"),
        run: Some("run_1"),
        item: Some(1),
        step: Some("s1"),
        shown: Some(shown),
    }
}

fn schema(c: &Connection) -> String {
    c.query_row("SELECT group_concat(sql, ';') FROM sqlite_master", [], |r| r.get(0)).unwrap()
}

fn shown_of(c: &Connection, seq: i64) -> Option<String> {
    c.query_row("SELECT shown FROM audit WHERE seq = ?1", [seq], |r| r.get(0)).ok()
}

#[test]
fn migrations_create_the_tables_and_rerunning_changes_nothing() {
    let d = tmp("mig");
    let before = {
        let _s = open(&d);
        let c = side(&d);
        let names: Vec<String> = c
            .prepare("SELECT name FROM sqlite_master WHERE type='table'")
            .unwrap()
            .query_map([], |r| r.get(0))
            .unwrap()
            .map(|r| r.unwrap())
            .collect();
        for want in ["tasks", "runs", "run_items", "audit", "chat"] {
            assert!(names.contains(&want.to_string()), "missing {want}");
        }
        schema(&c)
    };
    {
        let mut s = open(&d);
        s.save_task("t1", "n", "{}", 1).unwrap();
        s.append_audit(&entry(1, "a")).unwrap();
    }
    for _ in 0..2 {
        let s = open(&d);
        assert_eq!(before, schema(&side(&d)));
        assert_eq!(s.task_versions("t1").unwrap(), vec![1]);
        assert_eq!(s.audit_count().unwrap(), 1);
        s.verify_audit().unwrap();
    }
}

#[test]
fn the_file_is_encrypted_and_a_wrong_key_fails() {
    let d = tmp("enc");
    {
        let mut s = open(&d);
        s.save_task("t1", "n", "{}", 1).unwrap();
    }
    let path = d.join("s.db");
    let plain = Connection::open(&path).unwrap();
    assert!(plain.query_row("SELECT count(*) FROM sqlite_master", [], |r| r.get::<_, i64>(0)).is_err());
    let bytes = std::fs::read(&path).unwrap();
    assert!(!bytes.starts_with(b"SQLite format 3"));
    assert!(Store::open(&path, &Fixed([2u8; 32])).is_err());
}

#[test]
fn saving_a_task_writes_a_new_version_and_keeps_the_old_ones() {
    let d = tmp("ver");
    let mut s = open(&d);
    assert_eq!(s.save_task("t1", "first", r#"{"v":1}"#, 1).unwrap(), 1);
    assert_eq!(s.save_task("t1", "second", r#"{"v":2}"#, 2).unwrap(), 2);
    assert_eq!(s.save_task("t2", "other", "{}", 3).unwrap(), 1);
    assert_eq!(s.save_task("t1", "third", r#"{"v":3}"#, 4).unwrap(), 3);
    assert_eq!(s.task_versions("t1").unwrap(), vec![1, 2, 3]);
    assert_eq!(s.task_body("t1", 1).unwrap().unwrap(), r#"{"v":1}"#);
    assert_eq!(s.task_body("t1", 3).unwrap().unwrap(), r#"{"v":3}"#);
}

#[test]
fn runs_items_and_chat_round_trip() {
    let d = tmp("run");
    let mut s = open(&d);
    s.save_task("t1", "n", "{}", 1).unwrap();
    s.start_run("run_1", "t1", 1, "ask", 2).unwrap();
    s.set_run_item("run_1", 1, "running", Some("s1")).unwrap();
    s.set_run_item("run_1", 1, "done", Some("s9")).unwrap();
    let state: String = side(&d)
        .query_row("SELECT state FROM run_items WHERE run_id='run_1' AND item=1", [], |r| r.get(0))
        .unwrap();
    assert_eq!(state, "done");
    s.add_chat(4, "user", "synthetic message").unwrap();
}

#[test]
fn audit_rejects_update_and_delete() {
    let d = tmp("trig");
    let mut s = open(&d);
    s.append_audit(&entry(100, "Create a staff account for Test Person")).unwrap();
    let c = side(&d);
    let upd = c.execute("UPDATE audit SET shown = 'edited'", []).unwrap_err();
    assert!(upd.to_string().contains("append-only"));
    let del = c.execute("DELETE FROM audit", []).unwrap_err();
    assert!(del.to_string().contains("append-only"));
    assert_eq!(s.audit_count().unwrap(), 1);
    assert_eq!(shown_of(&c, 1).unwrap(), "Create a staff account for Test Person");
}

#[test]
fn replace_cannot_rewrite_or_remove_an_audit_row() {
    let d = tmp("replace");
    let mut s = open(&d);
    s.append_audit(&entry(10, "amount=100")).unwrap();
    s.append_audit(&entry(11, "b")).unwrap();
    // `side` has recursive_triggers off (the default), so only the insert guard stops this.
    let c = side(&d);
    let r = c.execute(
        "INSERT OR REPLACE INTO audit (seq,at,officer,event,task,run,item,step,shown,prev_hash,hash)
         SELECT seq,at,officer,event,task,run,item,step,'amount=999',prev_hash,hash FROM audit WHERE seq=1",
        [],
    );
    assert!(r.is_err());
    assert!(c
        .execute("REPLACE INTO audit (seq,at,officer,event,prev_hash,hash) VALUES (1,10,'o','x','p','h')", [])
        .is_err());
    assert!(c
        .execute("REPLACE INTO audit (seq,at,officer,event,prev_hash,hash) VALUES (3,10,'o','x','p','h')", [])
        .is_err());
    assert_eq!(shown_of(&c, 1).unwrap(), "amount=100");
    assert_eq!(s.audit_count().unwrap(), 2);
    s.verify_audit().unwrap();
}

#[test]
fn audit_chain_verifies_and_detects_tampering() {
    let d = tmp("chain");
    let mut s = open(&d);
    for i in 0..3 {
        s.append_audit(&entry(100 + i, "v")).unwrap();
    }
    s.verify_audit().unwrap();
    // Someone with the key drops the triggers and edits a row.
    let c = side(&d);
    c.execute_batch("DROP TRIGGER audit_no_update").unwrap();
    c.execute("UPDATE audit SET shown = 'edited' WHERE seq = 2", []).unwrap();
    assert!(matches!(s.verify_audit(), Err(StoreError::Chain(2))));
}

#[test]
fn truncated_tail_and_full_wipe_fail_verification() {
    let d = tmp("trunc");
    let mut s = open(&d);
    for i in 0..3 {
        s.append_audit(&entry(100 + i, "v")).unwrap();
    }
    let c = side(&d);
    c.execute_batch("DROP TRIGGER audit_no_delete").unwrap();
    c.execute("DELETE FROM audit WHERE seq = 3", []).unwrap();
    assert!(s.verify_audit().is_err(), "tail truncation must be detected");
    c.execute("DELETE FROM audit", []).unwrap();
    assert!(s.verify_audit().is_err(), "a wipe must be detected");
}

#[test]
fn none_and_the_old_sentinel_string_hash_differently() {
    let a_dir = tmp("h1");
    let b_dir = tmp("h2");
    let mut a = open(&a_dir);
    let mut b = open(&b_dir);
    let base = AuditEntry { at: 1, officer: "o", event: "x", task: None, run: None, item: None, step: None, shown: None };
    a.append_audit(&base).unwrap();
    b.append_audit(&AuditEntry { task: Some("\u{0}none"), ..base }).unwrap();
    let ha: String = side(&a_dir).query_row("SELECT hash FROM audit", [], |r| r.get(0)).unwrap();
    let hb: String = side(&b_dir).query_row("SELECT hash FROM audit", [], |r| r.get(0)).unwrap();
    assert_ne!(ha, hb);
    // Some("") also differs from None.
    let c_dir = tmp("h3");
    let mut c = open(&c_dir);
    c.append_audit(&AuditEntry { task: Some(""), ..base }).unwrap();
    let hc: String = side(&c_dir).query_row("SELECT hash FROM audit", [], |r| r.get(0)).unwrap();
    assert_ne!(ha, hc);
}

#[test]
fn retention_purge_removes_the_expired_prefix_and_logs_itself() {
    let d = tmp("purge");
    let mut s = open(&d);
    let now = 10_000 * DAY;
    s.append_audit(&entry(now - 3000 * DAY, "old")).unwrap();
    s.append_audit(&entry(now - 2600 * DAY, "old2")).unwrap();
    s.append_audit(&entry(now - 10 * DAY, "recent")).unwrap();
    assert_eq!(DEFAULT_AUDIT_RETENTION_DAYS, 2555);
    assert_eq!(s.purge_expired(now, DEFAULT_AUDIT_RETENTION_DAYS).unwrap(), 2);
    // recent + the purge record
    assert_eq!(s.audit_count().unwrap(), 2);
    s.verify_audit().unwrap();
    let c = side(&d);
    let (who, ev, n): (String, String, i64) = c
        .query_row("SELECT officer, event, item FROM audit WHERE seq = 4", [], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
        .unwrap();
    assert_eq!((who.as_str(), ev.as_str(), n), ("system", "audit_purged", 2));
    // A second purge with nothing expired is a no-op and adds no record.
    assert_eq!(s.purge_expired(now, DEFAULT_AUDIT_RETENTION_DAYS).unwrap(), 0);
    assert_eq!(s.audit_count().unwrap(), 2);
    // Appending after a purge keeps verifying.
    s.append_audit(&entry(now, "after")).unwrap();
    s.verify_audit().unwrap();
}

#[test]
fn purge_only_takes_a_contiguous_prefix_so_the_chain_never_has_a_gap() {
    let d = tmp("prefix");
    let mut s = open(&d);
    let now = 10_000 * DAY;
    // Out-of-order timestamps: seq 2 is old but sits behind a recent record.
    s.append_audit(&entry(now - 3000 * DAY, "old")).unwrap();
    s.append_audit(&entry(now - 5 * DAY, "recent")).unwrap();
    s.append_audit(&entry(now - 3000 * DAY, "old but later")).unwrap();
    assert_eq!(s.purge_expired(now, 2555).unwrap(), 1);
    s.verify_audit().unwrap();
    let c = side(&d);
    assert!(shown_of(&c, 3).is_some(), "later old row is kept, not punched out of the middle");
}

#[test]
fn purge_cannot_wipe_recent_records_or_use_zero_retention() {
    let d = tmp("cap");
    let mut s = open(&d);
    let now = 10_000 * DAY;
    s.append_audit(&entry(now - DAY, "yesterday")).unwrap();
    s.append_audit(&entry(now, "today")).unwrap();
    assert!(s.purge_expired(now, 0).is_err());
    assert_eq!(s.purge_expired(now, 30).unwrap(), 0);
    assert_eq!(s.audit_count().unwrap(), 2);
    // Even with a huge `now` the age rule decides, and the purge is on the record afterwards.
    assert_eq!(s.purge_expired(now + 100 * DAY, 30).unwrap(), 2);
    s.verify_audit().unwrap();
    assert_eq!(s.audit_count().unwrap(), 1, "only the audit_purged record remains");
}

#[test]
fn the_purge_gate_and_control_row_cannot_be_driven_from_sql() {
    let d = tmp("gate");
    let mut s = open(&d);
    s.append_audit(&entry(10, "a")).unwrap();
    s.append_audit(&entry(20, "b")).unwrap();
    let c = side(&d);
    // Open the gate to everything by hand: refused (anchor must match the purged row).
    assert!(c.execute("UPDATE audit_meta SET purge_upto = 9223372036854775807", []).is_err());
    assert!(c.execute("UPDATE audit_meta SET purge_upto = 2", []).is_err());
    // Rewind or forge the head, replace or delete the row.
    assert!(c.execute("UPDATE audit_meta SET head_seq = 0", []).is_err());
    assert!(c.execute("UPDATE audit_meta SET head_hash = 'x'", []).is_err());
    assert!(c.execute("UPDATE audit_meta SET anchor_hash = 'x'", []).is_err());
    assert!(c.execute("DELETE FROM audit_meta", []).is_err());
    assert!(c.execute("INSERT OR REPLACE INTO audit_meta VALUES (1,0,'0',0,'0',9223372036854775807)", []).is_err());
    assert!(c.execute("DELETE FROM audit", []).is_err());
    assert_eq!(s.audit_count().unwrap(), 2);
    s.verify_audit().unwrap();
    // After a real purge the gate is closed again.
    assert_eq!(s.purge_expired(30 * DAY, 1).unwrap(), 2);
    let gate: Option<i64> = c.query_row("SELECT purge_upto FROM audit_meta", [], |r| r.get(0)).unwrap();
    assert!(gate.is_none());
    assert!(c.execute("DELETE FROM audit", []).is_err());
}

#[test]
fn errors_do_not_carry_values() {
    let d = tmp("err");
    let mut s = open(&d);
    s.append_audit(&entry(1, "SECRET-SYNTHETIC-VALUE")).unwrap();
    let e = side(&d).execute("UPDATE audit SET shown = 'SECRET-SYNTHETIC-VALUE2'", []).unwrap_err();
    assert!(!StoreError::from(e).to_string().contains("SECRET-SYNTHETIC-VALUE"));
}

#[test]
fn the_key_comes_from_the_keychain_and_is_stable() {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let kc = Keychain {
        service: format!("sg.gov.grl.companion.test-{}-{nanos}", std::process::id()),
        account: "throwaway".into(),
    };
    let d = tmp("kc");
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        {
            let mut s = Store::open(&d.join("s.db"), &kc).unwrap();
            s.save_task("t1", "n", "{}", 1).unwrap();
        }
        let s = Store::open(&d.join("s.db"), &kc).unwrap();
        assert_eq!(s.task_versions("t1").unwrap(), vec![1]);
    }));
    kc.delete().unwrap();
    assert!(kc.delete().is_err(), "item must be gone after cleanup");
    if let Err(p) = result {
        std::panic::resume_unwind(p);
    }
}
