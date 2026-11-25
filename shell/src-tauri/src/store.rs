//! Local store (TD 3.4): one SQLCipher database for tasks, runs, run items, audit and chat.
//! The key lives in the macOS keychain. No network. Nothing here logs or prints a value:
//! errors carry a kind, never row content.

use rusqlite::{params, Connection, OptionalExtension, TransactionBehavior};
use sha2::{Digest, Sha256};
use std::fmt;
use std::path::Path;

/// Default audit retention: 7 years (2555 days), a config value the pilot agency can change.
/// Callers pass the configured value to `purge_expired`; nothing purges on its own.
pub const DEFAULT_AUDIT_RETENTION_DAYS: u32 = 2555;
const SECONDS_PER_DAY: i64 = 86_400;

/// Errors never include row values (PDPA), only a short reason.
#[derive(Debug)]
pub enum StoreError {
    Db(String),
    Key(String),
    Chain(i64),
}

impl fmt::Display for StoreError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            StoreError::Db(m) => write!(f, "store error: {m}"),
            StoreError::Key(m) => write!(f, "key error: {m}"),
            StoreError::Chain(seq) => write!(f, "audit chain broken at seq {seq}"),
        }
    }
}
impl std::error::Error for StoreError {}

impl From<rusqlite::Error> for StoreError {
    fn from(e: rusqlite::Error) -> Self {
        StoreError::Db(e.to_string())
    }
}

pub type Result<T> = std::result::Result<T, StoreError>;

/// A 256-bit database key. No Debug/Display, so it cannot reach a log by accident.
pub struct Key([u8; 32]);

impl Key {
    pub fn from_bytes(b: [u8; 32]) -> Self {
        Key(b)
    }
    fn pragma(&self) -> String {
        let hex: String = self.0.iter().map(|b| format!("{b:02x}")).collect();
        format!("x'{hex}'")
    }
}

/// Where the key comes from. The shell uses [`Keychain`]; tests inject a fixed key.
pub trait KeySource {
    /// The key, and whether this call created it (first use).
    fn key_and_origin(&self) -> Result<(Key, bool)>;
    fn key(&self) -> Result<Key> {
        Ok(self.key_and_origin()?.0)
    }
}

pub struct Fixed(pub [u8; 32]);
impl KeySource for Fixed {
    fn key_and_origin(&self) -> Result<(Key, bool)> {
        Ok((Key(self.0), false))
    }
}

/// The macOS keychain item holding the database key. Created on first use.
pub struct Keychain {
    pub service: String,
    pub account: String,
}

impl Keychain {
    pub fn app_default() -> Self {
        Keychain { service: "sg.gov.grl.companion".into(), account: "local-store-key".into() }
    }

    /// Removes the item. Used by tests to clean up their throwaway item.
    pub fn delete(&self) -> Result<()> {
        security_framework::passwords::delete_generic_password(&self.service, &self.account)
            .map_err(|e| StoreError::Key(e.to_string()))
    }
}

/// errSecDuplicateItem
const DUPLICATE_ITEM: i32 = -25299;

/// Outcome of an add-only keychain write.
pub enum AddOutcome {
    Added,
    /// The item already exists (another first open won the race).
    Duplicate,
    Failed(String),
}

/// The two keychain calls the key logic needs. [`Keychain`] uses the real macOS keychain;
/// tests inject an in-memory backend so CI never touches a keychain.
pub trait KeychainBackend {
    fn get(&self) -> Option<Vec<u8>>;
    /// Add-only: must never overwrite an existing item.
    fn add(&self, data: &[u8]) -> AddOutcome;
}

/// Get, else create with an add-only call. If two first opens race, one add succeeds and the
/// other gets a duplicate outcome and reads the winner's key, so both end with the same key.
pub fn key_via(backend: &dyn KeychainBackend) -> Result<(Key, bool)> {
    let read = |b: &dyn KeychainBackend| {
        b.get().map(|v| {
            <[u8; 32]>::try_from(v)
                .map(Key)
                .map_err(|_| StoreError::Key("stored key has the wrong length".into()))
        })
    };
    if let Some(k) = read(backend) {
        return Ok((k?, false));
    }
    let mut fresh = [0u8; 32];
    std::fs::File::open("/dev/urandom")
        .and_then(|mut f| std::io::Read::read_exact(&mut f, &mut fresh))
        .map_err(|e| StoreError::Key(e.to_string()))?;
    match backend.add(&fresh) {
        AddOutcome::Added => Ok((Key(fresh), true)),
        AddOutcome::Duplicate => match read(backend) {
            Some(k) => Ok((k?, false)),
            None => Err(StoreError::Key("key item vanished during creation".into())),
        },
        AddOutcome::Failed(m) => Err(StoreError::Key(m)),
    }
}

impl KeychainBackend for Keychain {
    fn get(&self) -> Option<Vec<u8>> {
        security_framework::passwords::get_generic_password(&self.service, &self.account).ok()
    }
    fn add(&self, data: &[u8]) -> AddOutcome {
        use core_foundation::data::CFData;
        use security_framework::item::{ItemAddOptions, ItemAddValue, ItemClass};
        let added = ItemAddOptions::new(ItemAddValue::Data {
            class: ItemClass::generic_password(),
            data: CFData::from_buffer(data),
        })
        .set_service(&self.service)
        .set_account_name(&self.account)
        .add();
        match added {
            Ok(()) => AddOutcome::Added,
            Err(e) if e.code() == DUPLICATE_ITEM => AddOutcome::Duplicate,
            Err(e) => AddOutcome::Failed(e.to_string()),
        }
    }
}

impl KeySource for Keychain {
    fn key_and_origin(&self) -> Result<(Key, bool)> {
        key_via(self)
    }
}

/// `~/Library/Application Support/sg.gov.grl.companion/store.db`
pub fn default_path() -> Option<std::path::PathBuf> {
    let home = std::env::var_os("HOME")?;
    Some(
        std::path::PathBuf::from(home)
            .join("Library/Application Support/sg.gov.grl.companion/store.db"),
    )
}

/// Creates the parent directory as 0700 and the database file as 0600 before SQLite touches it.
/// SQLite gives its journal and WAL files the database file's mode.
fn private_dir_and_file(path: &Path) -> Result<()> {
    use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt};
    let io = |e: std::io::Error| StoreError::Db(e.to_string());
    if let Some(dir) = path.parent().filter(|d| !d.as_os_str().is_empty()) {
        std::fs::DirBuilder::new().recursive(true).mode(0o700).create(dir).map_err(io)?;
    }
    match std::fs::OpenOptions::new().write(true).create_new(true).mode(0o600).open(path) {
        Ok(_) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => Ok(()),
        Err(e) => Err(io(e)),
    }
}

/// An audit record to append. `shown` holds the approved values (decision 0005); the file is
/// encrypted, and this is the only table that keeps them.
pub struct AuditEntry<'a> {
    pub at: i64,
    pub officer: &'a str,
    pub event: &'a str,
    pub task: Option<&'a str>,
    pub run: Option<&'a str>,
    pub item: Option<i64>,
    pub step: Option<&'a str>,
    pub shown: Option<&'a str>,
}

/// The events of TD 7.2, plus the one the store writes for itself. `append_event` accepts nothing else.
pub const AUDIT_EVENTS: [&str; 12] = [
    "run_started",
    "approved",
    "rejected",
    "step_diverged",
    "run_stopped",
    "recovered",
    "run_finished",
    "item_marked_by_officer",
    "task_saved",
    "task_shared",
    "task_handed_over",
    "audit_purged",
];

/// A step id as the task schema writes it: `s` and digits (`schema/task.v1.json`). Nothing else is stored.
pub fn valid_step_id(s: &str) -> bool {
    s.len() >= 2 && s.len() <= 12 && s.starts_with('s') && s[1..].bytes().all(|b| b.is_ascii_digit())
}

/// One task version's verification state (#42).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VersionState {
    pub version: i64,
    /// `verified` or `needs_verification`.
    pub state: String,
    pub step: Option<String>,
    /// A run of this version has finished at some point.
    pub was_verified: bool,
}

pub struct Store {
    conn: Connection,
    path: std::path::PathBuf,
}

/// The right to run one list run, held for as long as this value lives. It is an advisory file lock beside
/// the database, so it is released when the holder exits or crashes, and two stores on one database (two
/// processes, or two threads) cannot both hold it. Without it a run's rows are not touched.
#[derive(Debug)]
pub struct RunLease(#[allow(dead_code)] std::fs::File);

const SCHEMA_VERSION: i64 = 1;

impl Store {
    pub fn open(path: &Path, keys: &dyn KeySource) -> Result<Store> {
        // Key first: if it fails, nothing is created on disk.
        let (key, key_is_new) = keys.key_and_origin()?;
        // An empty store file next to a key that already existed is a damaged store, not a new
        // one; opening it as fresh would silently start a second history.
        if !key_is_new && std::fs::metadata(path).is_ok_and(|m| m.len() == 0) {
            return Err(StoreError::Db("store file is empty but a key exists (delete the empty file to start fresh)".into()));
        }
        private_dir_and_file(path)?;
        let conn = Connection::open(path)?;
        conn.pragma_update(None, "key", key.pragma())?;
        // Forces SQLCipher to decrypt page 1: a wrong key fails here, not later.
        conn.query_row("SELECT count(*) FROM sqlite_master", [], |r| r.get::<_, i64>(0))?;
        conn.pragma_update(None, "recursive_triggers", true)?;
        let store = Store { conn, path: path.to_path_buf() };
        store.migrate()?;
        store.ensure_notes()?;
        store.migrate_verification()?;
        Ok(store)
    }

    /// Versioned and idempotent: re-running changes nothing.
    fn migrate(&self) -> Result<()> {
        let v: i64 = self.conn.query_row("PRAGMA user_version", [], |r| r.get(0))?;
        if v >= SCHEMA_VERSION {
            return Ok(());
        }
        self.conn.execute_batch(
            "BEGIN;
             CREATE TABLE IF NOT EXISTS tasks (
               id TEXT NOT NULL, version INTEGER NOT NULL, name TEXT NOT NULL,
               body TEXT NOT NULL, saved_at INTEGER NOT NULL,
               PRIMARY KEY (id, version));
             CREATE TABLE IF NOT EXISTS runs (
               id TEXT PRIMARY KEY, task_id TEXT NOT NULL, task_version INTEGER NOT NULL,
               inputs_source TEXT NOT NULL, status TEXT NOT NULL, started_at INTEGER NOT NULL,
               FOREIGN KEY (task_id, task_version) REFERENCES tasks(id, version));
             CREATE TABLE IF NOT EXISTS run_items (
               run_id TEXT NOT NULL REFERENCES runs(id), item INTEGER NOT NULL,
               state TEXT NOT NULL, last_step TEXT,
               PRIMARY KEY (run_id, item));
             CREATE TABLE IF NOT EXISTS audit (
               seq INTEGER PRIMARY KEY, at INTEGER NOT NULL,
               officer TEXT NOT NULL, event TEXT NOT NULL, task TEXT, run TEXT,
               item INTEGER, step TEXT, shown TEXT,
               prev_hash TEXT NOT NULL, hash TEXT NOT NULL);
             CREATE TABLE IF NOT EXISTS chat (
               id INTEGER PRIMARY KEY AUTOINCREMENT, at INTEGER NOT NULL,
               role TEXT NOT NULL, content TEXT NOT NULL);
             -- One control row: the chain head, the anchor left by a retention purge (the hash
             -- of the last purged record), and the purge gate (purge_upto, NULL when closed).
             CREATE TABLE IF NOT EXISTS audit_meta (
               only_row INTEGER PRIMARY KEY CHECK (only_row = 1),
               head_seq INTEGER NOT NULL, head_hash TEXT NOT NULL,
               anchor_seq INTEGER NOT NULL, anchor_hash TEXT NOT NULL,
               purge_upto INTEGER);
             INSERT OR IGNORE INTO audit_meta VALUES (1, 0, '0000000000000000000000000000000000000000000000000000000000000000', 0, '0000000000000000000000000000000000000000000000000000000000000000', NULL);
             -- Rows are append-only. Each insert must be the next seq and link to the head, so
             -- REPLACE cannot overwrite a row even with recursive triggers off.
             CREATE TRIGGER IF NOT EXISTS audit_insert_guard BEFORE INSERT ON audit
               WHEN NEW.seq IS NOT (SELECT head_seq + 1 FROM audit_meta)
                 OR NEW.prev_hash IS NOT (SELECT head_hash FROM audit_meta)
               BEGIN SELECT RAISE(ABORT, 'audit is append-only'); END;
             CREATE TRIGGER IF NOT EXISTS audit_head_advance AFTER INSERT ON audit
               BEGIN UPDATE audit_meta SET head_seq = NEW.seq, head_hash = NEW.hash; END;
             CREATE TRIGGER IF NOT EXISTS audit_no_update BEFORE UPDATE ON audit
               BEGIN SELECT RAISE(ABORT, 'audit is append-only'); END;
             CREATE TRIGGER IF NOT EXISTS audit_no_delete BEFORE DELETE ON audit
               WHEN OLD.seq > COALESCE((SELECT purge_upto FROM audit_meta), -1)
               BEGIN SELECT RAISE(ABORT, 'audit is append-only'); END;
             -- The control row moves only in three shapes: head advance (from the insert trigger),
             -- open the purge gate up to a seq (anchor set to that row's hash), close the gate.
             CREATE TRIGGER IF NOT EXISTS audit_meta_guard BEFORE UPDATE ON audit_meta
               WHEN NOT (
                 (NEW.head_seq = OLD.head_seq + 1
                  AND NEW.head_hash IS (SELECT hash FROM audit WHERE seq = NEW.head_seq)
                  AND NEW.anchor_seq = OLD.anchor_seq AND NEW.anchor_hash = OLD.anchor_hash
                  AND NEW.purge_upto IS OLD.purge_upto)
                 OR
                 (OLD.purge_upto IS NULL AND NEW.purge_upto IS NOT NULL
                  AND NEW.purge_upto > OLD.anchor_seq AND NEW.purge_upto <= OLD.head_seq
                  AND NEW.anchor_seq = NEW.purge_upto
                  AND NEW.anchor_hash IS (SELECT hash FROM audit WHERE seq = NEW.purge_upto)
                  AND NEW.head_seq = OLD.head_seq AND NEW.head_hash = OLD.head_hash)
                 OR
                 (OLD.purge_upto IS NOT NULL AND NEW.purge_upto IS NULL
                  AND NEW.head_seq = OLD.head_seq AND NEW.head_hash = OLD.head_hash
                  AND NEW.anchor_seq = OLD.anchor_seq AND NEW.anchor_hash = OLD.anchor_hash))
               BEGIN SELECT RAISE(ABORT, 'audit control row is guarded'); END;
             CREATE TRIGGER IF NOT EXISTS audit_meta_no_insert BEFORE INSERT ON audit_meta
               BEGIN SELECT RAISE(ABORT, 'audit control row is guarded'); END;
             CREATE TRIGGER IF NOT EXISTS audit_meta_no_delete BEFORE DELETE ON audit_meta
               BEGIN SELECT RAISE(ABORT, 'audit control row is guarded'); END;
             PRAGMA user_version = 1;
             COMMIT;",
        )?;
        Ok(())
    }

    /// Notes table (#44). Additive and independent of `user_version`: `IF NOT EXISTS`, so it is
    /// safe to run on every open and does not collide with other schema work. Notes hang off the
    /// task id, not a version, so they travel with the task across versions. `scope` is 'device'
    /// today; the shared library (#29, #41) will add 'team'. Note text may hold personal data, so
    /// it lives only in this encrypted file and no error or log line carries it.
    fn ensure_notes(&self) -> Result<()> {
        self.conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS task_notes (
               id INTEGER PRIMARY KEY AUTOINCREMENT, task_id TEXT NOT NULL,
               author TEXT NOT NULL, at INTEGER NOT NULL, body TEXT NOT NULL,
               scope TEXT NOT NULL DEFAULT 'device');
             CREATE INDEX IF NOT EXISTS task_notes_by_task ON task_notes (task_id, id);",
        )?;
        Ok(())
    }

    /// Task versions' verification state (#42). Additive and idempotent, kept apart from `migrate` and
    /// `user_version` so it touches no existing table. Holds step ids and states only, never a value.
    /// `was_verified` is 1 once a run of the version has finished, and stays 1 if it is later flagged.
    fn migrate_verification(&self) -> Result<()> {
        self.conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS step_passes (
               task_id TEXT NOT NULL, version INTEGER NOT NULL, step TEXT NOT NULL, at INTEGER NOT NULL,
               PRIMARY KEY (task_id, version, step));
             CREATE TABLE IF NOT EXISTS version_state (
               task_id TEXT NOT NULL, version INTEGER NOT NULL,
               state TEXT NOT NULL CHECK (state IN ('verified', 'needs_verification')),
               step TEXT CHECK (step IS NULL OR (length(step) BETWEEN 2 AND 12 AND step GLOB 's[0-9]*'
                                                 AND substr(step, 2) NOT GLOB '*[^0-9]*')),
               was_verified INTEGER NOT NULL DEFAULT 0, at INTEGER NOT NULL,
               PRIMARY KEY (task_id, version));",
        )?;
        Ok(())
    }

    /// Adds a note to a saved task. Returns the note id. Fails if the task does not exist.
    pub fn add_note(&self, task_id: &str, author: &str, body: &str, now: i64) -> Result<i64> {
        let exists: bool = self.conn.query_row(
            "SELECT EXISTS (SELECT 1 FROM tasks WHERE id = ?1)",
            [task_id],
            |r| r.get(0),
        )?;
        if !exists {
            return Err(StoreError::Db("no such task".into()));
        }
        self.conn.execute(
            "INSERT INTO task_notes (task_id, author, at, body) VALUES (?1, ?2, ?3, ?4)",
            params![task_id, author, now, body],
        )?;
        Ok(self.conn.last_insert_rowid())
    }

    /// A task's notes, oldest first: (id, author, at, body).
    pub fn notes_for(&self, task_id: &str) -> Result<Vec<(i64, String, i64, String)>> {
        let mut st = self
            .conn
            .prepare("SELECT id, author, at, body FROM task_notes WHERE task_id = ?1 ORDER BY id")?;
        let rows = st
            .query_map([task_id], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))?
            .collect::<std::result::Result<_, _>>()?;
        Ok(rows)
    }

    /// Deletes one note of a task. True if a note was removed.
    pub fn delete_note(&self, task_id: &str, note_id: i64) -> Result<bool> {
        Ok(self.conn.execute(
            "DELETE FROM task_notes WHERE task_id = ?1 AND id = ?2",
            params![task_id, note_id],
        )? > 0)
    }

    /// Records one replay's result for a task version, in one transaction (#42). Steps that completed are
    /// remembered as passed. A divergence that means the interface changed (`interface_changed`, from the
    /// executor) on a step that had passed before marks the version `needs_verification`; a finished run
    /// marks it `verified`. Returns the state it set, if it set one. Step ids (`s` and digits) and states
    /// only: anything else is dropped, never stored. A task version that does not exist is an error. An
    /// older run never overwrites a newer one's state. No old version is touched or removed.
    #[allow(clippy::too_many_arguments)]
    pub fn record_replay_outcome(
        &mut self,
        task_id: &str,
        version: i64,
        outcome: &str,
        diverged_step: Option<&str>,
        interface_changed: bool,
        completed: &[String],
        now: i64,
    ) -> Result<Option<&'static str>> {
        let tx = self.conn.transaction()?;
        let exists: bool = tx.query_row(
            "SELECT count(*) > 0 FROM tasks WHERE id = ?1 AND version = ?2",
            params![task_id, version],
            |r| r.get(0),
        )?;
        if !exists {
            return Err(StoreError::Db("no such task version".into()));
        }
        let diverged_step = diverged_step.filter(|s| valid_step_id(s));
        let mut set = None;
        if outcome == "diverged" && interface_changed {
            if let Some(step) = diverged_step {
                let passed: bool = tx.query_row(
                    "SELECT count(*) > 0 FROM step_passes WHERE task_id = ?1 AND version = ?2 AND step = ?3",
                    params![task_id, version, step],
                    |r| r.get(0),
                )?;
                if passed {
                    set = Some("needs_verification");
                }
            }
        } else if outcome == "finished" {
            set = Some("verified");
        }
        if let Some(state) = set {
            let step = if state == "verified" { None } else { diverged_step };
            let verified = i64::from(state == "verified");
            let changed = tx.execute(
                "INSERT INTO version_state (task_id, version, state, step, was_verified, at) VALUES (?1, ?2, ?3, ?4, ?5, ?6)
                 ON CONFLICT (task_id, version) DO UPDATE SET
                   was_verified = CASE WHEN excluded.state = 'verified' OR version_state.state = 'verified'
                                       OR version_state.was_verified = 1 THEN 1 ELSE 0 END,
                   state = excluded.state, step = excluded.step, at = excluded.at
                 WHERE excluded.at >= version_state.at",
                params![task_id, version, state, step, verified, now],
            )?;
            if changed == 0 {
                set = None;
            }
        }
        for s in completed.iter().filter(|s| valid_step_id(s)) {
            tx.execute(
                "INSERT OR IGNORE INTO step_passes (task_id, version, step, at) VALUES (?1, ?2, ?3, ?4)",
                params![task_id, version, s, now],
            )?;
        }
        tx.commit()?;
        Ok(set)
    }

    /// Versions of a task that have a recorded state, in version order.
    pub fn version_states(&self, task_id: &str) -> Result<Vec<VersionState>> {
        let mut st = self.conn.prepare(
            "SELECT version, state, step, was_verified FROM version_state WHERE task_id = ?1 ORDER BY version",
        )?;
        let rows = st
            .query_map([task_id], |r| {
                Ok(VersionState { version: r.get(0)?, state: r.get(1)?, step: r.get(2)?, was_verified: r.get::<_, i64>(3)? == 1 })
            })?
            .collect::<std::result::Result<_, _>>()?;
        Ok(rows)
    }

    /// Saves a task as a new version (max + 1; 1 for a new id). Older versions are kept.
    pub fn save_task(&mut self, id: &str, name: &str, body: &str, now: i64) -> Result<i64> {
        let tx = self.conn.transaction()?;
        let next: i64 = tx.query_row(
            "SELECT COALESCE(MAX(version), 0) + 1 FROM tasks WHERE id = ?1",
            [id],
            |r| r.get(0),
        )?;
        tx.execute(
            "INSERT INTO tasks (id, version, name, body, saved_at) VALUES (?1, ?2, ?3, ?4, ?5)",
            params![id, next, name, body, now],
        )?;
        tx.commit()?;
        Ok(next)
    }

    /// Saves a task as the next version and appends its `task_saved` audit record in the same
    /// transaction, so a task never exists without its record or the reverse. `build` gets the
    /// version number and returns the body; an error from it rolls everything back. The audit
    /// record names the task and officer only, never a value.
    pub fn save_task_audited(
        &mut self,
        id: &str,
        name: &str,
        officer: &str,
        now: i64,
        build: impl FnOnce(i64) -> Result<String>,
    ) -> Result<i64> {
        let tx = self.conn.transaction()?;
        let next: i64 = tx.query_row(
            "SELECT COALESCE(MAX(version), 0) + 1 FROM tasks WHERE id = ?1",
            [id],
            |r| r.get(0),
        )?;
        let body = build(next)?;
        tx.execute(
            "INSERT INTO tasks (id, version, name, body, saved_at) VALUES (?1, ?2, ?3, ?4, ?5)",
            params![id, next, name, body, now],
        )?;
        let reference = format!("{id}@{next}");
        let e = AuditEntry {
            at: now,
            officer,
            event: "task_saved",
            task: Some(&reference),
            run: None,
            item: None,
            step: None,
            shown: None,
        };
        append_in(&tx, &e)?;
        tx.commit()?;
        Ok(next)
    }

    /// The newest version of every saved task: (id, version, body). Read-only.
    pub fn latest_tasks(&self) -> Result<Vec<(String, i64, String)>> {
        let mut st = self.conn.prepare(
            "SELECT t.id, t.version, t.body FROM tasks t
             WHERE t.version = (SELECT MAX(version) FROM tasks WHERE id = t.id)
             ORDER BY t.saved_at, t.id",
        )?;
        let rows = st.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?.collect::<std::result::Result<_, _>>()?;
        Ok(rows)
    }

    pub fn task_versions(&self, id: &str) -> Result<Vec<i64>> {
        let mut st = self.conn.prepare("SELECT version FROM tasks WHERE id = ?1 ORDER BY version")?;
        let rows = st.query_map([id], |r| r.get(0))?.collect::<std::result::Result<_, _>>()?;
        Ok(rows)
    }

    pub fn task_body(&self, id: &str, version: i64) -> Result<Option<String>> {
        Ok(self
            .conn
            .query_row(
                "SELECT body FROM tasks WHERE id = ?1 AND version = ?2",
                params![id, version],
                |r| r.get(0),
            )
            .optional()?)
    }

    pub fn start_run(
        &self,
        id: &str,
        task_id: &str,
        task_version: i64,
        inputs_source: &str,
        now: i64,
    ) -> Result<()> {
        self.conn.execute(
            "INSERT INTO runs (id, task_id, task_version, inputs_source, status, started_at)
             VALUES (?1, ?2, ?3, ?4, 'running', ?5)",
            params![id, task_id, task_version, inputs_source, now],
        )?;
        Ok(())
    }

    /// Sets a run's final status ('finished', 'stopped', 'refused').
    pub fn finish_run(&self, id: &str, status: &str) -> Result<()> {
        self.conn.execute("UPDATE runs SET status = ?2 WHERE id = ?1", params![id, status])?;
        Ok(())
    }

    /// A task's reuse on this device: finished runs, and when the latest clean one began (#43). Clean means
    /// no row ended diverged, rejected, unknown or skipped. Counts and a time only; no officer is read.
    pub fn reuse_counts(&self, task_id: &str) -> Result<(i64, Option<i64>)> {
        Ok(self.conn.query_row(
            "SELECT count(*),
                    max(CASE WHEN NOT EXISTS (SELECT 1 FROM run_items i WHERE i.run_id = runs.id
                                              AND i.state IN ('diverged', 'rejected', 'unknown', 'skipped'))
                             THEN started_at END)
             FROM runs WHERE task_id = ?1 AND status = 'finished'",
            [task_id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )?)
    }

    pub fn set_run_item(
        &self,
        run_id: &str,
        item: i64,
        state: &str,
        last_step: Option<&str>,
    ) -> Result<()> {
        self.conn.execute(
            "INSERT INTO run_items (run_id, item, state, last_step) VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT (run_id, item) DO UPDATE SET state = excluded.state, last_step = excluded.last_step",
            params![run_id, item, state, last_step],
        )?;
        Ok(())
    }

    /// Creates a list run's rows 1..=n, all `pending`. Rows are numbered from 1, the same number the audit
    /// trail's `item` carries. All or none.
    pub fn create_run_items(&mut self, run_id: &str, n: i64) -> Result<()> {
        let tx = self.conn.transaction()?;
        for item in 1..=n {
            tx.execute(
                "INSERT INTO run_items (run_id, item, state, last_step) VALUES (?1, ?2, 'pending', NULL)",
                params![run_id, item],
            )?;
        }
        tx.commit()?;
        Ok(())
    }

    /// Creates a list run and its `n` rows, only if the task has no unfinished list run (#108). The check and
    /// both inserts are one immediate transaction, so two shells on one store cannot both create one. Returns
    /// the unfinished runs that stood in the way; empty means this run was created.
    pub fn start_list_run(
        &mut self,
        id: &str,
        task_id: &str,
        task_version: i64,
        inputs_source: &str,
        n: i64,
        now: i64,
    ) -> Result<Vec<OpenRun>> {
        let tx = self.conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let open = open_runs_of(&tx, task_id, id)?;
        if open.is_empty() {
            tx.execute(
                "INSERT INTO runs (id, task_id, task_version, inputs_source, status, started_at)
                 VALUES (?1, ?2, ?3, ?4, 'running', ?5)",
                params![id, task_id, task_version, inputs_source, now],
            )?;
            for item in 1..=n {
                tx.execute(
                    "INSERT INTO run_items (run_id, item, state, last_step) VALUES (?1, ?2, 'pending', NULL)",
                    params![id, item],
                )?;
            }
            tx.commit()?;
        }
        Ok(open)
    }

    /// The unfinished list runs of one task, newest first.
    pub fn open_list_runs_of(&self, task_id: &str) -> Result<Vec<OpenRun>> {
        open_runs_of(&self.conn, task_id, "")
    }

    /// Marks a list run `running`, only if no OTHER list run of its task is unfinished, in one immediate
    /// transaction (this also guards reopening a finished run). Returns those others; empty means it was marked.
    pub fn mark_list_run_running(&mut self, id: &str, task_id: &str) -> Result<Vec<OpenRun>> {
        let tx = self.conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let others = open_runs_of(&tx, task_id, id)?;
        if others.is_empty() {
            tx.execute("UPDATE runs SET status = 'running' WHERE id = ?1", params![id])?;
            tx.commit()?;
        }
        Ok(others)
    }

    /// Every row of a run, in order: (item, state, last_step).
    pub fn run_items(&self, run_id: &str) -> Result<Vec<(i64, String, Option<String>)>> {
        let mut st = self
            .conn
            .prepare("SELECT item, state, last_step FROM run_items WHERE run_id = ?1 ORDER BY item")?;
        let rows = st.query_map(params![run_id], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?;
        Ok(rows.collect::<std::result::Result<Vec<_>, _>>()?)
    }

    pub fn item_state(&self, run_id: &str, item: i64) -> Result<Option<String>> {
        Ok(self
            .conn
            .query_row(
                "SELECT state FROM run_items WHERE run_id = ?1 AND item = ?2",
                params![run_id, item],
                |r| r.get(0),
            )
            .optional()?)
    }

    /// Moves a row from one of `from` to `to`, in one statement, so two callers cannot both win. Returns
    /// whether this call made the move. A row that is not in a `from` state is left as it is.
    pub fn move_run_item(
        &self,
        run_id: &str,
        item: i64,
        from: &[&str],
        to: &str,
        last_step: Option<&str>,
    ) -> Result<bool> {
        let marks = vec!["?"; from.len()].join(",");
        let sql = format!(
            "UPDATE run_items SET state = ?1, last_step = COALESCE(?2, last_step)
             WHERE run_id = ?3 AND item = ?4 AND state IN ({marks})"
        );
        let mut args: Vec<&dyn rusqlite::ToSql> = vec![&to, &last_step, &run_id, &item];
        for f in from {
            args.push(f);
        }
        Ok(self.conn.execute(&sql, args.as_slice())? == 1)
    }

    /// Takes the lease on a run, or None if another holder has it. See `RunLease`.
    pub fn lease_run(&self, run_id: &str) -> Result<Option<RunLease>> {
        use std::os::unix::fs::OpenOptionsExt;
        let safe: String = run_id.chars().filter(|c| c.is_ascii_alphanumeric() || *c == '_').collect();
        let mut name = self.path.as_os_str().to_owned();
        name.push(format!(".lease-{safe}"));
        let f = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .mode(0o600)
            .open(std::path::PathBuf::from(name))
            .map_err(|e| StoreError::Db(format!("cannot open the run lease: {}", e.kind())))?;
        match f.try_lock() {
            Ok(()) => Ok(Some(RunLease(f))),
            Err(std::fs::TryLockError::WouldBlock) => Ok(None),
            Err(std::fs::TryLockError::Error(e)) => Err(StoreError::Db(format!("cannot lock the run: {}", e.kind()))),
        }
    }

    /// List runs that are not over (not finished, not ended by the officer): (run, task, version, status).
    /// Newest first. `inputs_source` is parsed by the caller's rules: only runs that came from a file.
    pub fn open_list_runs(&self) -> Result<Vec<(String, String, i64, String, String)>> {
        let mut st = self.conn.prepare(
            "SELECT id, task_id, task_version, status, inputs_source FROM runs
             WHERE status NOT IN ('finished', 'ended') AND inputs_source LIKE '{%' ORDER BY started_at DESC, id DESC",
        )?;
        let rows = st.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?)))?;
        Ok(rows.collect::<std::result::Result<Vec<_>, _>>()?)
    }

    /// A run's task, version, inputs source and status.
    pub fn run_info(&self, run_id: &str) -> Result<Option<(String, i64, String, String)>> {
        Ok(self
            .conn
            .query_row(
                "SELECT task_id, task_version, inputs_source, status FROM runs WHERE id = ?1",
                params![run_id],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
            )
            .optional()?)
    }

    pub fn add_chat(&self, at: i64, role: &str, content: &str) -> Result<()> {
        self.conn.execute(
            "INSERT INTO chat (at, role, content) VALUES (?1, ?2, ?3)",
            params![at, role, content],
        )?;
        Ok(())
    }

    /// Appends a hash-chained audit record. Returns its seq.
    pub fn append_audit(&mut self, e: &AuditEntry) -> Result<i64> {
        let tx = self.conn.transaction()?;
        let seq = append_in(&tx, e)?;
        tx.commit()?;
        Ok(seq)
    }

    /// Appends one record of a TD 7.2 event. Unlike `append_audit`, an event name outside the list is an
    /// error, so a typo or a made-up event cannot enter the trail. `audit_purged` is the store's own.
    pub fn append_event(&mut self, e: &AuditEntry) -> Result<i64> {
        if !AUDIT_EVENTS.contains(&e.event) || e.event == "audit_purged" {
            return Err(StoreError::Db("unknown audit event".into()));
        }
        self.append_audit(e)
    }

    /// The trail as JSON lines for `executor-macos verify-audit`: a control line (anchor and head), then
    /// one object per record in seq order. It holds the approved values (decision 0005): send it to the
    /// verifier's standard input, never to a file or a log.
    pub fn export_audit(&self) -> Result<String> {
        let (head_seq, head_hash, anchor_seq, anchor_hash): (i64, String, i64, String) =
            self.conn.query_row(
                "SELECT head_seq, head_hash, anchor_seq, anchor_hash FROM audit_meta",
                [],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
            )?;
        let mut out = serde_json::json!({
            "anchor_seq": anchor_seq, "anchor_hash": anchor_hash,
            "head_seq": head_seq, "head_hash": head_hash,
        })
        .to_string();
        out.push('\n');
        let mut st = self.conn.prepare(
            "SELECT seq, at, officer, event, task, run, item, step, shown, prev_hash, hash
             FROM audit ORDER BY seq",
        )?;
        let mut rows = st.query([])?;
        while let Some(r) = rows.next()? {
            let rec = serde_json::json!({
                "seq": r.get::<_, i64>(0)?, "at": r.get::<_, i64>(1)?,
                "officer": r.get::<_, String>(2)?, "event": r.get::<_, String>(3)?,
                "task": r.get::<_, Option<String>>(4)?, "run": r.get::<_, Option<String>>(5)?,
                "item": r.get::<_, Option<i64>>(6)?, "step": r.get::<_, Option<String>>(7)?,
                "shown": r.get::<_, Option<String>>(8)?,
                "prev_hash": r.get::<_, String>(9)?, "hash": r.get::<_, String>(10)?,
            });
            out.push_str(&rec.to_string());
            out.push('\n');
        }
        Ok(out)
    }

    /// Walks the chain from the anchor (zeros, or what the last retention purge left) and checks
    /// it ends at the recorded head, so a gap, an edit, a truncated tail or a wipe all fail.
    pub fn verify_audit(&self) -> Result<()> {
        let (head_seq, head_hash, anchor_seq, anchor_hash): (i64, String, i64, String) =
            self.conn.query_row(
                "SELECT head_seq, head_hash, anchor_seq, anchor_hash FROM audit_meta",
                [],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
            )?;
        let mut st = self.conn.prepare(
            "SELECT seq, at, officer, event, task, run, item, step, shown, prev_hash, hash
             FROM audit ORDER BY seq",
        )?;
        let mut rows = st.query([])?;
        let (mut next_seq, mut next_prev) = (anchor_seq + 1, anchor_hash);
        while let Some(r) = rows.next()? {
            let seq: i64 = r.get(0)?;
            let prev: String = r.get(9)?;
            let hash: String = r.get(10)?;
            let calc = audit_hash(
                &prev,
                r.get(1)?,
                &r.get::<_, String>(2)?,
                &r.get::<_, String>(3)?,
                r.get::<_, Option<String>>(4)?.as_deref(),
                r.get::<_, Option<String>>(5)?.as_deref(),
                r.get(6)?,
                r.get::<_, Option<String>>(7)?.as_deref(),
                r.get::<_, Option<String>>(8)?.as_deref(),
            );
            // A gap names the record that should be there, the same as executor-macos verify-audit.
            if seq != next_seq {
                return Err(StoreError::Chain(next_seq));
            }
            if prev != next_prev || calc != hash {
                return Err(StoreError::Chain(seq));
            }
            next_seq += 1;
            next_prev = hash;
        }
        if next_seq - 1 != head_seq || next_prev != head_hash {
            return Err(StoreError::Chain(next_seq));
        }
        Ok(())
    }

    pub fn audit_count(&self) -> Result<i64> {
        Ok(self.conn.query_row("SELECT count(*) FROM audit", [], |r| r.get(0))?)
    }

    /// Retention (the only delete path). Removes the contiguous oldest prefix of records whose
    /// `at` is older than `now - retention_days`, stopping at the first record that is not.
    /// It takes an age, not row ids or content, so it cannot edit a row. It leaves an anchor so
    /// the rest still verifies, and appends an `audit_purged` record (count in `item`).
    /// `retention_days` of 0 is refused. Returns rows removed.
    pub fn purge_expired(&mut self, now: i64, retention_days: u32) -> Result<usize> {
        if retention_days == 0 {
            return Err(StoreError::Db("retention must be at least 1 day".into()));
        }
        let cutoff = now - i64::from(retention_days) * SECONDS_PER_DAY;
        let tx = self.conn.transaction()?;
        let (head_seq, anchor_seq): (i64, i64) = tx.query_row(
            "SELECT head_seq, anchor_seq FROM audit_meta",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )?;
        let first_kept: Option<i64> =
            tx.query_row("SELECT MIN(seq) FROM audit WHERE at >= ?1", [cutoff], |r| r.get(0))?;
        let upto = first_kept.map_or(head_seq, |s| s - 1);
        if upto <= anchor_seq {
            return Ok(0);
        }
        tx.execute(
            "UPDATE audit_meta SET purge_upto = ?1, anchor_seq = ?1,
             anchor_hash = (SELECT hash FROM audit WHERE seq = ?1)",
            [upto],
        )?;
        let n = tx.execute("DELETE FROM audit WHERE seq <= ?1", [upto])?;
        tx.execute("UPDATE audit_meta SET purge_upto = NULL", [])?;
        let note = AuditEntry {
            at: now,
            officer: "system",
            event: "audit_purged",
            task: None,
            run: None,
            item: Some(n as i64),
            step: None,
            shown: None,
        };
        append_in(&tx, &note)?;
        tx.commit()?;
        Ok(n)
    }
}

/// An unfinished list run of a task.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OpenRun {
    pub id: String,
    pub version: i64,
    pub inputs_source: String,
}

/// "Unfinished" means the same as in `open_list_runs`: not finished, not ended, and started from a file.
fn open_runs_of(conn: &Connection, task_id: &str, except: &str) -> Result<Vec<OpenRun>> {
    let mut st = conn.prepare(
        "SELECT id, task_version, inputs_source FROM runs
         WHERE task_id = ?1 AND id <> ?2 AND status NOT IN ('finished', 'ended') AND inputs_source LIKE '{%'
         ORDER BY started_at DESC, id DESC",
    )?;
    let rows = st.query_map(params![task_id, except], |r| {
        Ok(OpenRun { id: r.get(0)?, version: r.get(1)?, inputs_source: r.get(2)? })
    })?;
    Ok(rows.collect::<std::result::Result<Vec<_>, _>>()?)
}

fn append_in(tx: &rusqlite::Transaction, e: &AuditEntry) -> Result<i64> {
    let (head_seq, prev): (i64, String) = tx.query_row(
        "SELECT head_seq, head_hash FROM audit_meta",
        [],
        |r| Ok((r.get(0)?, r.get(1)?)),
    )?;
    let seq = head_seq + 1;
    let hash = audit_hash(&prev, e.at, e.officer, e.event, e.task, e.run, e.item, e.step, e.shown);
    tx.execute(
        "INSERT INTO audit (seq, at, officer, event, task, run, item, step, shown, prev_hash, hash)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
        params![seq, e.at, e.officer, e.event, e.task, e.run, e.item, e.step, e.shown, prev, hash],
    )?;
    Ok(seq)
}

#[allow(clippy::too_many_arguments)]
fn audit_hash(
    prev: &str,
    at: i64,
    officer: &str,
    event: &str,
    task: Option<&str>,
    run: Option<&str>,
    item: Option<i64>,
    step: Option<&str>,
    shown: Option<&str>,
) -> String {
    let mut h = Sha256::new();
    // Length-prefixed fields with a presence byte, so no two different records share an encoding.
    let mut put = |s: Option<&str>| match s {
        None => h.update([0u8]),
        Some(s) => {
            h.update([1u8]);
            h.update((s.len() as u64).to_be_bytes());
            h.update(s.as_bytes());
        }
    };
    put(Some(prev));
    put(Some(&at.to_string()));
    put(Some(officer));
    put(Some(event));
    put(task);
    put(run);
    put(step);
    put(shown);
    put(item.map(|i| i.to_string()).as_deref());
    h.finalize().iter().map(|b| format!("{b:02x}")).collect()
}
