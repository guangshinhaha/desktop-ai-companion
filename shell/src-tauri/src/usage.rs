//! Content-free usage log (#35): the adoption metric. Counts per day for a closed set of event
//! names, nothing else. There is no text column, and what writes the log (`record`, `Tally`) takes an
//! [`Event`] and no string, so an input, chat text, label or screen content cannot be written.
//! `counted_replay` holds a run's task and inputs only to pass them to the replay. The file is local;
//! nothing here touches the network. The only way out is `export_to`, which the officer triggers.
//!
//! What each count means:
//!  - `replay_started`: the executor was launched and sent the task, once per launch, for a single replay or
//!    a list row. A retried row counts again. A run that fails before that counts nothing.
//!  - `replay_finished`: a launch whose report said `finished`. Refused, not approved, diverged, stopped and
//!    an executor that died count as started only.
//!  - `list_run`: a new list run. Picking an unfinished one up is the same list run.
//!  - `row_done`: a row a replay finished. A row the officer marked done or did by hand is not counted.
//!  - `chat_request`: a chat turn that got a reply.
//!
//! Days are UTC.

use crate::approval::{run_replay_observed, Hooks, ItemObserver, RunContext};
use crate::listrun::Loaded;
use crate::progress::RunHandle;
use crate::store::Store;
use rusqlite::{params, Connection};
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

const SECONDS_PER_DAY: i64 = 86_400;

/// The closed set of events. Names are fixed strings, never derived from user content.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Event {
    ReplayStarted,
    ReplayFinished,
    ListRun,
    RowDone,
    ChatRequest,
}

impl Event {
    pub const ALL: [Event; 5] =
        [Event::ReplayStarted, Event::ReplayFinished, Event::ListRun, Event::RowDone, Event::ChatRequest];

    pub fn name(self) -> &'static str {
        match self {
            Event::ReplayStarted => "replay_started",
            Event::ReplayFinished => "replay_finished",
            Event::ListRun => "list_run",
            Event::RowDone => "row_done",
            Event::ChatRequest => "chat_request",
        }
    }
}

/// `~/Library/Application Support/sg.gov.grl.companion/usage.db`
pub fn default_path() -> Option<PathBuf> {
    crate::store::default_path().map(|p| p.with_file_name("usage.db"))
}

/// What the app's flows record into. It takes an [`Event`] and nothing else, and it never fails.
pub struct Tally(Option<PathBuf>);

impl Tally {
    pub fn at(path: PathBuf) -> Tally {
        Tally(Some(path))
    }

    /// The device's log ([`default_path`]).
    pub fn device() -> Tally {
        Tally(default_path())
    }

    /// Records nothing.
    pub const fn off() -> Tally {
        Tally(None)
    }

    /// Adds one to today's count. A log that cannot be written is ignored: counting never affects the app.
    pub fn note(&self, event: Event) {
        let Some(path) = &self.0 else { return };
        let Ok(now) = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH) else { return };
        let _ = UsageLog::open(path).and_then(|log| log.record(event, 1, now.as_secs() as i64));
    }

    /// Counts a list run the officer started. One picked up again was counted when it was new.
    pub fn list_run_loaded(&self, loaded: &Loaded) {
        if !loaded.resumed {
            self.note(Event::ListRun);
        }
    }
}

/// `approval::run_replay_observed`, counted.
#[allow(clippy::too_many_arguments)]
pub fn counted_replay(
    tally: &Tally,
    bin: &Path,
    store: &mut Store,
    ctx: &RunContext,
    task: &Value,
    inputs: &Value,
    pid: Option<i64>,
    decide: &mut dyn FnMut(&Value) -> bool,
    hooks: &mut Hooks,
    obs: &mut dyn ItemObserver,
) -> std::result::Result<Value, String> {
    let on_start = &mut *hooks.on_start;
    // Called only once the executor is up and has the task. The shell gets its handle first.
    let mut started = |handle: RunHandle| {
        on_start(handle);
        tally.note(Event::ReplayStarted);
    };
    let mut counted = Hooks { on_start: &mut started, on_progress: &mut *hooks.on_progress, stop_grace: hooks.stop_grace };
    let result = run_replay_observed(bin, store, ctx, task, inputs, pid, decide, &mut counted, obs);
    if result.as_ref().is_ok_and(|report| report["outcome"] == "finished") {
        tally.note(Event::ReplayFinished);
    }
    result
}

pub struct UsageLog {
    conn: Connection,
}

/// Errors carry a short reason only.
pub type Result<T> = std::result::Result<T, String>;

fn s<E: ToString>(e: E) -> String {
    e.to_string()
}

impl UsageLog {
    pub fn open(path: &Path) -> Result<UsageLog> {
        use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt};
        if let Some(dir) = path.parent().filter(|d| !d.as_os_str().is_empty()) {
            std::fs::DirBuilder::new().recursive(true).mode(0o700).create(dir).map_err(s)?;
        }
        match std::fs::OpenOptions::new().write(true).create_new(true).mode(0o600).open(path) {
            Ok(_) => {}
            Err(x) if x.kind() == std::io::ErrorKind::AlreadyExists => {}
            Err(x) => return Err(s(x)),
        }
        let conn = Connection::open(path).map_err(s)?;
        // A run writes here as it goes, so a locked log is given up on quickly (the default wait is 5 s).
        conn.busy_timeout(std::time::Duration::from_millis(200)).map_err(s)?;
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS usage_daily (
               day INTEGER NOT NULL, event TEXT NOT NULL, n INTEGER NOT NULL,
               PRIMARY KEY (day, event));",
        )
        .map_err(s)?;
        Ok(UsageLog { conn })
    }

    /// Adds `n` to the event's count for the UTC day of `now` (unix seconds).
    pub fn record(&self, event: Event, n: u32, now: i64) -> Result<()> {
        self.conn
            .execute(
                "INSERT INTO usage_daily (day, event, n) VALUES (?1, ?2, ?3)
                 ON CONFLICT(day, event) DO UPDATE SET n = n + excluded.n",
                params![now.div_euclid(SECONDS_PER_DAY), event.name(), n],
            )
            .map(|_| ())
            .map_err(s)
    }

    /// day -> event name -> count.
    fn rows(&self) -> Result<BTreeMap<i64, BTreeMap<String, i64>>> {
        let mut st = self.conn.prepare("SELECT day, event, n FROM usage_daily").map_err(s)?;
        let it = st
            .query_map([], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?, r.get::<_, i64>(2)?)))
            .map_err(s)?;
        let mut out: BTreeMap<i64, BTreeMap<String, i64>> = BTreeMap::new();
        for row in it {
            let (d, e, n) = row.map_err(s)?;
            out.entry(d).or_default().insert(e, n);
        }
        Ok(out)
    }

    /// Per-day counts as JSON, for the officer to export. Only dates, event names, counts.
    pub fn export_json(&self) -> Result<String> {
        let days: Vec<serde_json::Value> = self
            .rows()?
            .into_iter()
            .map(|(d, ev)| serde_json::json!({ "date": civil(d), "counts": ev }))
            .collect();
        serde_json::to_string_pretty(&serde_json::json!({ "format": "usage/v1", "days": days })).map_err(s)
    }

    /// Writes the export to a file the officer chose. The one way data leaves the log.
    pub fn export_to(&self, dest: &Path) -> Result<()> {
        std::fs::write(dest, self.export_json()?).map_err(s)
    }

    /// Weekly returns for the last `weeks` weeks ending at the week of `now`: the number of
    /// distinct days in that week with a replay started, and (separately) with a chat request.
    /// Weeks start on Monday (UTC).
    pub fn stats(&self, now: i64, weeks: u32) -> Result<String> {
        let rows = self.rows()?;
        let week_of = |day: i64| (day + 3).div_euclid(7); // epoch day 0 is a Thursday
        let this = week_of(now.div_euclid(SECONDS_PER_DAY));
        let days_with = |ev: Event, w: i64| -> usize {
            rows.iter()
                .filter(|(d, m)| week_of(**d) == w && m.get(ev.name()).copied().unwrap_or(0) > 0)
                .map(|(d, _)| *d)
                .collect::<BTreeSet<_>>()
                .len()
        };
        let range: Vec<i64> = (0..weeks as i64).rev().map(|i| this - i).collect();
        let line = |label: &str, ev: Event| {
            let cells: Vec<String> =
                range.iter().map(|w| format!("{}:{}", civil(w * 7 - 3), days_with(ev, *w))).collect();
            format!("{label}: {}", cells.join("  "))
        };
        Ok(format!(
            "weekly returns (days active, week starting Monday)\n{}\n{}\n",
            line("replay returns", Event::ReplayStarted),
            line("chat returns  ", Event::ChatRequest)
        ))
    }
}

/// Epoch day number to `YYYY-MM-DD` (proleptic Gregorian).
fn civil(days: i64) -> String {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    format!("{y:04}-{m:02}-{d:02}")
}
