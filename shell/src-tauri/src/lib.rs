//! Menu-bar shell. One tray icon, one canvas window created on demand and destroyed on close,
//! so an idle shell holds no web view. The only network call is chat to the companion service
//! (`chat.rs`), over https or to this machine.

pub mod approval;
pub mod chat;
pub mod csv;
pub mod executor;
pub mod extract;
pub mod listrun;
pub mod progress;
pub mod recording;
pub mod notes;
pub mod review;
pub mod router;
mod run_ui;
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
    /// The replay on screen, or the last one's report.
    run: Mutex<Option<progress::ActiveRun>>,
    /// The one list run going now, if any.
    listrun: Mutex<Option<ActiveListRun>>,
    /// What went wrong with a list run, for the screen: (run, why).
    listrun_error: Mutex<Option<(String, String)>>,
}

/// A list run going on a background thread. The stop flag is read between rows.
struct ActiveListRun {
    run: String,
    stop: std::sync::Arc<std::sync::atomic::AtomicBool>,
    /// Set when the officer asks to end the run: the thread records the end when it finishes its row.
    end_by: std::sync::Arc<Mutex<Option<String>>>,
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

/// Shows the approval card and waits for the officer. No timeout: waiting is not consent. A closed window or
/// a dropped sender is a no.
fn ask_officer(app: &AppHandle, card: &serde_json::Value) -> bool {
    if run_ui::stop_asked(app) {
        return false;
    }
    let (tx, rx) = std::sync::mpsc::channel();
    let label = format!(
        "approval-{:x}",
        std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0)
    );
    if let Ok(mut p) = app.state::<Session>().approval.lock() {
        *p = Some(PendingApproval { card: card.clone(), answer: tx, label: label.clone() });
    }
    // A card is never put up for a run the officer has stopped; one stopped while it is up is a no.
    if run_ui::stop_asked(app) {
        answer_approval(app, Some(&label), false);
    } else {
        open_approval_window(app, &label);
    }
    rx.recv().unwrap_or(false)
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
        // The list run and a single replay share the approval card and the progress window: one at a time.
        if app.state::<Session>().listrun.lock().map_err(|_| "busy")?.is_some() {
            return Err("a list run is going; wait for it to end".to_string());
        }
        let mut store = open_store()?;
        let body = store
            .task_body(&task_id, version)
            .map_err(|e| e.to_string())?
            .ok_or("no such task version")?;
        let task: serde_json::Value = serde_json::from_str(&body).map_err(|_| "the saved task is not readable")?;
        // Never start with an input missing: nothing is defaulted or guessed (#32). Names only in the error.
        let missing = router::missing_inputs(&task, &inputs);
        if !missing.is_empty() {
            return Err(format!("missing input: {}", missing.join(", ")));
        }
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        let run = format!("run_{nanos:x}");
        // The progress view, its Pause and Stop, and the stop shortcut, for as long as the run lasts.
        let guard = run_ui::begin(&app, progress::Progress::new(&run, &task))?;
        if let Err(e) = store.start_run(&run, &task_id, version, "shell", now_secs()) {
            let e = e.to_string();
            guard.finish(&app, &Err(e.clone()));
            return Err(e);
        }
        let ctx = approval::RunContext { officer, task_ref: format!("{task_id}@{version}"), run: run.clone(), item: None };
        let mut on_start = |h| run_ui::attach(&app, h);
        let mut on_progress = |p: &serde_json::Value| run_ui::on_progress(&app, p);
        let mut hooks = approval::Hooks { on_start: &mut on_start, on_progress: &mut on_progress, stop_grace: progress::STOP_GRACE };
        let result = approval::run_replay_with(&executor::executor_path(), &mut store, &ctx, &task, &inputs, pid, &mut |card| {
            // A card is never put up for a run the officer has stopped.
            if run_ui::stop_asked(&app) {
                return false;
            }
            let (tx, rx) = std::sync::mpsc::channel();
            let label = format!("approval-{:x}", std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0));
            if let Ok(mut p) = app.state::<Session>().approval.lock() {
                *p = Some(PendingApproval { card: card.clone(), answer: tx, label: label.clone() });
            }
            // Checked again now the card is pending: a stop from here on answers it, and one just before is seen here.
            if run_ui::stop_asked(&app) {
                answer_approval(&app, Some(&label), false);
            } else {
                open_approval_window(&app, &label);
            }
            // No timeout: waiting is not consent. A closed window or a dropped sender is a no.
            rx.recv().unwrap_or(false)
        }, &mut hooks);
        guard.finish(&app, &result);
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

/// The saved tasks and the inputs each needs from the officer, for routing a request in the canvas (#32).
#[tauri::command]
async fn task_catalog() -> Result<Vec<serde_json::Value>, String> {
    tauri::async_runtime::spawn_blocking(|| router::catalog(&open_store()?))
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

const TASK: &str = "task";

fn open_task_window(app: &AppHandle) {
    if let Some(w) = app.get_webview_window(TASK) {
        let _ = w.set_focus();
        return;
    }
    let _ = WebviewWindowBuilder::new(app, TASK, WebviewUrl::App("task.html".into()))
        .title("Task notes")
        .inner_size(760.0, 680.0)
        .center()
        .build();
}

/// A saved task's steps and the notes beside them (#44). Notes may hold personal data: they go to
/// this window only, and nothing here logs them.
#[tauri::command]
async fn task_detail(task_id: String) -> Result<serde_json::Value, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let store = open_store()?;
        notes::detail(&store, &notes::LocalNotes(&store), &task_id)
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
async fn task_note_add(task_id: String, author: String, body: String) -> Result<serde_json::Value, String> {
    tauri::async_runtime::spawn_blocking(move || {
        use notes::NoteBackend;
        let store = open_store()?;
        let backend = notes::LocalNotes(&store);
        backend.add(&task_id, &author, &body, now_secs())?;
        notes::detail(&store, &backend, &task_id)
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
async fn task_note_remove(task_id: String, note_id: i64) -> Result<serde_json::Value, String> {
    tauri::async_runtime::spawn_blocking(move || {
        use notes::NoteBackend;
        let store = open_store()?;
        let backend = notes::LocalNotes(&store);
        backend.remove(&task_id, note_id)?;
        notes::detail(&store, &backend, &task_id)
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

// MARK: list runs (#33)

const LISTRUN: &str = "listrun";

fn open_listrun_window(app: &AppHandle) {
    if let Some(w) = app.get_webview_window(LISTRUN) {
        let _ = w.set_focus();
        return;
    }
    let _ = WebviewWindowBuilder::new(app, LISTRUN, WebviewUrl::App("listrun.html".into()))
        .title("Run a task over a spreadsheet")
        .inner_size(760.0, 700.0)
        .center()
        .build();
}

fn saved_task(store: &store::Store, task_id: &str, version: i64) -> Result<serde_json::Value, String> {
    let body = store.task_body(task_id, version).map_err(|e| e.to_string())?.ok_or("no such task version")?;
    serde_json::from_str(&body).map_err(|_| "the saved task is not readable".to_string())
}

/// Reads the file on this machine and says what it holds: column names and a row count, never a cell.
/// Until the officer says the first row is a header, no cell is shown as a name: the columns are numbered.
#[tauri::command]
async fn listrun_preview(path: String, task_id: String, version: i64, header: bool) -> Result<serde_json::Value, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let store = open_store()?;
        let task = saved_task(&store, &task_id, version)?;
        let inputs = csv::task_inputs(&task);
        let (file, _) = csv::read_file(std::path::Path::new(&path), header)?;
        let names: Vec<&String> = inputs.iter().map(|i| &i.name).collect();
        // A step that is not marked "no" is asked about. None means every row runs with no card.
        let asked = task["steps"].as_array().map_or(0, |s| s.iter().filter(|x| x["consequential"] != "no").count());
        Ok(serde_json::json!({
            "columns": file.header,
            "rows": file.rows.len(),
            "inputs": names,
            "suggested": if header { csv::suggest_mapping(&file.header, &inputs) } else { Default::default() },
            "approval_steps": asked,
        }))
    })
    .await
    .map_err(|e| e.to_string())?
}

/// The slot that says a list run is going. Held from the moment the run is claimed until its thread ends,
/// however it ends. While held, a single replay is refused (they share the approval card).
struct ActiveGuard(AppHandle);
impl Drop for ActiveGuard {
    fn drop(&mut self) {
        if let Ok(mut a) = self.0.state::<Session>().listrun.lock() {
            *a = None;
        }
    }
}

fn claim_listrun(app: &AppHandle, run: &str) -> Result<(ActiveGuard, std::sync::Arc<std::sync::atomic::AtomicBool>, std::sync::Arc<Mutex<Option<String>>>), String> {
    use std::sync::{atomic::AtomicBool, Arc};
    if run_ui::is_live(app) {
        return Err("a replay is running; wait for it to end".into());
    }
    let stop = Arc::new(AtomicBool::new(false));
    let end_by = Arc::new(Mutex::new(None));
    let session = app.state::<Session>();
    let mut slot = session.listrun.lock().map_err(|_| "busy")?;
    if slot.is_some() {
        return Err("a list run is already going".into());
    }
    *slot = Some(ActiveListRun { run: run.to_string(), stop: stop.clone(), end_by: end_by.clone() });
    drop(slot);
    // A new go at a run clears what went wrong the last time.
    if let Ok(mut e) = session.listrun_error.lock() {
        *e = None;
    }
    Ok((ActiveGuard(app.clone()), stop, end_by))
}

fn set_listrun_error(app: &AppHandle, run: &str, why: String) {
    if let Ok(mut e) = app.state::<Session>().listrun_error.lock() {
        *e = Some((run.to_string(), why));
    }
}

/// One row through the executor, with the progress window and Stop (#28) for that row.
#[allow(clippy::too_many_arguments)]
fn replay_row(
    app: &AppHandle,
    store: &mut store::Store,
    ctx: &approval::RunContext,
    task: &serde_json::Value,
    inputs: &serde_json::Value,
    pid: Option<i64>,
    stop: &std::sync::atomic::AtomicBool,
    obs: &mut dyn approval::ItemObserver,
) -> Result<serde_json::Value, String> {
    let label = format!("{}, row {}", ctx.run, ctx.item.unwrap_or(0));
    let guard = run_ui::begin(app, progress::Progress::new(&label, task))?;
    let mut on_start = |h| run_ui::attach(app, h);
    let mut on_progress = |p: &serde_json::Value| run_ui::on_progress(app, p);
    let mut hooks = approval::Hooks { on_start: &mut on_start, on_progress: &mut on_progress, stop_grace: progress::STOP_GRACE };
    let result = approval::run_replay_observed(&executor::executor_path(), store, ctx, task, inputs, pid, &mut |card| ask_officer(app, card), &mut hooks, obs);
    // The officer pressed Stop for this row: no more rows after it.
    if run_ui::stop_asked(app) {
        stop.store(true, std::sync::atomic::Ordering::SeqCst);
    }
    guard.finish(app, &result);
    result
}

/// Runs the rows on a thread. The officer sees progress through `listrun_status`; anything that goes wrong is
/// kept for the screen, not dropped.
#[allow(clippy::too_many_arguments)]
fn spawn_listrun(
    app: AppHandle,
    guard: ActiveGuard,
    stop: std::sync::Arc<std::sync::atomic::AtomicBool>,
    end_by: std::sync::Arc<Mutex<Option<String>>>,
    run: String,
    task_id: String,
    version: i64,
    rows: Vec<serde_json::Value>,
    from: Option<i64>,
    officer: String,
    pid: Option<i64>,
) {
    use std::sync::atomic::Ordering;
    std::thread::spawn(move || {
        let _guard = guard;
        let outcome = (|| -> Result<(), String> {
            let mut store = open_store()?;
            let task = saved_task(&store, &task_id, version)?;
            let task_ref = format!("{task_id}@{version}");
            let stopped = || stop.load(Ordering::SeqCst);
            let params = listrun::RunParams { run: &run, task_ref: &task_ref, officer: &officer, rows: &rows, from, stop: &stopped };
            listrun::run_rows(&mut store, &params, &mut |store, ctx, inputs, obs| replay_row(&app, store, ctx, &task, inputs, pid, &stop, obs))?;
            Ok(())
        })();
        if let Err(e) = outcome {
            set_listrun_error(&app, &run, e);
        }
        if let Some(by) = end_by.lock().ok().and_then(|mut b| b.take()) {
            let ended = open_store().and_then(|mut s| listrun::end_run(&mut s, &run, &by, now_secs()));
            if let Err(e) = ended {
                set_listrun_error(&app, &run, format!("the run could not be ended: {e}"));
            }
        }
    });
}

fn run_id() -> String {
    let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0);
    format!("run_{nanos:x}")
}

/// Starts a run over the file, or, if an unfinished run for this task and file exists (after a crash or a
/// restart), picks that one up: rows done are not asked again and a row that was mid-commit shows as unknown.
#[tauri::command]
#[allow(clippy::too_many_arguments)]
async fn listrun_start(
    app: AppHandle,
    path: String,
    task_id: String,
    version: i64,
    header: bool,
    mapping: std::collections::BTreeMap<String, String>,
    officer: String,
    pid: Option<i64>,
) -> Result<String, String> {
    if !approval::valid_officer(&officer) {
        return Err("the officer's name is needed".into());
    }
    tauri::async_runtime::spawn_blocking(move || {
        // The slot first: a start that cannot go leaves no run behind.
        let new_run = run_id();
        let (guard, stop, end_by) = claim_listrun(&app, &new_run)?;
        let mut store = open_store()?;
        let inputs = csv::task_inputs(&saved_task(&store, &task_id, version)?);
        let loaded = listrun::start_or_resume(&mut store, std::path::Path::new(&path), &task_id, version, &inputs, header, &mapping, &new_run, now_secs())?;
        if let Ok(mut s) = app.state::<Session>().listrun.lock() {
            if let Some(a) = s.as_mut() {
                a.run = loaded.run.clone();
            }
        }
        drop(store);
        let run = loaded.run.clone();
        spawn_listrun(app, guard, stop, end_by, loaded.run, task_id, version, loaded.rows, None, officer, pid);
        Ok(run)
    })
    .await
    .map_err(|e| e.to_string())?
}

/// Continues a stopped run. The same file is needed again: values are never stored. `from` is the row to
/// start at; a diverged or rejected row there is asked again, and that is recorded as the officer's act.
#[tauri::command]
async fn listrun_resume(
    app: AppHandle,
    run: String,
    path: String,
    from: Option<i64>,
    officer: String,
    pid: Option<i64>,
) -> Result<(), String> {
    if !approval::valid_officer(&officer) {
        return Err("the officer's name is needed".into());
    }
    tauri::async_runtime::spawn_blocking(move || {
        let (guard, stop, end_by) = claim_listrun(&app, &run)?;
        let store = open_store()?;
        let (task_id, version, _, _) = store.run_info(&run).map_err(|e| e.to_string())?.ok_or("no such run")?;
        let inputs = csv::task_inputs(&saved_task(&store, &task_id, version)?);
        let rows = listrun::rows_for_run(&store, &run, std::path::Path::new(&path), &inputs)?;
        drop(store);
        spawn_listrun(app, guard, stop, end_by, run, task_id, version, rows, from, officer, pid);
        Ok(())
    })
    .await
    .map_err(|e| e.to_string())?
}

/// A run's rows and what went wrong with it, if anything. A run nobody holds is first made safe to read:
/// a row that died mid-commit shows as unknown.
#[tauri::command]
fn listrun_status(app: AppHandle, run: String) -> Result<serde_json::Value, String> {
    let session = app.state::<Session>();
    let active = session.listrun.lock().map_err(|_| "busy")?.as_ref().is_some_and(|a| a.run == run);
    let mut store = open_store()?;
    if !active {
        listrun::settle_orphans(&mut store, &run)?;
    }
    let mut v = listrun::status(&store, &run)?;
    v["active"] = serde_json::json!(active);
    let err = session.listrun_error.lock().map_err(|_| "busy")?.clone();
    if let Some((r, why)) = err {
        if r == run {
            v["error"] = serde_json::json!(why);
        }
    }
    Ok(v)
}

/// Unfinished runs, so a restart can find them.
#[tauri::command]
fn listrun_unfinished() -> Result<serde_json::Value, String> {
    listrun::unfinished(&open_store()?)
}

/// The officer's answer for a stopped row: `mark_done`, `mark_not_done`, `skip`, `fixed_by_hand` or `retry`.
#[tauri::command]
fn listrun_answer(session: tauri::State<Session>, run: String, item: i64, answer: String, officer: String) -> Result<(), String> {
    if session.listrun.lock().map_err(|_| "busy")?.is_some() {
        return Err("the run is still going".into());
    }
    let a = match answer.as_str() {
        "mark_done" => listrun::Answer::MarkDone,
        "mark_not_done" => listrun::Answer::MarkNotDone,
        "skip" => listrun::Answer::Skip,
        "fixed_by_hand" => listrun::Answer::FixedByHand,
        "retry" => listrun::Answer::Retry,
        _ => return Err("not an answer".into()),
    };
    let mut store = open_store()?;
    listrun::settle_orphans(&mut store, &run)?;
    listrun::officer_answer(&mut store, &run, item, a, &officer, now_secs())
}

/// Ends the run. If it is going, it stops after the row it is on and the end is recorded then; if that
/// fails, the screen is told.
#[tauri::command]
fn listrun_end(session: tauri::State<Session>, run: String, officer: String) -> Result<(), String> {
    if !approval::valid_officer(&officer) {
        return Err("the officer's name is needed".into());
    }
    let active = session.listrun.lock().map_err(|_| "busy")?;
    if let Some(a) = active.as_ref().filter(|a| a.run == run) {
        *a.end_by.lock().map_err(|_| "busy")? = Some(officer);
        a.stop.store(true, std::sync::atomic::Ordering::SeqCst);
        return Ok(());
    }
    drop(active);
    listrun::end_run(&mut open_store()?, &run, &officer, now_secs())
}

/// Stops between rows without ending: the run can be resumed.
#[tauri::command]
fn listrun_pause(session: tauri::State<Session>, run: String) {
    if let Ok(active) = session.listrun.lock() {
        if let Some(a) = active.as_ref().filter(|a| a.run == run) {
            a.stop.store(true, std::sync::atomic::Ordering::SeqCst);
        }
    }
}

#[tauri::command]
fn listrun_open(app: AppHandle) {
    open_listrun_window(&app);
}

pub fn run() {
    let shortcut = Shortcut::new(Some(Modifiers::SUPER | Modifiers::SHIFT), Code::Space);
    let app = tauri::Builder::default()
        .manage(Session::default())
        .plugin(
            tauri_plugin_global_shortcut::Builder::new()
                .with_shortcuts([shortcut])
                .expect("register shortcut")
                .with_handler(|app, s, event| {
                    if event.state == ShortcutState::Pressed {
                        // The stop shortcut is registered only while a replay runs.
                        if *s == run_ui::stop_shortcut() {
                            run_ui::stop_active(app);
                        } else {
                            toggle_canvas(app);
                        }
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
            listrun_open,
            listrun_preview,
            listrun_start,
            listrun_resume,
            listrun_unfinished,
            listrun_status,
            listrun_answer,
            listrun_end,
            listrun_pause,
            replay_start,
            task_catalog,
            run_ui::progress_current,
            run_ui::run_pause,
            run_ui::run_resume,
            run_ui::run_stop,
            audit_verify,
            task_detail,
            task_note_add,
            task_note_remove
        ])
        .setup(|app| {
            #[cfg(target_os = "macos")]
            app.set_activation_policy(tauri::ActivationPolicy::Accessory);

            let quit = tauri::menu::MenuItem::with_id(app, "quit", "Quit Companion", true, None::<&str>)?;
            let list = tauri::menu::MenuItem::with_id(app, "listrun", "Run a task over a spreadsheet...", true, None::<&str>)?;
            let notes = tauri::menu::MenuItem::with_id(app, "notes", "Task notes", true, None::<&str>)?;
            let menu = tauri::menu::Menu::with_items(app, &[&list, &notes, &quit])?;
            TrayIconBuilder::new()
                .icon(tauri::image::Image::from_bytes(include_bytes!("../icons/icon.png"))?)
                .icon_as_template(true)
                .tooltip("Companion")
                .menu(&menu)
                .show_menu_on_left_click(false)
                .on_menu_event(|app, e| {
                    if e.id() == "quit" {
                        app.exit(0);
                    } else if e.id() == "listrun" {
                        open_listrun_window(app);
                    } else if e.id() == "notes" {
                        open_task_window(app);
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
