//! Task versions and re-verification (#42). Synthetic data only. Keys are injected, so no test touches
//! the login keychain.

use companion_shell::router::catalog;
use companion_shell::store::{Fixed, Store};
use companion_shell::verification::{
    record_or_note, record_outcome, route_rerecord, status, FlagChange, FlagSync, LocalOnly, NoAgreement, Standing, WorkingAgreement,
};
use serde_json::{json, Value};
use std::cell::RefCell;

fn open(tag: &str) -> (Store, std::path::PathBuf) {
    let d = std::env::temp_dir().join(format!("companion-verif-{tag}-{}", std::process::id()));
    std::fs::create_dir_all(&d).unwrap();
    (Store::open(&d.join("s.db"), &Fixed([6u8; 32])).unwrap(), d)
}

fn body(author: &str) -> String {
    json!({ "name": "Add test account", "author": author, "inputs": [{ "name": "full_name", "type": "text", "source": "ask" }] }).to_string()
}

fn finished() -> Value {
    json!({ "outcome": "finished", "completed": ["s1", "s2", "s3"], "not_run": [] })
}
/// A divergence that means the interface changed (the executor sets `interface_changed`).
fn diverged(step: &str, completed: &[&str]) -> Value {
    json!({ "outcome": "diverged", "step": step, "reason": "element not found", "interface_changed": true, "completed": completed, "not_run": [] })
}
fn diverged_other(step: &str, reason: &str) -> Value {
    json!({ "outcome": "diverged", "step": step, "reason": reason, "interface_changed": false, "completed": [], "not_run": [] })
}

#[test]
fn a_divergence_on_a_step_that_used_to_pass_flags_the_version() {
    let (mut s, d) = open("flag");
    s.save_task("tsk_a", "n", &body("officer-1"), 1).unwrap();
    assert_eq!(record_outcome(&mut s, &LocalOnly, "tsk_a", 1, &finished(), 10).unwrap(), Some("verified"));
    assert_ne!(status(&s, "tsk_a").unwrap().unwrap().standing, Standing::NeedsVerification);

    assert_eq!(record_outcome(&mut s, &LocalOnly, "tsk_a", 1, &diverged("s2", &["s1"]), 20).unwrap(), Some("needs_verification"));
    let st = status(&s, "tsk_a").unwrap().unwrap();
    assert_eq!(st.standing, Standing::NeedsVerification);
    assert_eq!(st.flagged_step.as_deref(), Some("s2"));
    assert_eq!(st.default_version, 1, "the flagged version stays replayable");
    let _ = std::fs::remove_dir_all(&d);
}

#[test]
fn a_divergence_on_a_step_that_never_passed_does_not_flag() {
    let (mut s, d) = open("never");
    s.save_task("tsk_a", "n", &body("officer-1"), 1).unwrap();
    assert_eq!(record_outcome(&mut s, &LocalOnly, "tsk_a", 1, &diverged("s1", &[]), 10).unwrap(), None);
    // s1 passes once, then a later run diverges at s2, which never passed: still no flag.
    record_outcome(&mut s, &LocalOnly, "tsk_a", 1, &diverged("s2", &["s1"]), 11).unwrap();
    assert_ne!(status(&s, "tsk_a").unwrap().unwrap().standing, Standing::NeedsVerification);
    // Now s1 used to pass and fails.
    record_outcome(&mut s, &LocalOnly, "tsk_a", 1, &diverged("s1", &[]), 12).unwrap();
    assert_eq!(status(&s, "tsk_a").unwrap().unwrap().standing, Standing::NeedsVerification);
    let _ = std::fs::remove_dir_all(&d);
}

#[test]
fn stopped_and_refused_runs_never_flag() {
    let (mut s, d) = open("stop");
    s.save_task("tsk_a", "n", &body("o"), 1).unwrap();
    record_outcome(&mut s, &LocalOnly, "tsk_a", 1, &finished(), 10).unwrap();
    for o in ["stopped", "not_approved", "refused", "ended"] {
        let r = json!({ "outcome": o, "step": "s2", "completed": ["s1"] });
        assert_eq!(record_outcome(&mut s, &LocalOnly, "tsk_a", 1, &r, 20).unwrap(), None);
    }
    assert_ne!(status(&s, "tsk_a").unwrap().unwrap().standing, Standing::NeedsVerification);
    let _ = std::fs::remove_dir_all(&d);
}

#[test]
fn the_old_version_stays_the_default_until_the_new_one_passes() {
    let (mut s, d) = open("old");
    s.save_task("tsk_a", "n", &body("officer-1"), 1).unwrap();
    record_outcome(&mut s, &LocalOnly, "tsk_a", 1, &finished(), 10).unwrap();
    record_outcome(&mut s, &LocalOnly, "tsk_a", 1, &diverged("s2", &["s1"]), 20).unwrap();

    // Re-recorded as version 2: not yet verified, so version 1 is still what is offered.
    s.save_task("tsk_a", "n", &body("officer-1"), 30).unwrap();
    let st = status(&s, "tsk_a").unwrap().unwrap();
    assert_eq!((st.default_version, st.standing, st.candidate_version), (1, Standing::NeedsVerification, Some(2)));
    let c = catalog(&s).unwrap();
    assert_eq!(c[0]["version"], 1);
    assert_eq!(c[0]["status"], "needs_verification");
    assert_eq!(c[0]["candidate"]["version"], 2);
    assert_eq!(c[0]["candidate"]["inputs"][0]["name"], "full_name", "the candidate carries its own inputs");
    assert_eq!(c[0]["flagged_step"], "s2");
    assert!(!c[0].to_string().contains("officer-1"), "the author's name is not in the catalog");

    // A candidate that diverges does not take over.
    record_outcome(&mut s, &LocalOnly, "tsk_a", 2, &diverged("s1", &[]), 40).unwrap();
    assert_eq!(status(&s, "tsk_a").unwrap().unwrap().default_version, 1);

    // The new version passes: it becomes the default, the flag clears, version 1 is still stored.
    record_outcome(&mut s, &LocalOnly, "tsk_a", 2, &finished(), 50).unwrap();
    let st = status(&s, "tsk_a").unwrap().unwrap();
    assert_eq!((st.default_version, st.standing, st.candidate_version), (2, Standing::Verified, None));
    assert_eq!(catalog(&s).unwrap()[0]["version"], 2);
    assert_eq!(catalog(&s).unwrap()[0]["status"], "ok");
    assert!(s.task_body("tsk_a", 1).unwrap().is_some());
    let _ = std::fs::remove_dir_all(&d);
}

#[test]
fn a_flagged_version_that_passes_again_is_cleared() {
    let (mut s, d) = open("clear");
    s.save_task("tsk_a", "n", &body("o"), 1).unwrap();
    record_outcome(&mut s, &LocalOnly, "tsk_a", 1, &finished(), 10).unwrap();
    record_outcome(&mut s, &LocalOnly, "tsk_a", 1, &diverged("s1", &[]), 20).unwrap();
    assert_eq!(status(&s, "tsk_a").unwrap().unwrap().standing, Standing::NeedsVerification);
    record_outcome(&mut s, &LocalOnly, "tsk_a", 1, &finished(), 30).unwrap();
    assert_ne!(status(&s, "tsk_a").unwrap().unwrap().standing, Standing::NeedsVerification);
    let _ = std::fs::remove_dir_all(&d);
}

#[test]
fn the_flag_survives_reopening_the_store() {
    let d = std::env::temp_dir().join(format!("companion-verif-reopen-{}", std::process::id()));
    std::fs::create_dir_all(&d).unwrap();
    {
        let mut s = Store::open(&d.join("s.db"), &Fixed([6u8; 32])).unwrap();
        s.save_task("tsk_a", "n", &body("o"), 1).unwrap();
        record_outcome(&mut s, &LocalOnly, "tsk_a", 1, &finished(), 10).unwrap();
        record_outcome(&mut s, &LocalOnly, "tsk_a", 1, &diverged("s3", &["s1", "s2"]), 20).unwrap();
    }
    let s = Store::open(&d.join("s.db"), &Fixed([6u8; 32])).unwrap();
    assert_eq!(status(&s, "tsk_a").unwrap().unwrap().standing, Standing::NeedsVerification);
    let _ = std::fs::remove_dir_all(&d);
}

#[test]
fn the_sync_seam_hears_each_change_and_a_failure_keeps_the_local_flag() {
    struct Spy(RefCell<Vec<(String, i64, &'static str)>>);
    impl FlagSync for Spy {
        fn publish(&self, c: &FlagChange) -> Result<(), String> {
            self.0.borrow_mut().push((c.task_id.to_string(), c.version, c.state));
            Err("service unreachable".into())
        }
    }
    let (mut s, d) = open("sync");
    let spy = Spy(RefCell::new(vec![]));
    s.save_task("tsk_a", "n", &body("o"), 1).unwrap();
    record_outcome(&mut s, &spy, "tsk_a", 1, &finished(), 10).unwrap();
    record_outcome(&mut s, &spy, "tsk_a", 1, &diverged("s2", &["s1"]), 20).unwrap();
    assert_eq!(*spy.0.borrow(), vec![("tsk_a".into(), 1, "verified"), ("tsk_a".into(), 1, "needs_verification")]);
    assert_eq!(status(&s, "tsk_a").unwrap().unwrap().standing, Standing::NeedsVerification);
    let _ = std::fs::remove_dir_all(&d);
}

#[test]
fn routing_names_the_owner_and_only_the_teammate_the_agreement_names() {
    struct Agreed;
    impl WorkingAgreement for Agreed {
        fn teammate_for(&self, _: &str) -> Option<String> {
            Some("officer-2".into())
        }
    }
    let r = route_rerecord("tsk_a", &body("officer-1"), &NoAgreement).unwrap();
    assert_eq!((r.owner.as_str(), r.teammate), ("officer-1", None));
    let r = route_rerecord("tsk_a", &body("officer-1"), &Agreed).unwrap();
    assert_eq!(r.teammate.as_deref(), Some("officer-2"));
    assert!(route_rerecord("tsk_a", "not json", &NoAgreement).is_none());
    assert!(route_rerecord("tsk_a", &json!({ "author": "  " }).to_string(), &NoAgreement).is_none());
}

#[test]
fn a_run_of_the_old_version_never_promotes_an_unpassed_candidate() {
    let (mut s, d) = open("promote");
    s.save_task("tsk_a", "n", &body("o"), 1).unwrap();
    record_outcome(&mut s, &LocalOnly, "tsk_a", 1, &finished(), 10).unwrap();
    record_outcome(&mut s, &LocalOnly, "tsk_a", 1, &diverged("s2", &["s1"]), 20).unwrap();
    s.save_task("tsk_a", "n", &body("o"), 30).unwrap();
    // Version 1 passes again: the flag clears, but version 2 has had no run, so it is still only a candidate.
    record_outcome(&mut s, &LocalOnly, "tsk_a", 1, &finished(), 40).unwrap();
    let st = status(&s, "tsk_a").unwrap().unwrap();
    assert_eq!((st.default_version, st.standing, st.candidate_version), (1, Standing::Verified, Some(2)));
    let c = catalog(&s).unwrap();
    assert_eq!((c[0]["version"].as_i64(), c[0]["status"].as_str()), (Some(1), Some("ok")));
    assert_eq!(c[0]["candidate"]["version"], 2);
    // Only its own finished run promotes it.
    record_outcome(&mut s, &LocalOnly, "tsk_a", 2, &finished(), 50).unwrap();
    assert_eq!(status(&s, "tsk_a").unwrap().unwrap().default_version, 2);
    let _ = std::fs::remove_dir_all(&d);
}

#[test]
fn a_new_version_with_no_history_is_unverified_not_ok() {
    let (mut s, d) = open("unver");
    s.save_task("tsk_a", "n", &body("o"), 1).unwrap();
    assert_eq!(status(&s, "tsk_a").unwrap().unwrap().standing, Standing::Unverified);
    assert_eq!(catalog(&s).unwrap()[0]["status"], "unverified");
    let _ = std::fs::remove_dir_all(&d);
}

#[test]
fn only_a_divergence_the_executor_calls_an_interface_change_flags() {
    let (mut s, d) = open("cause");
    s.save_task("tsk_a", "n", &body("o"), 1).unwrap();
    record_outcome(&mut s, &LocalOnly, "tsk_a", 1, &finished(), 10).unwrap();
    for why in [
        "the audit trail could not be written; nothing was done for this step",
        "the window is gone after approval; nothing was done",
        "a dialog appeared after approval; nothing was clicked",
        "the action failed (AX error).",
    ] {
        assert_eq!(record_outcome(&mut s, &LocalOnly, "tsk_a", 1, &diverged_other("s2", why), 20).unwrap(), None, "{why}");
    }
    // A report with no cause at all (an older executor) never flags either.
    let no_cause = json!({ "outcome": "diverged", "step": "s2", "completed": ["s1"] });
    assert_eq!(record_outcome(&mut s, &LocalOnly, "tsk_a", 1, &no_cause, 21).unwrap(), None);
    assert_eq!(status(&s, "tsk_a").unwrap().unwrap().standing, Standing::Verified);
    assert_eq!(record_outcome(&mut s, &LocalOnly, "tsk_a", 1, &diverged("s2", &["s1"]), 22).unwrap(), Some("needs_verification"));
    let _ = std::fs::remove_dir_all(&d);
}

#[test]
fn the_latest_run_wins_not_the_last_writer() {
    let (mut s, d) = open("order");
    s.save_task("tsk_a", "n", &body("o"), 1).unwrap();
    record_outcome(&mut s, &LocalOnly, "tsk_a", 1, &finished(), 10).unwrap();
    record_outcome(&mut s, &LocalOnly, "tsk_a", 1, &diverged("s2", &["s1"]), 30).unwrap();
    // A run that started earlier (at 20) and was written later must not clear the newer flag.
    assert_eq!(record_outcome(&mut s, &LocalOnly, "tsk_a", 1, &finished(), 20).unwrap(), None);
    assert_eq!(status(&s, "tsk_a").unwrap().unwrap().standing, Standing::NeedsVerification);
    let _ = std::fs::remove_dir_all(&d);
}

#[test]
fn a_missing_task_version_is_an_error_and_free_text_is_never_stored() {
    let (mut s, d) = open("valid");
    assert!(record_outcome(&mut s, &LocalOnly, "tsk_none", 1, &finished(), 10).is_err());
    assert!(record_or_note(&mut s, "tsk_none", 1, &finished(), 10).is_some(), "a failed write is reported, not dropped");
    s.save_task("tsk_a", "n", &body("o"), 1).unwrap();
    assert!(record_outcome(&mut s, &LocalOnly, "tsk_a", 2, &finished(), 10).is_err(), "version 2 was never saved");
    // A step id that is not `s` and numbers is dropped: it cannot become a pass or a flagged step.
    let odd = "Alice Tan at the desk";
    record_outcome(&mut s, &LocalOnly, "tsk_a", 1, &json!({ "outcome": "stopped", "completed": [odd, "s1"] }), 10).unwrap();
    assert_eq!(record_outcome(&mut s, &LocalOnly, "tsk_a", 1, &diverged(odd, &[]), 11).unwrap(), None);
    record_outcome(&mut s, &LocalOnly, "tsk_a", 1, &finished(), 12).unwrap();
    assert_eq!(record_outcome(&mut s, &LocalOnly, "tsk_a", 1, &diverged("s1", &[]), 13).unwrap(), Some("needs_verification"));
    assert_eq!(status(&s, "tsk_a").unwrap().unwrap().flagged_step.as_deref(), Some("s1"));
    let _ = std::fs::remove_dir_all(&d);
}
