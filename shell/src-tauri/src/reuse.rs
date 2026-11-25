//! How much a task has been reused (PRD L-4; #43). A count of replays and the last success, never who ran
//! them: the unit is a team (or, until the shared library exists, this device), so there is no per-officer
//! figure here to read, store, rank or log. Counts and a time only; no task text, value or name.
//!
//! The seam: callers go through [`ReuseBackend`]. The only backend today is [`LocalReuse`], computed from the
//! local runs table, so the numbers are the "this device" slice. The per-team aggregate from the shared
//! library (#41, #29) arrives as a second backend, with no change to callers.

use crate::store::Store;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Reuse {
    /// Finished replays of the task (any version). Stopped, refused, ended and still-running runs are not counted.
    pub replays: i64,
    /// When the latest run that succeeded began (unix seconds). A run succeeded when it finished and no row
    /// diverged, was rejected or was left out.
    pub last_success_at: Option<i64>,
    /// What the numbers cover: `this_device` now, `team` once the shared library backs it.
    pub scope: &'static str,
}

pub trait ReuseBackend {
    fn reuse(&self, task_id: &str) -> Result<Reuse, String>;
}

/// Reuse from this device's runs table.
pub struct LocalReuse<'a>(pub &'a Store);

impl ReuseBackend for LocalReuse<'_> {
    fn reuse(&self, task_id: &str) -> Result<Reuse, String> {
        let (replays, last_success_at) = self.0.reuse_counts(task_id).map_err(|e| e.to_string())?;
        Ok(Reuse { replays, last_success_at, scope: "this_device" })
    }
}
