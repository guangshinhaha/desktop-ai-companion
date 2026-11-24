//! Menu-bar shell. One tray icon, one canvas window created on demand and destroyed on close,
//! so an idle shell holds no web view. No network calls.

pub mod executor;
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
            ping_executor,
            record_start,
            record_stop,
            review_view,
            review_edit,
            review_save,
            review_discard
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
