//! Store tests. Synthetic data only. Keys are injected or a throwaway keychain item that is
//! deleted at the end of the test.

use companion_shell::store::*;
use std::path::PathBuf;

fn tmp(name: &str) -> PathBuf {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let d = std::env::temp_dir().join(format!("companion-store-{name}-{}-{nanos}", std::process::id()));
    std::fs::create_dir_all(&d).unwrap();
    d
}

fn open(dir: &std::path::Path) -> Store {
    Store::open(&dir.join("s.db"), &Fixed([1u8; 32])).unwrap()
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

fn tables(s: &Store) -> Vec<String> {
    let c = s.raw();
    let mut st = c
        .prepare("SELECT name FROM sqlite_master WHERE type='table' AND name NOT LIKE 'sqlite_%' ORDER BY name")
        .unwrap();
    st.query_map([], |r| r.get(0)).unwrap().map(|r| r.unwrap()).collect()
}

#[test]
fn migrations_create_the_tables_and_rerunning_changes_nothing() {
    let d = tmp("mig");
    let before = {
        let s = open(&d);
        let t = tables(&s);
        for want in ["tasks", "runs", "run_items", "audit", "chat"] {
            assert!(t.contains(&want.to_string()), "missing {want}");
        }
        let schema: String = s
            .raw()
            .query_row("SELECT group_concat(sql, ';') FROM sqlite_master", [], |r| r.get(0))
            .unwrap();
        schema
    };
    // Reopen twice more: migrations run again and must change nothing, rows included.
    {
        let mut s = open(&d);
        s.save_task("t1", "n", "{}", 1).unwrap();
    }
    for _ in 0..2 {
        let s = open(&d);
        let after: String = s
            .raw()
            .query_row("SELECT group_concat(sql, ';') FROM sqlite_master", [], |r| r.get(0))
            .unwrap();
        assert_eq!(before, after);
        assert_eq!(s.task_versions("t1").unwrap(), vec![1]);
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
    // No key: SQLite cannot read it.
    let plain = rusqlite::Connection::open(&path).unwrap();
    assert!(plain.query_row("SELECT count(*) FROM sqlite_master", [], |r| r.get::<_, i64>(0)).is_err());
    // The header is not the plain SQLite magic.
    let bytes = std::fs::read(&path).unwrap();
    assert!(!bytes.starts_with(b"SQLite format 3"));
    // Wrong key: open fails.
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
    let state: String = s
        .raw()
        .query_row("SELECT state FROM run_items WHERE run_id='run_1' AND item=1", [], |r| r.get(0))
        .unwrap();
    assert_eq!(state, "done");
    // A run must point at a real task version.
    s.raw().execute_batch("PRAGMA foreign_keys = ON").unwrap();
    assert!(s.start_run("run_2", "t1", 99, "ask", 3).is_err());
    s.add_chat(4, "user", "synthetic message").unwrap();
}

#[test]
fn audit_rejects_update_and_delete() {
    let d = tmp("trig");
    let mut s = open(&d);
    s.append_audit(&entry(100, "Create a staff account for Test Person")).unwrap();
    let c = s.raw();
    let upd = c.execute("UPDATE audit SET shown = 'edited'", []);
    assert!(upd.is_err(), "update must be rejected");
    assert!(upd.unwrap_err().to_string().contains("append-only"));
    let del = c.execute("DELETE FROM audit", []);
    assert!(del.is_err(), "delete must be rejected");
    assert!(del.unwrap_err().to_string().contains("append-only"));
    assert_eq!(s.audit_count().unwrap(), 1);
    let shown: String = s.raw().query_row("SELECT shown FROM audit", [], |r| r.get(0)).unwrap();
    assert_eq!(shown, "Create a staff account for Test Person");
}

#[test]
fn audit_chain_verifies_and_detects_tampering() {
    let d = tmp("chain");
    let mut s = open(&d);
    for i in 0..3 {
        s.append_audit(&entry(100 + i, "v")).unwrap();
    }
    s.verify_audit().unwrap();
    // Tamper by dropping the triggers (what someone with the key could do).
    s.raw().execute_batch("DROP TRIGGER audit_no_update").unwrap();
    s.raw().execute("UPDATE audit SET shown = 'edited' WHERE seq = 2", []).unwrap();
    assert!(matches!(s.verify_audit(), Err(StoreError::Chain(2))));
}

#[test]
fn retention_purge_removes_only_expired_rows_and_is_not_an_edit_path() {
    let d = tmp("purge");
    let mut s = open(&d);
    let day = 86_400;
    let now = 10_000 * day;
    s.append_audit(&entry(now - 3000 * day, "old")).unwrap();
    s.append_audit(&entry(now - 2600 * day, "old2")).unwrap();
    s.append_audit(&entry(now - 10 * day, "recent")).unwrap();
    assert_eq!(DEFAULT_AUDIT_RETENTION_DAYS, 2555);
    let removed = s.purge_expired(now, DEFAULT_AUDIT_RETENTION_DAYS).unwrap();
    assert_eq!(removed, 2);
    assert_eq!(s.audit_count().unwrap(), 1);
    // The surviving record still verifies (first survivor is the anchor).
    s.verify_audit().unwrap();
    // A shorter configured retention purges more.
    assert_eq!(s.purge_expired(now, 5).unwrap(), 1);
    assert_eq!(s.audit_count().unwrap(), 0);
}

#[test]
fn the_purge_gate_closes_afterwards_so_deletes_and_updates_stay_rejected() {
    let d = tmp("gate");
    let mut s = open(&d);
    s.append_audit(&entry(10, "a")).unwrap();
    s.append_audit(&entry(20, "b")).unwrap();
    assert_eq!(s.purge_audit_before(15).unwrap(), 1);
    // Gate closed: plain delete of the remaining row fails, even though its age is below any cutoff.
    assert!(s.raw().execute("DELETE FROM audit", []).is_err());
    assert!(s.raw().execute("UPDATE audit SET shown = 'x'", []).is_err());
    // A purge cannot reach rows at or after the cutoff.
    assert_eq!(s.purge_audit_before(20).unwrap(), 0);
    assert_eq!(s.audit_count().unwrap(), 1);
    // Purge with nothing expired is a no-op, not an error.
    assert_eq!(s.purge_audit_before(i64::MIN + 1).unwrap(), 0);
}

#[test]
fn errors_do_not_carry_values() {
    let d = tmp("err");
    let mut s = open(&d);
    s.append_audit(&entry(1, "SECRET-SYNTHETIC-VALUE")).unwrap();
    let e = s.raw().execute("UPDATE audit SET shown = 'SECRET-SYNTHETIC-VALUE2'", []).unwrap_err();
    let msg = StoreError::from(e).to_string();
    assert!(!msg.contains("SECRET-SYNTHETIC-VALUE"));
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
        // Second open reads the same key back from the keychain.
        let s = Store::open(&d.join("s.db"), &kc).unwrap();
        assert_eq!(s.task_versions("t1").unwrap(), vec![1]);
    }));
    kc.delete().unwrap();
    assert!(Keychain::delete(&kc).is_err(), "item must be gone after cleanup");
    if let Err(p) = result {
        std::panic::resume_unwind(p);
    }
}
