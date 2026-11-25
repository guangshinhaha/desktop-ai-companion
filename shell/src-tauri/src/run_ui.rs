//! The progress window and the commands behind its Pause and Stop (PRD C-3, C-4). The logic is in
//! `progress.rs`, which has no window and is tested headless; this file only wires it to Tauri.
//!
//! The window is created when a replay starts and stays after it ends, to show the report, until the
//! officer closes it. Nothing runs while no replay is on. While one is, the stop shortcut is registered
//! system-wide (the replayed app has the focus, not this window) and dropped when the run ends.

use crate::progress::{ActiveRun, Progress, RunHandle};
use crate::Session;
use serde_json::Value;
use tauri::{AppHandle, Emitter, Manager, WebviewUrl, WebviewWindowBuilder};
use tauri_plugin_global_shortcut::{Code, GlobalShortcutExt, Modifiers, Shortcut};

pub const PROGRESS: &str = "progress";

/// Cmd+Shift+Period. Registered only while a replay runs.
pub fn stop_shortcut() -> Shortcut {
    Shortcut::new(Some(Modifiers::SUPER | Modifiers::SHIFT), Code::Period)
}

/// The run slot, even if a run panicked while holding it: a poisoned lock must not leave the slot stuck.
fn with_slot<T>(app: &AppHandle, f: impl FnOnce(&mut Option<ActiveRun>) -> T) -> T {
    let session = app.state::<Session>();
    let mut g = session.run.lock().unwrap_or_else(|p| p.into_inner());
    f(&mut g)
}

fn view_of(app: &AppHandle) -> Option<Value> {
    with_slot(app, |s| s.as_ref().map(|r| r.progress.view()))
}

/// Whether a replay is on now.
pub fn is_live(app: &AppHandle) -> bool {
    with_slot(app, |s| s.as_ref().is_some_and(|r| r.is_live()))
}

/// Whether the officer has asked to stop the run on screen.
pub fn stop_asked(app: &AppHandle) -> bool {
    with_slot(app, |s| s.as_ref().is_some_and(|r| r.stop_requested()))
}

fn publish(app: &AppHandle) {
    if let Some(v) = view_of(app) {
        let _ = app.emit_to(PROGRESS, "progress", v);
    }
}

fn open_window(app: &AppHandle) {
    if app.get_webview_window(PROGRESS).is_some() {
        return;
    }
    // Not focused: the officer's attention stays where it is. Always on top, so it is never lost behind the app.
    let _ = WebviewWindowBuilder::new(app, PROGRESS, WebviewUrl::App("progress.html".into()))
        .title("Replay progress")
        .inner_size(400.0, 560.0)
        .always_on_top(true)
        .focused(false)
        .build();
}

/// Starts tracking a run. Refuses if one is already on: one replay at a time.
/// The returned guard ends the run if dropped without `finish`, so a panic cannot leave the slot taken
/// or the shortcut registered.
pub fn begin(app: &AppHandle, mut progress: Progress) -> Result<RunGuard, String> {
    // Failing to register (another app holds it) leaves the window's Stop button and Escape; the view says so.
    let registered = app.global_shortcut().register(stop_shortcut()).is_ok();
    progress.set_shortcut(registered);
    let taken = with_slot(app, |s| {
        if s.as_ref().is_some_and(|r| r.is_live()) {
            return true;
        }
        *s = Some(ActiveRun::new(progress));
        false
    });
    if taken {
        // Not ours: leave the first run's shortcut as it is.
        return Err("a replay is already running".into());
    }
    open_window(app);
    publish(app);
    let handle = app.clone();
    Ok(RunGuard(Some(crate::progress::OnDrop::new(Box::new(move || end(&handle, &Err("the run ended unexpectedly".into())))))))
}

pub struct RunGuard(Option<crate::progress::OnDrop<Box<dyn FnOnce() + Send>>>);

impl RunGuard {
    /// The run is over with this result. Disarms the guard.
    pub fn finish(mut self, app: &AppHandle, result: &Result<Value, String>) {
        if let Some(g) = self.0.as_mut() {
            g.disarm();
        }
        end(app, result);
    }
}

/// The executor is running: hand its controls to the run.
pub fn attach(app: &AppHandle, handle: RunHandle) {
    with_slot(app, |s| {
        if let Some(r) = s.as_mut() {
            r.attach(handle);
        }
    });
}

pub fn on_progress(app: &AppHandle, params: &Value) {
    with_slot(app, |s| {
        if let Some(r) = s.as_mut() {
            r.progress.apply(params);
        }
    });
    publish(app);
}

/// The run is over: show its report, and let go of the stop shortcut and the executor's controls.
pub fn end(app: &AppHandle, result: &Result<Value, String>) {
    with_slot(app, |s| {
        if let Some(r) = s.as_mut() {
            r.progress.finish(result);
            r.handle = None;
        }
    });
    let _ = app.global_shortcut().unregister(stop_shortcut());
    publish(app);
}

fn with_run(app: &AppHandle, f: impl FnOnce(&mut ActiveRun)) {
    with_slot(app, |s| {
        if let Some(r) = s.as_mut() {
            f(r);
        }
    });
    publish(app);
}

/// Stops the run on screen, if any. Also the global shortcut's action.
pub fn stop_active(app: &AppHandle) {
    with_run(app, |r| r.stop());
    // A question open on the officer's screen is answered no, so the run is not left waiting on it.
    crate::answer_approval(app, None, false);
    for (label, w) in app.webview_windows() {
        if label.starts_with("approval-") {
            let _ = w.close();
        }
    }
}

#[tauri::command]
pub fn progress_current(app: AppHandle) -> Option<Value> {
    view_of(&app)
}

#[tauri::command]
pub fn run_pause(app: AppHandle) {
    with_run(&app, |r| r.pause());
}

#[tauri::command]
pub fn run_resume(app: AppHandle) {
    with_run(&app, |r| r.resume());
}

#[tauri::command]
pub fn run_stop(app: AppHandle) {
    stop_active(&app);
}
