//! The task catalog the canvas routes from (#32): newest version of each saved task, ask-inputs only.
//! Synthetic data only. Keys are injected, so no test touches the login keychain.

use companion_shell::router::catalog;
use companion_shell::store::{Fixed, Store};
use serde_json::json;

#[test]
fn catalog_lists_the_newest_version_of_each_task_with_its_ask_inputs() {
    let d = std::env::temp_dir().join(format!("companion-router-{}", std::process::id()));
    std::fs::create_dir_all(&d).unwrap();
    let mut s = Store::open(&d.join("s.db"), &Fixed([5u8; 32])).unwrap();
    let body = |name: &str| json!({ "name": name, "inputs": [{ "name": "full_name", "type": "text", "source": "ask" }, { "name": "row", "type": "text", "source": "column" }] }).to_string();
    s.save_task("tsk_a", "Add test account", &body("Add test account"), 10).unwrap();
    s.save_task("tsk_a", "Add test account", &body("Add test account"), 20).unwrap();
    s.save_task("tsk_b", "Close test case", "not json", 30).unwrap();
    let c = catalog(&s).unwrap();
    assert_eq!(c.len(), 1, "an unreadable body is skipped, not offered");
    assert_eq!(c[0]["id"], "tsk_a");
    assert_eq!(c[0]["version"], 2);
    assert_eq!(c[0]["inputs"], json!([{ "name": "full_name", "type": "text" }]));
    let _ = std::fs::remove_dir_all(&d);
}
