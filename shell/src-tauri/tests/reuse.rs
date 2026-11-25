//! Reuse counts on a task (#43). Synthetic data only. Keys are injected, so no test touches the keychain.
//! The count unit is this device now, a team later (#41). Never an officer.

use companion_shell::reuse::{LocalReuse, Reuse, ReuseBackend};
use companion_shell::router::{catalog, catalog_with};
use companion_shell::store::{Fixed, Store};
use serde_json::json;

fn open(tag: &str) -> Store {
    let d = std::env::temp_dir().join(format!("companion-reuse-{tag}-{}", std::process::id()));
    std::fs::create_dir_all(&d).unwrap();
    let _ = std::fs::remove_file(d.join("s.db"));
    let mut s = Store::open(&d.join("s.db"), &Fixed([8u8; 32])).unwrap();
    let body = json!({ "name": "Add test account", "author": "officer-1" }).to_string();
    s.save_task("tsk_a", "Add test account", &body, 1).unwrap();
    s.save_task("tsk_b", "Other task", &body, 1).unwrap();
    s
}

fn run(s: &Store, id: &str, task: &str, status: &str, at: i64) {
    s.start_run(id, task, 1, "shell", at).unwrap();
    s.finish_run(id, status).unwrap();
}

#[test]
fn a_task_never_run_shows_zero_and_no_date() {
    let s = open("zero");
    assert_eq!(LocalReuse(&s).reuse("tsk_a").unwrap(), Reuse { replays: 0, last_success_at: None, scope: "this_device" });
}

#[test]
fn only_finished_runs_count_and_the_date_is_the_last_success() {
    let s = open("count");
    run(&s, "r1", "tsk_a", "finished", 100);
    run(&s, "r2", "tsk_a", "finished", 300);
    run(&s, "r3", "tsk_a", "stopped", 400);
    run(&s, "r4", "tsk_a", "refused", 500);
    run(&s, "r5", "tsk_a", "ended", 600);
    s.start_run("r6", "tsk_a", 1, "shell", 700).unwrap(); // still running
    run(&s, "r7", "tsk_b", "finished", 800); // another task
    let r = LocalReuse(&s).reuse("tsk_a").unwrap();
    assert_eq!((r.replays, r.last_success_at), (2, Some(300)));
}

#[test]
fn a_finished_list_run_with_a_left_out_or_rejected_row_counts_but_did_not_succeed() {
    let mut s = open("div");
    run(&s, "r1", "tsk_a", "finished", 100);
    for (id, at, state) in [("r2", 200, "skipped"), ("r3", 300, "rejected"), ("r4", 400, "diverged")] {
        s.start_run(id, "tsk_a", 1, "{}", at).unwrap();
        s.create_run_items(id, 2).unwrap();
        s.set_run_item(id, 0, "done", None).unwrap();
        s.set_run_item(id, 1, state, Some("s2")).unwrap();
        s.finish_run(id, "finished").unwrap();
    }
    let r = LocalReuse(&s).reuse("tsk_a").unwrap();
    assert_eq!((r.replays, r.last_success_at), (4, Some(100)));
}

#[test]
fn the_catalog_carries_the_numbers_and_nothing_per_officer() {
    let s = open("cat");
    run(&s, "r1", "tsk_a", "finished", 100);
    let c = catalog(&s).unwrap();
    let a = c.iter().find(|t| t["id"] == "tsk_a").unwrap();
    assert_eq!(a["reuse"], json!({ "replays": 1, "last_success_at": 100, "scope": "this_device" }));
    assert!(!a.to_string().contains("officer"), "no officer in the catalog");
}

struct FakeTeam;
impl ReuseBackend for FakeTeam {
    fn reuse(&self, _: &str) -> Result<Reuse, String> {
        Ok(Reuse { replays: 9, last_success_at: Some(5), scope: "team" })
    }
}

#[test]
fn a_shared_backend_slots_in_without_changing_the_catalog() {
    let s = open("seam");
    let c = catalog_with(&s, &FakeTeam).unwrap();
    assert_eq!(c[0]["reuse"]["scope"], "team");
    assert_eq!(c[0]["reuse"]["replays"], 9);
}

#[test]
fn the_shape_has_no_officer_field() {
    let r = Reuse { replays: 1, last_success_at: None, scope: "this_device" };
    let dbg = format!("{r:?}");
    assert!(!dbg.contains("officer") && !dbg.contains("by_"), "{dbg}");
}
