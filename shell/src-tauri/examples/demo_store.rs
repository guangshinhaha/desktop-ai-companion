//! Writes a small encrypted store with synthetic data so the file can be poked at with plain
//! `sqlite3`. Usage: cargo run --example demo_store -- <path>. Fixed demo key; never a real one.
use companion_shell::store::{AuditEntry, Fixed, Store};

fn main() {
    let path = std::env::args().nth(1).expect("path");
    let mut s = Store::open(std::path::Path::new(&path), &Fixed([7u8; 32])).unwrap();
    s.save_task("tsk_demo", "Demo task", "{}", 1).unwrap();
    let e = AuditEntry {
        at: 1,
        officer: "o1",
        event: "approved",
        task: None,
        run: None,
        item: None,
        step: None,
        shown: Some("synthetic value"),
    };
    s.append_audit(&e).unwrap();
}
