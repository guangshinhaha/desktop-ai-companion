//! Notes and decisions on a task (PRD M-1; #44). Context that travels with the task: why it is
//! done this way, exceptions, decisions made.
//!
//! Notes may hold personal data, so they are treated like other stored content: encrypted at rest
//! in the local store, never logged, never sent anywhere. Errors here name a reason, never the text.
//!
//! The seam: everything the shell does with notes goes through [`NoteBackend`]. Today the only
//! backend is [`LocalNotes`] (this device). Per-team visibility and sync with the task (M-3, #41)
//! need the shared library (#29); that arrives as a second backend, with no change to callers.

use crate::store::Store;
use serde_json::{json, Value};

pub const MAX_NOTE_CHARS: usize = 2000;

pub trait NoteBackend {
    fn add(&self, task_id: &str, author: &str, body: &str, now: i64) -> Result<i64, String>;
    /// Oldest first.
    fn list(&self, task_id: &str) -> Result<Vec<Note>, String>;
    fn remove(&self, task_id: &str, note_id: i64) -> Result<bool, String>;
}

pub struct Note {
    pub id: i64,
    pub author: String,
    pub at: i64,
    pub body: String,
}

/// Notes held in the encrypted local store, visible on this device only.
pub struct LocalNotes<'a>(pub &'a Store);

impl NoteBackend for LocalNotes<'_> {
    fn add(&self, task_id: &str, author: &str, body: &str, now: i64) -> Result<i64, String> {
        let body = body.trim();
        if body.is_empty() {
            return Err("A note cannot be empty.".into());
        }
        if body.chars().count() > MAX_NOTE_CHARS {
            return Err(format!("A note can be at most {MAX_NOTE_CHARS} characters."));
        }
        let author = author.trim();
        if author.is_empty() {
            return Err("Say who is adding the note.".into());
        }
        self.0.add_note(task_id, author, body, now).map_err(|e| e.to_string())
    }

    fn list(&self, task_id: &str) -> Result<Vec<Note>, String> {
        let rows = self.0.notes_for(task_id).map_err(|e| e.to_string())?;
        Ok(rows.into_iter().map(|(id, author, at, body)| Note { id, author, at, body }).collect())
    }

    fn remove(&self, task_id: &str, note_id: i64) -> Result<bool, String> {
        self.0.delete_note(task_id, note_id).map_err(|e| e.to_string())
    }
}

/// What the task screen shows: the newest version's plain-language steps, and the notes beside them.
pub fn detail(store: &Store, notes: &dyn NoteBackend, task_id: &str) -> Result<Value, String> {
    let (version, body) = store
        .latest_tasks()
        .map_err(|e| e.to_string())?
        .into_iter()
        .find(|(id, _, _)| id == task_id)
        .map(|(_, v, b)| (v, b))
        .ok_or("No such task.")?;
    let task: Value = serde_json::from_str(&body).map_err(|_| "This task could not be read.".to_string())?;
    let steps: Vec<Value> = task["steps"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|s| json!({ "id": s["id"], "say": s["say"] }))
        .collect();
    let notes: Vec<Value> = notes
        .list(task_id)?
        .into_iter()
        .map(|n| json!({ "id": n.id, "author": n.author, "at": n.at, "body": n.body }))
        .collect();
    Ok(json!({
        "id": task_id, "version": version, "name": task["name"],
        "steps": steps, "notes": notes, "max_chars": MAX_NOTE_CHARS,
    }))
}
