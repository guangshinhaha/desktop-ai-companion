//! Usage log tests (#35). Synthetic data only, no network, no keychain (the log is not keyed).

use companion_shell::usage::{Event, UsageLog};
use std::path::PathBuf;
use std::process::Command;

const DAY: i64 = 86_400;
// 2026-11-23 is a Monday (epoch day 20780).
const MON: i64 = 20_780 * DAY;

fn tmp(name: &str) -> PathBuf {
    let n = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos();
    let d = std::env::temp_dir().join(format!("companion-usage-{name}-{}-{n}", std::process::id()));
    std::fs::create_dir_all(&d).unwrap();
    d
}

#[test]
fn counts_each_event_per_day() {
    let log = UsageLog::open(&tmp("counts").join("usage.db")).unwrap();
    log.record(Event::ReplayStarted, 1, MON).unwrap();
    log.record(Event::ReplayStarted, 1, MON + 100).unwrap();
    log.record(Event::ReplayFinished, 1, MON + 200).unwrap();
    log.record(Event::ListRun, 1, MON).unwrap();
    log.record(Event::RowDone, 40, MON).unwrap();
    log.record(Event::ChatRequest, 3, MON + DAY).unwrap();
    let v: serde_json::Value = serde_json::from_str(&log.export_json().unwrap()).unwrap();
    let days = v["days"].as_array().unwrap();
    assert_eq!(days.len(), 2);
    assert_eq!(days[0]["date"], "2026-11-23");
    assert_eq!(days[0]["counts"]["replay_started"], 2);
    assert_eq!(days[0]["counts"]["replay_finished"], 1);
    assert_eq!(days[0]["counts"]["list_run"], 1);
    assert_eq!(days[0]["counts"]["row_done"], 40);
    assert_eq!(days[1]["date"], "2026-11-24");
    assert_eq!(days[1]["counts"]["chat_request"], 3);
}

#[test]
fn stats_prints_replay_and_chat_returns_on_separate_lines() {
    let log = UsageLog::open(&tmp("stats").join("usage.db")).unwrap();
    // Previous week: replay on 2 days, no chat. This week: replay on 1 day, chat on 3 days.
    log.record(Event::ReplayStarted, 1, MON - 7 * DAY).unwrap();
    log.record(Event::ReplayStarted, 1, MON - 5 * DAY).unwrap();
    log.record(Event::ReplayStarted, 1, MON).unwrap();
    for d in 0..3 {
        log.record(Event::ChatRequest, 5, MON + d * DAY).unwrap();
    }
    let out = log.stats(MON + 2 * DAY, 2).unwrap();
    let replay = out.lines().find(|l| l.starts_with("replay returns")).unwrap();
    let chat = out.lines().find(|l| l.starts_with("chat returns")).unwrap();
    assert!(replay.contains("2026-11-16:2") && replay.contains("2026-11-23:1"), "{replay}");
    assert!(chat.contains("2026-11-16:0") && chat.contains("2026-11-23:3"), "{chat}");
}

#[test]
fn log_holds_no_content() {
    // Synthetic values an officer might type or see. The API takes no string, so they cannot
    // reach the log; prove it on every byte on disk, in the export, and in the schema.
    let secrets = ["Zebra-Quokka-7731", "S1234567A", "Aisha binti Rahman", "Approve payment"];
    let dir = tmp("content");
    let path = dir.join("usage.db");
    {
        let log = UsageLog::open(&path).unwrap();
        for s in secrets {
            // Stand-in for the app handling the value; only counts and event names are logged.
            let _handled = s.len();
            log.record(Event::RowDone, 1, MON).unwrap();
            log.record(Event::ChatRequest, 1, MON).unwrap();
        }
        let export = log.export_json().unwrap();
        for s in secrets {
            assert!(!export.contains(s));
        }
    }
    for entry in std::fs::read_dir(&dir).unwrap() {
        let bytes = std::fs::read(entry.unwrap().path()).unwrap();
        for s in secrets {
            assert!(!bytes.windows(s.len()).any(|w| w == s.as_bytes()), "{s} found on disk");
        }
    }
    let conn = rusqlite::Connection::open(&path).unwrap();
    let names: Vec<String> = conn
        .prepare("SELECT DISTINCT event FROM usage_daily").unwrap()
        .query_map([], |r| r.get(0)).unwrap().map(|r| r.unwrap()).collect();
    for n in names {
        assert!(Event::ALL.iter().any(|e| e.name() == n), "unexpected event {n}");
    }
    let cols: Vec<String> = conn
        .prepare("SELECT name FROM pragma_table_info('usage_daily')").unwrap()
        .query_map([], |r| r.get(0)).unwrap().map(|r| r.unwrap()).collect();
    assert_eq!(cols, ["day", "event", "n"]);
}

#[test]
fn export_is_the_only_way_out() {
    let dir = tmp("export");
    let log = UsageLog::open(&dir.join("usage.db")).unwrap();
    log.record(Event::ListRun, 1, MON).unwrap();
    let dest = dir.join("out.json");
    assert!(!dest.exists());
    log.export_to(&dest).unwrap();
    assert!(std::fs::read_to_string(&dest).unwrap().contains("list_run"));
    // The module links no network code.
    let src = include_str!("../src/usage.rs");
    for bad in ["std::net", "reqwest", "hyper", "TcpStream", "UdpSocket", "Command::new"] {
        assert!(!src.contains(bad), "{bad}");
    }
}

#[test]
fn companion_stats_command_prints_both_lines() {
    let home = tmp("cli");
    let run = |args: &[&str]| {
        Command::new(env!("CARGO_BIN_EXE_companion-shell")).args(args).env("HOME", &home).output().unwrap()
    };
    let path = home.join("Library/Application Support/sg.gov.grl.companion/usage.db");
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs() as i64;
    UsageLog::open(&path).unwrap().record(Event::ReplayStarted, 1, now).unwrap();
    let out = run(&["stats"]);
    assert!(out.status.success());
    let text = String::from_utf8(out.stdout).unwrap();
    assert!(text.lines().any(|l| l.starts_with("replay returns")), "{text}");
    assert!(text.lines().any(|l| l.starts_with("chat returns")), "{text}");
    let dest = home.join("export.json");
    assert!(run(&["stats", "--export", dest.to_str().unwrap()]).status.success());
    assert!(dest.exists());
}
