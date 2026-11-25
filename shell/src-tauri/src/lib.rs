//! Menu-bar shell. One tray icon, one canvas window created on demand and destroyed on close,
//! so an idle shell holds no web view. The only network call is chat to the companion service
//! (`chat.rs`), over https or to this machine.

pub mod approval;
pub mod chat;
pub mod executor;
pub mod extract;
pub mod progress;
pub mod recording;
pub mod review;
pub mod store;
pub mod usage;

use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use std::sync::Mutex;
use tauri::{AppHandle, Manager, RunEvent, WebviewUrl, WebviewWindowBuilder, WindowEvent};
use tauri_plugin_global_shortcut::{Code, Modifiers, Shortcut, ShortcutState};

const CANVAS: &str = "canvas";

fn open_canvas(app: &AppHandle) {
    if let Some(w) = app.get_webview_window(CANVAS) {
        let _ = w.set_focus();
        return;
    }
    let built = WebviewWindowBuilder::new(app, CANVAS, WebviewUrl::App("index.html".into()))
        .title("Companion")
        .inner_size(560.0, 76.0)
        .decorations(false)
        .resizable(false)
        .always_on_top(true)
        .skip_taskbar(true)
        .center()
        .build();
    if let Ok(w) = built {
        // Closing on blur keeps the shell dormant: click away and the web view is gone.
        let handle = w.clone();
        w.on_window_event(move |e| {
            if let WindowEvent::Focused(false) = e {
                let _ = handle.close();
            }
        });
    }
}

fn toggle_canvas(app: &AppHandle) {
    match app.get_webview_window(CANVAS) {
        Some(w) => {
            let _ = w.close();
        }
        None => open_canvas(app),
    }
}

#[tauri::command]
fn close_canvas(app: AppHandle) {
    if let Some(w) = app.get_webview_window(CANVAS) {
        let _ = w.close();
    }
}

/// Chat turn (#31). Chunks stream to the canvas over `on_event`; the full reply is not returned.
/// The error is plain words for the officer, with a kind the canvas can branch on.
#[tauri::command]
async fn chat_send(
    classification: String,
    messages: serde_json::Value,
    on_event: tauri::ipc::Channel<serde_json::Value>,
) -> Result<(), serde_json::Value> {
    tauri::async_runtime::spawn_blocking(move || {
        let now = now_secs();
        let service = chat::HttpService::from_env().map_err(|e| e.to_json())?;
        let store = open_store().map_err(|_| chat::SendError::NotSaved.to_json())?;
        let mut on_text = |t: &str| {
            let _ = on_event.send(serde_json::json!({ "text": t }));
        };
        chat::chat_turn(&service, &store, &classification, &messages, now, &mut on_text).map_err(|e| e.to_json())?;
        if let Some(path) = usage::default_path() {
            if let Ok(log) = usage::UsageLog::open(&path) {
                let _ = log.record(usage::Event::ChatRequest, 1, now);
            }
        }
        Ok(())
    })
    .await
    .map_err(|_| chat::SendError::ReplyFailed.to_json())?
}

/// Reads a dropped file's text on this machine. The text goes back to the canvas, nothing else.
#[tauri::command]
async fn chat_attach(path: String) -> Result<serde_json::Value, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let p = std::path::PathBuf::from(&path);
        let text = extract::extract_text(&p).map_err(|e| e.plain().to_string())?;
        let name = p.file_name().and_then(|n| n.to_str()).unwrap_or("document").to_string();
        Ok(serde_json::json!({ "name": name, "text": text }))
    })
    .await
    .map_err(|_| extract::ExtractError::Unreadable.plain().to_string())?
}

/// The canvas starts as one input; the first chat message opens it up to show the conversation.
#[tauri::command]
fn expand_canvas(app: AppHandle) {
    if let Some(w) = app.get_webview_window(CANVAS) {
        let _ = w.set_size(tauri::LogicalSize::new(560.0, 520.0));
    }
}

/// Starts the executor on demand, pings it, returns its version, and lets it exit.
#[tauri::command]
async fn ping_executor() -> Result<String, String> {
    tauri::async_runtime::spawn_blocking(|| executor::ping(&executor::executor_path()))
        .await
        .map_err(|e| e.to_string())?
}


const REVIEW: &str = "review";

/// The one draft under review and the recorder run in progress. Memory only.
#[derive(Default)]
struct Session {
    draft: Mutex<Option<review::Draft>>,
    recording: Mutex<Option<recording::Recording>>,
    approval: Mutex<Option<PendingApproval>>,
}

/// The card on screen and where the officer's answer goes. One at a time: a run asks one question at a time.
struct PendingApproval {
    card: serde_json::Value,
    answer: std::sync::mpsc::Sender<bool>,
    /// The window asking. A window closing late must not answer for the next card.
    label: String,
}

fn open_approval_window(app: &AppHandle, label: &str) {
    let built = WebviewWindowBuilder::new(app, label, WebviewUrl::App("approval.html".into()))
        .title("Approval needed")
        .inner_size(620.0, 640.0)
        .always_on_top(true)
        .center()
        .build();
    let Ok(w) = built else {
        // No card could be shown, so the question is a no and the run stops rather than hangs.
        answer_approval(app, Some(label), false);
        return;
    };
    {
        // Closing the window without answering is a no. The question was never approved.
        let handle = app.clone();
        let mine = label.to_string();
        w.on_window_event(move |e| {
            if let WindowEvent::Destroyed = e {
                answer_approval(&handle, Some(&mine), false);
            }
        });
    }
}

/// Delivers the officer's decision to the waiting run. A second answer finds nothing waiting.
fn answer_approval(app: &AppHandle, from: Option<&str>, approved: bool) {
    let pending = app.state::<Session>().approval.lock().ok().and_then(|mut p| {
        if from.is_some_and(|l| p.as_ref().is_some_and(|q| q.label != l)) {
            return None;
        }
        p.take()
    });
    if let Some(p) = pending {
        let _ = p.answer.send(approved);
    }
}

/// The card the officer is being asked about, or nothing.
#[tauri::command]
fn approval_current(session: tauri::State<Session>, window: tauri::WebviewWindow) -> Option<serde_json::Value> {
    session
        .approval
        .lock()
        .ok()
        .and_then(|p| p.as_ref().filter(|p| p.label == window.label()).map(|p| p.card.clone()))
}

/// Only an explicit approve passes `true`. The window closes either way.
#[tauri::command]
fn approval_answer(app: AppHandle, window: tauri::WebviewWindow, approved: bool) {
    // The answer counts only from the window that holds the pending card.
    answer_approval(&app, Some(window.label()), approved);
    let _ = window.close();
}

/// Runs a saved task through the executor. The officer is asked before every step that may commit; every
/// decision and the run's start and end are appended to the audit chain. Inputs stay in memory.
/// Returns the executor's report: outcome, completed and not-run step ids.
#[tauri::command]
async fn replay_start(
    app: AppHandle,
    task_id: String,
    version: i64,
    inputs: serde_json::Value,
    officer: String,
    pid: Option<i64>,
) -> Result<serde_json::Value, String> {
    if !approval::valid_officer(&officer) {
        return Err("the officer's name is needed".into());
    }
    tauri::async_runtime::spawn_blocking(move || {
        let mut store = open_store()?;
        let body = store
            .task_body(&task_id, version)
            .map_err(|e| e.to_string())?
            .ok_or("no such task version")?;
        let task: serde_json::Value = serde_json::from_str(&body).map_err(|_| "the saved task is not readable")?;
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        let run = format!("run_{nanos:x}");
        store.start_run(&run, &task_id, version, "shell", now_secs()).map_err(|e| e.to_string())?;
        let ctx = approval::RunContext { officer, task_ref: format!("{task_id}@{version}"), run: run.clone() };
        let result = approval::run_replay(&executor::executor_path(), &mut store, &ctx, &task, &inputs, pid, &mut |card| {
            let (tx, rx) = std::sync::mpsc::channel();
            let label = format!("approval-{:x}", std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0));
            if let Ok(mut p) = app.state::<Session>().approval.lock() {
                *p = Some(PendingApproval { card: card.clone(), answer: tx, label: label.clone() });
            }
            open_approval_window(&app, &label);
            // No timeout: waiting is not consent. A closed window or a dropped sender is a no.
            rx.recv().unwrap_or(false)
        });
        let status = match &result {
            Ok(r) if r["outcome"] == "finished" => "finished",
            Ok(r) if r["outcome"] == "refused" => "refused",
            _ => "stopped",
        };
        let _ = store.finish_run(&run, status);
        result
    })
    .await
    .map_err(|e| e.to_string())?
}

/// Verifies the whole audit trail with `executor-macos verify-audit`. Ok says the trail is intact;
/// Err names the sequence number where it breaks.
#[tauri::command]
async fn audit_verify() -> Result<String, String> {
    tauri::async_runtime::spawn_blocking(|| {
        let export = open_store()?.export_audit().map_err(|e| e.to_string())?;
        executor::verify_audit(&executor::executor_path(), &export)
    })
    .await
    .map_err(|e| e.to_string())?
}

fn open_review_window(app: &AppHandle) {
    if let Some(w) = app.get_webview_window(REVIEW) {
        let _ = w.set_focus();
        return;
    }
    let _ = WebviewWindowBuilder::new(app, REVIEW, WebviewUrl::App("review.html".into()))
        .title("Review recording")
        .inner_size(760.0, 680.0)
        .center()
        .build();
}

fn close_review_window(app: &AppHandle) {
    if let Some(w) = app.get_webview_window(REVIEW) {
        let _ = w.close();
    }
}

fn now_secs() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

fn open_store() -> Result<store::Store, String> {
    let path = store::default_path().ok_or("no home folder")?;
    store::Store::open(&path, &store::Keychain::app_default()).map_err(|e| e.to_string())
}

#[tauri::command]
fn record_start(session: tauri::State<Session>, app_bundle: String) -> Result<(), String> {
    let mut slot = session.recording.lock().map_err(|_| "busy")?;
    if slot.is_some() {
        return Err("already recording".into());
    }
    let dir = recording::drafts_dir().ok_or("no home folder")?;
    *slot = Some(recording::start(&executor::executor_path(), &app_bundle, &dir)?);
    Ok(())
}

/// Stop: the executor turns the tap off and writes the file; the review opens on it.
#[tauri::command]
async fn record_stop(app: AppHandle) -> Result<(), String> {
    let rec = app
        .state::<Session>()
        .recording
        .lock()
        .map_err(|_| "busy")?
        .take()
        .ok_or("not recording")?;
    let path = tauri::async_runtime::spawn_blocking(move || recording::stop(rec))
        .await
        .map_err(|e| e.to_string())??;
    let draft = match review::Draft::from_file(&path) {
        Ok(d) => d,
        Err(e) => {
            let _ = std::fs::remove_file(&path); // a refused recording does not linger
            return Err(e);
        }
    };
    *app.state::<Session>().draft.lock().map_err(|_| "busy")? = Some(draft);
    open_review_window(&app);
    Ok(())
}

#[tauri::command]
fn review_view(session: tauri::State<Session>) -> Result<serde_json::Value, String> {
    let d = session.draft.lock().map_err(|_| "busy")?;
    d.as_ref().map(|d| d.view()).ok_or_else(|| "nothing to review".into())
}

#[tauri::command]
fn review_edit(session: tauri::State<Session>, edit: serde_json::Value) -> Result<serde_json::Value, String> {
    let mut d = session.draft.lock().map_err(|_| "busy")?;
    let draft = d.as_mut().ok_or("nothing to review")?;
    draft.apply(&edit)?;
    Ok(draft.view())
}

#[tauri::command]
fn review_save(app: AppHandle, author: String) -> Result<serde_json::Value, String> {
    let session = app.state::<Session>();
    let mut slot = session.draft.lock().map_err(|_| "busy")?;
    let draft = slot.take().ok_or("nothing to review")?;
    // A failed save must not lose the officer's edits: the draft goes back.
    let kept = draft.clone_for_retry();
    let saved = open_store().and_then(|mut store| draft.save(&mut store, &author, now_secs()));
    match saved {
        Ok((id, version)) => {
            drop(slot);
            close_review_window(&app);
            Ok(serde_json::json!({ "id": id, "version": version }))
        }
        Err(e) => {
            *slot = Some(kept);
            Err(e)
        }
    }
}

#[tauri::command]
fn review_discard(app: AppHandle) {
    if let Ok(mut slot) = app.state::<Session>().draft.lock() {
        if let Some(d) = slot.take() {
            d.discard();
        }
    }
    close_review_window(&app);
}

pub fn run() {
    let shortcut = Shortcut::new(Some(Modifiers::SUPER | Modifiers::SHIFT), Code::Space);
    let app = tauri::Builder::default()
        .manage(Session::default())
        .plugin(
            tauri_plugin_global_shortcut::Builder::new()
                .with_shortcuts([shortcut])
                .expect("register shortcut")
                .with_handler(|app, _s, event| {
                    if event.state == ShortcutState::Pressed {
                        toggle_canvas(app);
                    }
                })
                .build(),
        )
        .invoke_handler(tauri::generate_handler![
            close_canvas,
            chat_send,
            chat_attach,
            expand_canvas,
            ping_executor,
            record_start,
            record_stop,
            review_view,
            review_edit,
            review_save,
            review_discard,
            approval_current,
            approval_answer,
            replay_start,
            audit_verify
        ])
        .setup(|app| {
            #[cfg(target_os = "macos")]
            app.set_activation_policy(tauri::ActivationPolicy::Accessory);

            let quit = tauri::menu::MenuItem::with_id(app, "quit", "Quit Companion", true, None::<&str>)?;
            let menu = tauri::menu::Menu::with_items(app, &[&quit])?;
            TrayIconBuilder::new()
                .icon(tauri::image::Image::from_bytes(include_bytes!("../icons/icon.png"))?)
                .icon_as_template(true)
                .tooltip("Companion")
                .menu(&menu)
                .show_menu_on_left_click(false)
                .on_menu_event(|app, e| {
                    if e.id() == "quit" {
                        app.exit(0);
                    }
                })
                .on_tray_icon_event(|tray, e| {
                    if let TrayIconEvent::Click {
                        button: MouseButton::Left,
                        button_state: MouseButtonState::Up,
                        ..
                    } = e
                    {
                        toggle_canvas(tray.app_handle());
                    }
                })
                .build(app)?;
            Ok(())
        })
        .build(tauri::generate_context!())
        .expect("build app");

    app.run(|_app, event| {
        // The canvas closing must not end the app: it lives in the menu bar.
        if let RunEvent::ExitRequested { api, code, .. } = event {
            if code.is_none() {
                api.prevent_exit();
            }
        }
    });
}
