//! Notes on tasks (#44). Synthetic data only. Keys are injected, so no test touches the keychain.

use companion_shell::notes::{detail, LocalNotes, NoteBackend, MAX_NOTE_CHARS};
use companion_shell::store::{Fixed, Store};
use serde_json::json;
use std::path::PathBuf;

fn dir(name: &str) -> PathBuf {
    let n = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos();
    let d = std::env::temp_dir().join(format!("companion-notes-{name}-{}-{n}", std::process::id()));
    std::fs::create_dir_all(&d).unwrap();
    d
}

fn task_body() -> String {
    json!({ "name": "Add test account", "steps": [
        { "id": "s1", "say": "Type into Full name" },
        { "id": "s2", "say": "Press Create account" } ] })
    .to_string()
}

fn seeded(d: &std::path::Path) -> Store {
    let mut s = Store::open(&d.join("s.db"), &Fixed([7u8; 32])).unwrap();
    s.save_task("tsk_a", "Add test account", &task_body(), 10).unwrap();
    s
}

#[test]
fn a_note_shows_beside_the_step_list_oldest_first() {
    let d = dir("show");
    let s = seeded(&d);
    let n = LocalNotes(&s);
    n.add("tsk_a", "officer-1", "  Use the test org only.  ", 100).unwrap();
    n.add("tsk_a", "officer-2", "Decided: skip the welcome email.", 200).unwrap();
    let v = detail(&s, &n, "tsk_a").unwrap();
    assert_eq!(v["steps"].as_array().unwrap().len(), 2);
    assert_eq!(v["steps"][0]["say"], "Type into Full name");
    let notes = v["notes"].as_array().unwrap();
    assert_eq!(notes.len(), 2);
    assert_eq!(notes[0]["body"], "Use the test org only.", "trimmed");
    assert_eq!(notes[0]["author"], "officer-1");
    assert_eq!(notes[1]["body"], "Decided: skip the welcome email.");
    let _ = std::fs::remove_dir_all(&d);
}

#[test]
fn notes_travel_with_the_task_across_versions_and_stay_on_their_own_task() {
    let d = dir("versions");
    let mut s = seeded(&d);
    s.save_task("tsk_b", "Other task", &task_body(), 11).unwrap();
    LocalNotes(&s).add("tsk_a", "officer-1", "why we do it", 100).unwrap();
    s.save_task("tsk_a", "Add test account", &task_body(), 20).unwrap();
    let n = LocalNotes(&s);
    let v = detail(&s, &n, "tsk_a").unwrap();
    assert_eq!(v["version"], 2);
    assert_eq!(v["notes"].as_array().unwrap().len(), 1);
    assert!(detail(&s, &n, "tsk_b").unwrap()["notes"].as_array().unwrap().is_empty());
    let _ = std::fs::remove_dir_all(&d);
}

#[test]
fn bad_notes_are_refused_and_errors_never_echo_the_text() {
    let d = dir("refuse");
    let s = seeded(&d);
    let n = LocalNotes(&s);
    assert!(n.add("tsk_a", "officer-1", "   ", 1).is_err());
    assert!(n.add("tsk_a", "  ", "text", 1).is_err());
    let long = "x".repeat(MAX_NOTE_CHARS + 1);
    let e = n.add("tsk_a", "officer-1", &long, 1).unwrap_err();
    assert!(!e.contains(&long));
    let secret = "Jane Tan S1234567A";
    let e = n.add("tsk_missing", "officer-1", secret, 1).unwrap_err();
    assert!(!e.contains("Jane") && !e.contains("S1234567A"), "error: {e}");
    assert!(n.list("tsk_a").unwrap().is_empty());
    let _ = std::fs::remove_dir_all(&d);
}

#[test]
fn an_officer_can_remove_a_note() {
    let d = dir("remove");
    let s = seeded(&d);
    let n = LocalNotes(&s);
    let id = n.add("tsk_a", "officer-1", "temp", 1).unwrap();
    assert!(!n.remove("tsk_other", id).unwrap(), "only through its own task");
    assert!(n.remove("tsk_a", id).unwrap());
    assert!(n.list("tsk_a").unwrap().is_empty());
    let _ = std::fs::remove_dir_all(&d);
}

#[test]
fn notes_are_encrypted_at_rest_and_survive_reopen() {
    let d = dir("enc");
    {
        let s = seeded(&d);
        LocalNotes(&s).add("tsk_a", "officer-1", "SYNTHETICNOTEMARKER", 1).unwrap();
    }
    let raw = std::fs::read(d.join("s.db")).unwrap();
    assert!(!raw.windows(19).any(|w| w == b"SYNTHETICNOTEMARKER"), "plain text found in the file");
    let s = Store::open(&d.join("s.db"), &Fixed([7u8; 32])).unwrap();
    assert_eq!(LocalNotes(&s).list("tsk_a").unwrap().len(), 1);
    assert!(Store::open(&d.join("s.db"), &Fixed([8u8; 32])).is_err(), "wrong key cannot read");
    let _ = std::fs::remove_dir_all(&d);
}

#[test]
fn opening_twice_keeps_notes_and_does_not_touch_the_schema_version() {
    let d = dir("idem");
    let s = seeded(&d);
    LocalNotes(&s).add("tsk_a", "o", "n", 1).unwrap();
    drop(s);
    let s = Store::open(&d.join("s.db"), &Fixed([7u8; 32])).unwrap();
    assert_eq!(LocalNotes(&s).list("tsk_a").unwrap().len(), 1);
    let _ = std::fs::remove_dir_all(&d);
}
