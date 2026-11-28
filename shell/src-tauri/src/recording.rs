//! Runs the executor's `record` run (TD 5.1). The executor owns the event tap and the recording
//! indicator. The shell starts it, waits for it to exit, and puts what it wrote under review.
//! The officer stops it from the REC menu, so the exit is the one signal that a recording ended.
//! The file is written by the executor only at stop, descriptors and input names only.

use crate::review::Draft;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::{mpsc, Arc, Condvar, Mutex, MutexGuard};
use std::time::Duration;

/// Ceiling for the executor to finish writing after a stop signal. A bound on waiting, not a pause.
const STOP_CEILING: Duration = Duration::from_secs(15);

/// A recorder run in progress.
pub struct Recording {
    watched: Arc<Watched>,
}

/// Asks a recorder to stop. Safe to hold after the run has ended.
pub struct Stopper(Arc<Watched>);

/// What the exit watcher and a stop share. The pid may be signalled only while `exited` is false
/// and the lock is held: the watcher sets it before it reaps, so the pid cannot have been reused.
struct Watched {
    pid: libc::pid_t,
    end: Mutex<End>,
    ended: Condvar,
}

#[derive(Default)]
struct End {
    exited: bool,
    /// The recorder ignored a stop and was killed. Nothing it left is kept.
    forced: bool,
}

/// Where recording stands. One value, so a run and a draft under review cannot both exist.
/// A stopping recorder is still `Recording` until it has exited and its file has been read.
#[derive(Default)]
pub enum Phase {
    #[default]
    Idle,
    Recording(Recording),
    /// Memory only, until save or discard.
    Review(Draft),
    /// The last recording gave nothing to review. Why, in words for the officer.
    Failed(String),
}

/// A panic elsewhere must not leave a finished recorder unnoticed.
fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
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

/// A draft under review lives in memory only, so any draft file present at launch is left over
/// from an earlier run (quit, crash or restart before save or discard). Nothing here is deleted
/// through a link: if the drafts path is not a real folder, launch leaves it alone.
pub fn sweep_at_launch(dir: Option<PathBuf>) {
    if let Some(d) = dir {
        if std::fs::symlink_metadata(&d).is_ok_and(|m| m.is_dir()) {
            sweep(&d, None);
        }
    }
}

/// Starts a recording unless one is going, stopping or under review. When the recorder exits,
/// however it was stopped, `phase` moves to `Review` or `Failed` and then `finished` is called,
/// with no lock held.
pub fn begin(
    phase: &Arc<Mutex<Phase>>,
    bin: &Path,
    app_bundle: &str,
    dir: &Path,
    finished: impl FnOnce() + Send + 'static,
) -> Result<(), String> {
    // Held until the run is in place, so a recorder that exits at once waits to be recorded as one.
    let mut now = lock(phase);
    match &*now {
        Phase::Recording(_) => return Err("already recording".into()),
        Phase::Review(_) => return Err("save or discard the recording under review first".into()),
        Phase::Idle | Phase::Failed(_) => {}
    }
    let shared = Arc::clone(phase);
    let run = start(bin, app_bundle, dir, move |written| {
        let reviewed = written.and_then(|path| {
            Draft::from_file(&path).inspect_err(|_| {
                let _ = std::fs::remove_file(&path); // a refused recording does not linger
            })
        });
        *lock(&shared) = match reviewed {
            Ok(draft) => Phase::Review(draft),
            Err(why) => Phase::Failed(why),
        };
        finished();
    })?;
    *now = Phase::Recording(run);
    Ok(())
}

/// Starts the recorder. `on_exit` is called once, from a thread that blocks until the process
/// exits: the file it wrote, or why there is none. A run that failed leaves no file.
pub fn start(
    bin: &Path,
    app_bundle: &str,
    dir: &Path,
    on_exit: impl FnOnce(Result<PathBuf, String>) + Send + 'static,
) -> Result<Recording, String> {
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
    // The watcher exists before the recorder does: a recorder with nothing watching it would run on.
    let (hand_over, run) = mpsc::channel::<(Arc<Watched>, Child, PathBuf)>();
    std::thread::Builder::new()
        .spawn(move || {
            // No run arrives when the recorder never started.
            if let Ok((watched, child, out)) = run.recv() {
                on_exit(watch(&watched, child, out));
            }
        })
        .map_err(|e| format!("cannot start the recorder: {}", e.kind()))?;
    let child = Command::new(bin)
        .args(["record", "--app", app_bundle, "--out"])
        .arg(&out)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|e| format!("cannot start the recorder: {}", e.kind()))?;
    let watched = Arc::new(Watched {
        pid: child.id() as libc::pid_t,
        end: Mutex::default(),
        ended: Condvar::new(),
    });
    // The watcher is blocked on this channel, so the send has a receiver.
    let _ = hand_over.send((Arc::clone(&watched), child, out));
    Ok(Recording { watched })
}

/// Blocks until the recorder exits, then reaps it and judges what it left.
fn watch(watched: &Watched, mut child: Child, out: PathBuf) -> Result<PathBuf, String> {
    wait_without_reaping(watched.pid);
    let forced = {
        let mut end = lock(&watched.end);
        end.exited = true;
        watched.ended.notify_all();
        end.forced
    };
    let succeeded = child.wait().is_ok_and(|s| s.success());
    if !forced && succeeded && out.exists() {
        return Ok(out);
    }
    let _ = std::fs::remove_file(&out);
    Err(if forced {
        "the recorder did not stop in time and was ended".into()
    } else {
        "the recording was not saved (no steps were recorded, or the recorder failed)".into()
    })
}

/// Returns once the process has exited but before it is reaped, so its pid is still its own.
fn wait_without_reaping(pid: libc::pid_t) {
    loop {
        // SAFETY: `info` is a plain struct that waitid fills in; nothing else is passed to it.
        let mut info: libc::siginfo_t = unsafe { std::mem::zeroed() };
        let r = unsafe {
            libc::waitid(libc::P_PID, pid as libc::id_t, &mut info, libc::WEXITED | libc::WNOWAIT)
        };
        if r != 0 {
            if std::io::Error::last_os_error().kind() == std::io::ErrorKind::Interrupted {
                continue;
            }
            return; // nothing to wait for; the reap that follows says so
        }
        if matches!(info.si_code, libc::CLD_EXITED | libc::CLD_KILLED | libc::CLD_DUMPED) {
            return;
        }
        // macOS also answers here for a paused child, which is still alive. Take that report,
        // or the next wait returns it again at once. No exit is taken: WEXITED is not asked for.
        // SAFETY: as above.
        unsafe {
            libc::waitid(
                libc::P_PID,
                pid as libc::id_t,
                &mut info,
                libc::WSTOPPED | libc::WCONTINUED | libc::WNOHANG,
            )
        };
    }
}

impl Recording {
    pub fn stopper(&self) -> Stopper {
        Stopper(Arc::clone(&self.watched))
    }
}

impl Stopper {
    /// Signals stop (SIGINT, which the executor handles by turning the tap and indicator off and
    /// then writing the file) and returns once it has exited. The result goes to `on_exit`.
    pub fn stop(&self) -> Result<(), String> {
        self.stop_within(STOP_CEILING)
    }

    /// `stop` with its own bound. A recorder still running after it is killed: one that ignores
    /// the stop must not keep its tap and indicator on.
    pub fn stop_within(&self, ceiling: Duration) -> Result<(), String> {
        let w = &*self.0;
        let mut end = lock(&w.end);
        if end.exited {
            return Ok(());
        }
        // SAFETY (both kills): not yet reaped while `exited` is false under this lock.
        if unsafe { libc::kill(w.pid, libc::SIGINT) } != 0 {
            end.forced = true;
            unsafe { libc::kill(w.pid, libc::SIGKILL) };
            return Err("could not tell the recorder to stop".into());
        }
        let (mut end, _) = w
            .ended
            .wait_timeout_while(end, ceiling, |e| !e.exited)
            .unwrap_or_else(|e| e.into_inner());
        if !end.exited {
            end.forced = true;
            unsafe { libc::kill(w.pid, libc::SIGKILL) };
        }
        Ok(())
    }
}
