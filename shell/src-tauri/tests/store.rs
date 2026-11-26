//! Store tests. Synthetic data only. Keys are injected, or come from an in-memory keychain
//! backend. The two tests that use the real macOS keychain are `#[ignore]`d so CI never touches
//! a (possibly locked or prompting) login keychain; run them on a Mac you are signed in to with
//! `cd shell/src-tauri && cargo test --test store -- --ignored`. Every test has a hard timeout. Tests that attack the audit table use a second plain
//! connection (same key, no recursive_triggers) so they do not depend on the Store's own setup.

use companion_shell::store::*;
use rusqlite::Connection;
use std::path::{Path, PathBuf};

const DAY: i64 = 86_400;

/// Runs a test body on its own thread and fails fast if it does not finish, so a blocked
/// keychain call cannot hold CI for the job timeout.
fn guarded<F: FnOnce() + Send + 'static>(secs: u64, f: F) {
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(f));
        let _ = tx.send(r);
    });
    match rx.recv_timeout(std::time::Duration::from_secs(secs)) {
        Ok(Ok(())) => {}
        Ok(Err(p)) => std::panic::resume_unwind(p),
        Err(_) => panic!("test timed out after {secs}s (blocked keychain call?)"),
    }
}

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
#[ignore = "touches the real login keychain; run locally with -- --ignored"]
fn the_key_comes_from_the_keychain_and_is_stable() {
    guarded(60, the_key_comes_from_the_keychain_and_is_stable_body);
}

fn the_key_comes_from_the_keychain_and_is_stable_body() {
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

struct NoKey;
impl KeySource for NoKey {
    fn key_and_origin(&self) -> Result<(Key, bool)> {
        Err(StoreError::Key("no key".into()))
    }
}

/// In-memory keychain: same add-only/duplicate semantics, a pause between the read and the add
/// to force the race.
struct MemKeychain(std::sync::Mutex<Option<Vec<u8>>>);
impl KeychainBackend for MemKeychain {
    fn get(&self) -> Option<Vec<u8>> {
        self.0.lock().unwrap().clone()
    }
    fn add(&self, data: &[u8]) -> AddOutcome {
        std::thread::sleep(std::time::Duration::from_millis(20));
        let mut g = self.0.lock().unwrap();
        if g.is_some() {
            return AddOutcome::Duplicate;
        }
        *g = Some(data.to_vec());
        AddOutcome::Added
    }
}
struct Shared(std::sync::Arc<MemKeychain>);
impl KeySource for Shared {
    fn key_and_origin(&self) -> Result<(Key, bool)> {
        key_via(&*self.0)
    }
}

#[test]
fn concurrent_first_opens_end_with_one_key_in_memory() {
    guarded(60, || {
        use std::sync::{Arc, Barrier};
        let kc = Arc::new(MemKeychain(std::sync::Mutex::new(None)));
        let n = 8;
        let barrier = Arc::new(Barrier::new(n));
        let dirs: Vec<PathBuf> = (0..n).map(|i| tmp(&format!("memrace{i}"))).collect();
        let handles: Vec<_> = dirs
            .iter()
            .cloned()
            .map(|d| {
                let (b, kc) = (barrier.clone(), kc.clone());
                std::thread::spawn(move || {
                    b.wait();
                    Store::open(&d.join("s.db"), &Shared(kc)).map(|_| ()).map_err(|e| e.to_string())
                })
            })
            .collect();
        let results: Vec<_> = handles.into_iter().map(|h| h.join().unwrap()).collect();
        assert!(results.iter().all(|r| r.is_ok()), "first opens: {results:?}");
        // Every file opens with the one stored key: none was keyed with a loser's key.
        assert!(dirs.iter().all(|d| Store::open(&d.join("s.db"), &Shared(kc.clone())).is_ok()));
    });
}

#[test]
fn a_wrong_length_or_failing_keychain_is_an_error_not_a_new_key() {
    let bad = MemKeychain(std::sync::Mutex::new(Some(vec![1, 2, 3])));
    assert!(key_via(&bad).is_err());
    let fresh = MemKeychain(std::sync::Mutex::new(None));
    assert!(key_via(&fresh).unwrap().1, "first call creates");
    assert!(!key_via(&fresh).unwrap().1, "second call reads");
}

#[test]
#[ignore = "touches the real login keychain; run locally with -- --ignored"]
fn concurrent_first_opens_end_with_one_key() {
    guarded(60, concurrent_first_opens_end_with_one_key_body);
}

fn concurrent_first_opens_end_with_one_key_body() {
    use std::sync::{Arc, Barrier};
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let service = format!("sg.gov.grl.companion.test-race-{}-{nanos}", std::process::id());
    let n = 8;
    let barrier = Arc::new(Barrier::new(n));
    let dirs: Vec<PathBuf> = (0..n).map(|i| tmp(&format!("race{i}"))).collect();
    let handles: Vec<_> = dirs
        .iter()
        .cloned()
        .map(|d| {
            let (b, service) = (barrier.clone(), service.clone());
            std::thread::spawn(move || {
                let kc = Keychain { service, account: "throwaway".into() };
                b.wait();
                Store::open(&d.join("s.db"), &kc).map(|_| ()).map_err(|e| e.to_string())
            })
        })
        .collect();
    let results: Vec<_> = handles.into_iter().map(|h| h.join().unwrap()).collect();
    let kc = Keychain { service, account: "throwaway".into() };
    // Every file opens with the one key now in the keychain: none was keyed with a loser's key.
    let reopened: Vec<bool> =
        dirs.iter().map(|d| Store::open(&d.join("s.db"), &kc).is_ok()).collect();
    kc.delete().unwrap();
    assert!(results.iter().all(|r| r.is_ok()), "first opens: {results:?}");
    assert!(reopened.iter().all(|ok| *ok), "all stores share one key: {reopened:?}");
}

#[test]
fn a_key_failure_leaves_nothing_on_disk() {
    let d = tmp("nokey");
    let dir = d.join("sub");
    assert!(Store::open(&dir.join("s.db"), &NoKey).is_err());
    assert!(!dir.exists(), "no directory or zero-byte file after a key failure");
}

#[test]
fn an_empty_store_file_with_an_existing_key_is_an_error() {
    let d = tmp("empty");
    std::fs::write(d.join("s.db"), b"").unwrap();
    assert!(Store::open(&d.join("s.db"), &Fixed([1u8; 32])).is_err());
    assert_eq!(std::fs::metadata(d.join("s.db")).unwrap().len(), 0, "left untouched");
}

#[test]
fn the_directory_is_private_and_the_files_are_owner_only() {
    use std::os::unix::fs::PermissionsExt;
    let d = tmp("perm");
    let dir = d.join("app");
    let mut s = Store::open(&dir.join("s.db"), &Fixed([1u8; 32])).unwrap();
    s.append_audit(&entry(1, "a")).unwrap();
    s.save_task("t", "n", "{}", 1).unwrap();
    let mode = |p: &Path| std::fs::metadata(p).unwrap().permissions().mode() & 0o777;
    assert_eq!(mode(&dir), 0o700);
    for f in std::fs::read_dir(&dir).unwrap() {
        let f = f.unwrap().path();
        assert_eq!(mode(&f), 0o600, "{}", f.display());
    }
}

/// Runs each job on its own thread with its own `Store` on one file, as the shell's commands do, all
/// released together. Returns every error from every thread.
type Job = Box<dyn FnOnce(&mut Store) -> Vec<String> + Send>;

fn race(dir: &Path, jobs: Vec<Job>) -> Vec<String> {
    let barrier = std::sync::Arc::new(std::sync::Barrier::new(jobs.len()));
    let handles: Vec<_> = jobs
        .into_iter()
        .map(|job| {
            let mut s = open(dir);
            let b = barrier.clone();
            std::thread::spawn(move || {
                b.wait();
                job(&mut s)
            })
        })
        .collect();
    handles.into_iter().flat_map(|h| h.join().unwrap()).collect()
}

#[test]
fn concurrent_opens_of_an_unmigrated_store_all_succeed() {
    guarded(120, || {
        for round in 0..20 {
            let d = tmp(&format!("race-open-{round}"));
            // A non-empty encrypted file at schema version 0. Racing the creation of the file itself
            // is a separate gap: the empty-file guard refuses the losers.
            let raw = Connection::open(d.join("s.db")).unwrap();
            raw.pragma_update(None, "key", format!("x'{}'", "01".repeat(32))).unwrap();
            raw.execute_batch("CREATE TABLE seed (x); DROP TABLE seed;").unwrap();
            drop(raw);
            let barrier = std::sync::Arc::new(std::sync::Barrier::new(4));
            let handles: Vec<_> = (0..4)
                .map(|_| {
                    let (d, b) = (d.clone(), barrier.clone());
                    std::thread::spawn(move || {
                        b.wait();
                        Store::open(&d.join("s.db"), &Fixed([1u8; 32])).err().map(|e| e.to_string())
                    })
                })
                .collect();
            let errors: Vec<String> = handles.into_iter().filter_map(|h| h.join().unwrap()).collect();
            assert!(errors.is_empty(), "round {round}: {errors:?}");
            open(&d).verify_audit().unwrap();
        }
    });
}

#[test]
fn concurrent_audit_appends_all_succeed() {
    guarded(120, || {
        let d = tmp("race-audit");
        drop(open(&d));
        let jobs = (0..4)
            .map(|_| -> Job {
                Box::new(|s| (0..50).filter_map(|i| s.append_audit(&entry(i, "a")).err().map(|e| e.to_string())).collect())
            })
            .collect();
        let errors = race(&d, jobs);
        assert!(errors.is_empty(), "{} errors, first: {}", errors.len(), errors[0]);
        let s = open(&d);
        assert_eq!(s.audit_count().unwrap(), 200);
        s.verify_audit().unwrap();
    });
}

#[test]
fn concurrent_saves_of_one_task_get_every_version_once() {
    guarded(120, || {
        let d = tmp("race-save");
        drop(open(&d));
        let jobs = (0..4)
            .map(|_| -> Job {
                Box::new(|s| (0..25).filter_map(|i| s.save_task("t", "n", "{}", i).err().map(|e| e.to_string())).collect())
            })
            .collect();
        let errors = race(&d, jobs);
        assert!(errors.is_empty(), "{} errors, first: {}", errors.len(), errors[0]);
        assert_eq!(open(&d).task_versions("t").unwrap(), (1..=100).collect::<Vec<i64>>());
    });
}

#[test]
fn audited_saves_and_replay_outcomes_succeed_while_a_purge_runs() {
    guarded(120, || {
        const N: i64 = 50;
        let d = tmp("race-purge");
        {
            let mut s = open(&d);
            s.save_task("seed", "n", "{}", 1).unwrap();
            for _ in 0..20 {
                s.append_audit(&entry(1, "old")).unwrap();
            }
        }
        let writer: Job = Box::new(|s| {
            let mut errors = Vec::new();
            for i in 0..N {
                if let Err(e) = s.save_task_audited("t", "n", "officer-1", 1, |_| Ok("{}".into())) {
                    errors.push(format!("save_task_audited: {e}"));
                }
                let done = ["s1".to_string()];
                if let Err(e) = s.record_replay_outcome("seed", 1, "finished", None, false, &done, i) {
                    errors.push(format!("record_replay_outcome: {e}"));
                }
            }
            errors
        });
        // `now` moves on each pass, so the last purge's own record is old enough to go next time.
        let purger: Job = Box::new(|s| {
            (1..=N)
                .filter_map(|i| s.purge_expired(i * 10 * DAY, 1).err().map(|e| format!("purge_expired: {e}")))
                .collect()
        });
        let errors = race(&d, vec![writer, purger]);
        assert!(errors.is_empty(), "{} errors, first: {}", errors.len(), errors[0]);
        let s = open(&d);
        assert_eq!(s.task_versions("t").unwrap(), (1..=N).collect::<Vec<i64>>());
        assert_eq!(s.version_states("seed").unwrap()[0].state, "verified");
        s.verify_audit().unwrap();
    });
}
