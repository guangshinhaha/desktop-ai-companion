//! Menu-bar shell. One tray icon, one canvas window created on demand and destroyed on close,
//! so an idle shell holds no web view. No network calls.

pub mod executor;

use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
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

pub fn run() {
    let shortcut = Shortcut::new(Some(Modifiers::SUPER | Modifiers::SHIFT), Code::Space);
    let app = tauri::Builder::default()
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
        .invoke_handler(tauri::generate_handler![close_canvas, ping_executor])
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
