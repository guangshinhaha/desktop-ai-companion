//! Task versions and re-verification (#42, PRD L-3). When a replay diverges because the target's interface
//! changed, on a step that passed before, that task version is marked `needs_verification`. Old versions
//! stay replayable until a newer one passes. This is per device: the shared library does not exist yet
//! (#41), so flagging "everyone who has it" and routing a re-recording to a named person go through the two
//! seams below, with a local stand-in each. Step ids and states only: no value is read, stored or logged
//! here. No network.

use crate::store::Store;
use serde_json::Value;

/// Where a task's default version stands.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Standing {
    /// A run of it has finished and nothing has diverged since.
    Verified,
    /// No run of it has finished yet. Honest, not "ok": a new recording starts here.
    Unverified,
    /// A step that used to pass diverged because the interface changed.
    NeedsVerification,
}

impl Standing {
    pub fn as_str(self) -> &'static str {
        match self {
            Standing::Verified => "ok",
            Standing::Unverified => "unverified",
            Standing::NeedsVerification => "needs_verification",
        }
    }
}

/// What a task offers the officer right now.
#[derive(Debug, PartialEq, Eq)]
pub struct TaskStatus {
    /// The version to replay by default.
    pub default_version: i64,
    pub standing: Standing,
    /// The step the flagged version diverged at (a step id, never free text).
    pub flagged_step: Option<String>,
    /// A newer version that has not passed yet. It becomes the default only when a run of that very version
    /// finishes; a run of an older version never promotes it. Replay it explicitly to verify it.
    pub candidate_version: Option<i64>,
}

/// Works out the default version and its standing from the versions and recorded states.
/// A version is trusted once a run of it has finished (`verified`, or flagged after that). The default is the
/// newest trusted version, so a flagged one stays replayable until a newer one passes; with none trusted it is
/// the newest version, shown as unverified. Anything newer than the default is the candidate. None when the
/// task has no versions.
pub fn status(store: &Store, task_id: &str) -> Result<Option<TaskStatus>, String> {
    let versions = store.task_versions(task_id).map_err(|e| e.to_string())?;
    let states = store.version_states(task_id).map_err(|e| e.to_string())?;
    let row = |v: i64| states.iter().find(|s| s.version == v);
    let trusted = |v: i64| row(v).is_some_and(|s| s.state == "verified" || s.was_verified);
    let Some(&newest) = versions.last() else { return Ok(None) };
    let default = versions.iter().copied().rev().find(|&v| trusted(v)).unwrap_or(newest);
    let (standing, flagged_step) = match row(default) {
        Some(s) if s.state == "verified" => (Standing::Verified, None),
        Some(s) => (Standing::NeedsVerification, s.step.clone()),
        None => (Standing::Unverified, None),
    };
    Ok(Some(TaskStatus { default_version: default, standing, flagged_step, candidate_version: (newest > default).then_some(newest) }))
}

/// A version's state changed. This is what the shared library will need to hear about (#41).
pub struct FlagChange<'a> {
    pub task_id: &'a str,
    pub version: i64,
    pub state: &'static str,
}

/// Seam for telling everyone who has the task. [`LocalOnly`] is the only implementation until the shared
/// library exists; nothing is sent anywhere.
pub trait FlagSync {
    fn publish(&self, change: &FlagChange) -> Result<(), String>;
}

pub struct LocalOnly;
impl FlagSync for LocalOnly {
    fn publish(&self, _: &FlagChange) -> Result<(), String> {
        Ok(())
    }
}

/// Who re-records. PRD 13 question 6 is open: the pilot team's working agreement names the teammate.
/// Until it does, nothing here guesses one.
pub trait WorkingAgreement {
    fn teammate_for(&self, task_id: &str) -> Option<String>;
}

pub struct NoAgreement;
impl WorkingAgreement for NoAgreement {
    fn teammate_for(&self, _: &str) -> Option<String> {
        None
    }
}

#[derive(Debug, PartialEq, Eq)]
pub struct Routing {
    /// The task's author, from the recorded task.
    pub owner: String,
    /// The teammate the working agreement names, if it names one.
    pub teammate: Option<String>,
}

/// Where a re-recording of this task goes. The owner is the author on the saved task. Sending it to them
/// (a notification) is the shared service's job (#41); this only says who. The author's name is personal
/// data: it is not put in the task catalog the canvas reads, and it must never be put in a model prompt.
pub fn route_rerecord(task_id: &str, body: &str, agreement: &dyn WorkingAgreement) -> Option<Routing> {
    let task: Value = serde_json::from_str(body).ok()?;
    let owner = task["author"].as_str().filter(|a| !a.trim().is_empty())?.to_string();
    Some(Routing { owner, teammate: agreement.teammate_for(task_id) })
}

/// Records a replay's report against the task version. Returns the state it set, if any, after telling
/// `sync`. A failing `sync` does not undo the local mark: the flag is a safety signal and stays. Only a
/// divergence the executor reports as `interface_changed` can flag; a report without it never does.
/// The error is a kind, never content.
pub fn record_outcome(
    store: &mut Store,
    sync: &dyn FlagSync,
    task_id: &str,
    version: i64,
    report: &Value,
    now: i64,
) -> Result<Option<&'static str>, String> {
    let completed: Vec<String> =
        report["completed"].as_array().into_iter().flatten().filter_map(|s| s.as_str().map(str::to_string)).collect();
    let set = store
        .record_replay_outcome(
            task_id,
            version,
            report["outcome"].as_str().unwrap_or(""),
            report["step"].as_str(),
            report["interface_changed"].as_bool() == Some(true),
            &completed,
            now,
        )
        .map_err(|e| e.to_string())?;
    if let Some(state) = set {
        let _ = sync.publish(&FlagChange { task_id, version, state });
    }
    Ok(set)
}

/// Records the report, and turns a failure into a short content-free line the caller can show, so a write
/// that failed is never silently dropped. The task is still replayable; only its flag may be stale.
pub fn record_or_note(store: &mut Store, task_id: &str, version: i64, report: &Value, now: i64) -> Option<String> {
    record_outcome(store, &LocalOnly, task_id, version, report, now)
        .err()
        .map(|_| "the result of this run could not be recorded, so the task's verification status may be out of date".to_string())
}
