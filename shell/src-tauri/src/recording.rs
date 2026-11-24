//! Starts and stops the executor's `record` run (TD 5.1). The executor owns the event tap and
//! the recording indicator; the shell only starts it, signals stop, and opens the review when the
//! file appears. The file is written by the executor only at stop, descriptors and input names only.

use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::mpsc;
use std::time::Duration;

/// Ceiling for the executor to finish writing after a stop signal. A bound on waiting, not a pause.
const STOP_CEILING: Duration = Duration::from_secs(15);

pub struct Recording {
    child: Child,
    out: PathBuf,
}

/// Where unreviewed recordings wait: a private folder next to the store.
pub fn drafts_dir() -> Option<PathBuf> {
    crate::store::default_path().and_then(|p| p.parent().map(|d| d.join("drafts")))
}

fn valid_bundle(b: &str) -> bool {
    !b.is_empty()
        && b.len() <= 255
        && b.chars().all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '-')
        && !b.starts_with('-')
}

fn private_dir(dir: &Path) -> Result<(), String> {
    use std::os::unix::fs::DirBuilderExt;
    std::fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(dir)
        .map_err(|e| format!("cannot create the drafts folder: {}", e.kind()))?;
    // A folder that already existed keeps its old mode unless it is set here.
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700))
        .map_err(|e| format!("cannot secure the drafts folder: {}", e.kind()))
}

/// Deletes leftover `draft-*.json` files (a review window closed without save or discard, or a
/// refused recording). Regular files only; a link is removed, never followed.
pub fn sweep(dir: &Path, keep: Option<&Path>) {
    let Ok(rd) = std::fs::read_dir(dir) else { return };
    for e in rd.flatten() {
        let p = e.path();
        let name = e.file_name();
        let name = name.to_string_lossy();
        if name.starts_with("draft-") && name.ends_with(".json") && Some(p.as_path()) != keep {
            let _ = std::fs::remove_file(&p);
        }
    }
}

pub fn start(bin: &Path, app_bundle: &str, dir: &Path) -> Result<Recording, String> {
    if !valid_bundle(app_bundle) {
        return Err("that is not an application bundle id".into());
    }
    private_dir(dir)?;
    sweep(dir, None);
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let out = dir.join(format!("draft-{}-{nanos}.json", std::process::id()));
    let child = Command::new(bin)
        .args(["record", "--app", app_bundle, "--out"])
        .arg(&out)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|e| format!("cannot start the recorder: {}", e.kind()))?;
    Ok(Recording { child, out })
}

/// Signals stop (SIGINT, which the executor handles by turning the tap and indicator off and
/// then writing the file), waits for it to exit, and returns the file's path.
pub fn stop(mut rec: Recording) -> Result<PathBuf, String> {
    let pid = rec.child.id().to_string();
    let signalled = Command::new("/bin/kill").args(["-INT", &pid]).status().is_ok_and(|s| s.success());
    if !signalled {
        let _ = rec.child.kill();
        let _ = rec.child.wait();
        return Err("could not tell the recorder to stop".into());
    }
    let (tx, rx) = mpsc::channel();
    let mut child = rec.child;
    let ceiling_pid = pid.clone();
    std::thread::spawn(move || {
        let _ = tx.send(child.wait());
    });
    match rx.recv_timeout(STOP_CEILING) {
        Ok(Ok(status)) if status.success() && rec.out.exists() => Ok(rec.out),
        Ok(_) => Err("the recording was not saved (no steps were recorded, or the recorder failed)".into()),
        Err(_) => {
            // A recorder that ignores the stop must not keep its tap and indicator running. The
            // waiting thread reaps it once it is gone.
            let _ = Command::new("/bin/kill").args(["-KILL", &ceiling_pid]).status();
            Err("the recorder did not stop in time and was ended".into())
        }
    }
}
