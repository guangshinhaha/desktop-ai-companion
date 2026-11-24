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

impl Keychain {
    fn read(&self) -> Option<Result<Key>> {
        let existing =
            security_framework::passwords::get_generic_password(&self.service, &self.account).ok()?;
        Some(
            <[u8; 32]>::try_from(existing)
                .map(Key)
                .map_err(|_| StoreError::Key("stored key has the wrong length".into())),
        )
    }
}

impl KeySource for Keychain {
    /// Get, else create with an add-only call. If two first opens race, one add succeeds and the
    /// other gets a duplicate error and reads the winner's key, so both end with the same key.
    fn key_and_origin(&self) -> Result<(Key, bool)> {
        use core_foundation::data::CFData;
        use security_framework::item::{ItemAddOptions, ItemAddValue, ItemClass};
        if let Some(k) = self.read() {
            return Ok((k?, false));
        }
        let mut fresh = [0u8; 32];
        std::fs::File::open("/dev/urandom")
            .and_then(|mut f| std::io::Read::read_exact(&mut f, &mut fresh))
            .map_err(|e| StoreError::Key(e.to_string()))?;
        let added = ItemAddOptions::new(ItemAddValue::Data {
            class: ItemClass::generic_password(),
            data: CFData::from_buffer(&fresh),
        })
        .set_service(&self.service)
        .set_account_name(&self.account)
        .add();
        match added {
            Ok(()) => Ok((Key(fresh), true)),
            Err(e) if e.code() == DUPLICATE_ITEM => match self.read() {
                Some(k) => Ok((k?, false)),
                None => Err(StoreError::Key("key item vanished during creation".into())),
            },
            Err(e) => Err(StoreError::Key(e.to_string())),
        }
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

pub struct Store {
    conn: Connection,
}

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
        let seq = append_in(&tx, e)?;
        tx.commit()?;
        Ok(seq)
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
            if seq != next_seq || prev != next_prev || calc != hash {
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
