//! Local store (TD 3.4): one SQLCipher database for tasks, runs, run items, audit and chat.
//! The key lives in the macOS keychain. No network. Nothing here logs or prints a value:
//! errors carry a kind, never row content.

use rusqlite::{params, Connection, OptionalExtension};
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
    fn key(&self) -> Result<Key>;
}

pub struct Fixed(pub [u8; 32]);
impl KeySource for Fixed {
    fn key(&self) -> Result<Key> {
        Ok(Key(self.0))
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

impl KeySource for Keychain {
    fn key(&self) -> Result<Key> {
        use security_framework::passwords::{get_generic_password, set_generic_password};
        if let Ok(existing) = get_generic_password(&self.service, &self.account) {
            let arr: [u8; 32] = existing
                .try_into()
                .map_err(|_| StoreError::Key("stored key has the wrong length".into()))?;
            return Ok(Key(arr));
        }
        let mut fresh = [0u8; 32];
        std::fs::File::open("/dev/urandom")
            .and_then(|mut f| std::io::Read::read_exact(&mut f, &mut fresh))
            .map_err(|e| StoreError::Key(e.to_string()))?;
        set_generic_password(&self.service, &self.account, &fresh)
            .map_err(|e| StoreError::Key(e.to_string()))?;
        Ok(Key(fresh))
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

pub struct Store {
    conn: Connection,
}

const SCHEMA_VERSION: i64 = 1;

impl Store {
    pub fn open(path: &Path, keys: &dyn KeySource) -> Result<Store> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).map_err(|e| StoreError::Db(e.to_string()))?;
        }
        let conn = Connection::open(path)?;
        conn.pragma_update(None, "key", keys.key()?.pragma())?;
        // Forces SQLCipher to decrypt page 1: a wrong key fails here, not later.
        conn.query_row("SELECT count(*) FROM sqlite_master", [], |r| r.get::<_, i64>(0))?;
        let store = Store { conn };
        store.migrate()?;
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
               seq INTEGER PRIMARY KEY AUTOINCREMENT, at INTEGER NOT NULL,
               officer TEXT NOT NULL, event TEXT NOT NULL, task TEXT, run TEXT,
               item INTEGER, step TEXT, shown TEXT,
               prev_hash TEXT NOT NULL, hash TEXT NOT NULL);
             CREATE TABLE IF NOT EXISTS chat (
               id INTEGER PRIMARY KEY AUTOINCREMENT, at INTEGER NOT NULL,
               role TEXT NOT NULL, content TEXT NOT NULL);
             -- Retention gate: normally NULL. Only purge_audit_before sets it, inside one
             -- transaction, to a cutoff; the delete trigger allows only rows older than it.
             CREATE TABLE IF NOT EXISTS audit_purge_gate (
               only_row INTEGER PRIMARY KEY CHECK (only_row = 1), cutoff INTEGER);
             INSERT OR IGNORE INTO audit_purge_gate (only_row, cutoff) VALUES (1, NULL);
             CREATE TRIGGER IF NOT EXISTS audit_no_update BEFORE UPDATE ON audit
               BEGIN SELECT RAISE(ABORT, 'audit is append-only'); END;
             CREATE TRIGGER IF NOT EXISTS audit_no_delete BEFORE DELETE ON audit
               WHEN OLD.at >= COALESCE((SELECT cutoff FROM audit_purge_gate WHERE only_row = 1), -9223372036854775807)
               BEGIN SELECT RAISE(ABORT, 'audit is append-only'); END;
             PRAGMA user_version = 1;
             COMMIT;",
        )?;
        Ok(())
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
        let prev: String = tx
            .query_row("SELECT hash FROM audit ORDER BY seq DESC LIMIT 1", [], |r| r.get(0))
            .optional()?
            .unwrap_or_else(|| "0".repeat(64));
        let hash = audit_hash(&prev, e.at, e.officer, e.event, e.task, e.run, e.item, e.step, e.shown);
        tx.execute(
            "INSERT INTO audit (at, officer, event, task, run, item, step, shown, prev_hash, hash)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
            params![e.at, e.officer, e.event, e.task, e.run, e.item, e.step, e.shown, prev, hash],
        )?;
        let seq = tx.last_insert_rowid();
        tx.commit()?;
        Ok(seq)
    }

    /// Walks the chain. After a retention purge the first surviving row is the anchor.
    pub fn verify_audit(&self) -> Result<()> {
        let mut st = self.conn.prepare(
            "SELECT seq, at, officer, event, task, run, item, step, shown, prev_hash, hash
             FROM audit ORDER BY seq",
        )?;
        let mut rows = st.query([])?;
        let mut expect_prev: Option<String> = None;
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
            if calc != hash || expect_prev.as_ref().is_some_and(|p| *p != prev) {
                return Err(StoreError::Chain(seq));
            }
            expect_prev = Some(hash);
        }
        Ok(())
    }

    pub fn audit_count(&self) -> Result<i64> {
        Ok(self.conn.query_row("SELECT count(*) FROM audit", [], |r| r.get(0))?)
    }

    /// Retention. Deletes only records older than `cutoff`, through the gate the delete trigger
    /// checks. It takes a cutoff, not row ids or content, so it cannot edit a row.
    /// Returns rows removed.
    pub fn purge_audit_before(&mut self, cutoff: i64) -> Result<usize> {
        let tx = self.conn.transaction()?;
        tx.execute("UPDATE audit_purge_gate SET cutoff = ?1 WHERE only_row = 1", [cutoff])?;
        let n = tx.execute("DELETE FROM audit WHERE at < ?1", [cutoff]);
        // Close the gate whether or not the delete worked; commit only on success.
        tx.execute("UPDATE audit_purge_gate SET cutoff = NULL WHERE only_row = 1", [])?;
        let n = n?;
        tx.commit()?;
        Ok(n)
    }

    /// Purges records older than `retention_days` before `now`.
    pub fn purge_expired(&mut self, now: i64, retention_days: u32) -> Result<usize> {
        self.purge_audit_before(now - i64::from(retention_days) * SECONDS_PER_DAY)
    }

    /// Raw connection, for tests that attempt forbidden writes.
    #[doc(hidden)]
    pub fn raw(&self) -> &Connection {
        &self.conn
    }
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
    // Length-prefixed fields, so two different records cannot hash the same bytes.
    let mut put = |s: &str| {
        h.update((s.len() as u64).to_be_bytes());
        h.update(s.as_bytes());
    };
    put(prev);
    put(&at.to_string());
    put(officer);
    put(event);
    for o in [task, run, step, shown] {
        put(o.unwrap_or("\u{0}none"));
    }
    put(&item.map_or("\u{0}none".to_string(), |i| i.to_string()));
    h.finalize().iter().map(|b| format!("{b:02x}")).collect()
}
