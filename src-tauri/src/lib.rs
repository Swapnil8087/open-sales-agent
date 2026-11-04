pub mod audio;
pub mod engine;
pub mod recorder;
pub mod stt;
pub mod session;
pub mod storage;
pub mod window;

use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use tauri::{
    menu::{Menu, MenuItem},
    tray::TrayIconBuilder,
    AppHandle, Emitter, Manager, PhysicalPosition, WebviewWindow, WindowEvent,
};
use tauri_plugin_global_shortcut::{GlobalShortcutExt, ShortcutState};

pub const DEFAULT_HOTKEY: &str = "Alt+Space";
const BAR_LABEL: &str = "main";

#[cfg(target_os = "macos")]
tauri_nspanel::tauri_panel! {
    panel!(BarPanel {
        config: {
            can_become_key_window: true,
            is_floating_panel: true
        }
    })
}

/// Turn the Tauri window into a non-activating panel so it can float over
/// fullscreen apps and every Space, like Spotlight.
#[cfg(target_os = "macos")]
fn make_panel(app: &AppHandle) -> Result<(), Box<dyn std::error::Error>> {
    use tauri_nspanel::{CollectionBehavior, PanelLevel, StyleMask, WebviewWindowExt};
    let win = bar(app).ok_or("main window missing")?;
    let panel = win.to_panel::<BarPanel>()?;
    panel.set_level(PanelLevel::Status.value());
    panel.add_style_mask(StyleMask::empty().nonactivating_panel().into())?;
    panel.set_collection_behavior(
        CollectionBehavior::new()
            .full_screen_auxiliary()
            .can_join_all_spaces()
            .into(),
    );
    Ok(())
}

/// Provision the data directory on boot. Returns the root that was prepared.
pub fn init_storage(root: Option<PathBuf>) -> std::io::Result<PathBuf> {
    let root = root
        .or_else(storage::default_root)
        .ok_or_else(|| std::io::Error::other("cannot resolve Documents directory"))?;
    storage::ensure_dirs(&root)?;
    Ok(root)
}

fn bar(app: &AppHandle) -> Option<WebviewWindow> {
    app.get_webview_window(BAR_LABEL)
}

/// Place the bar top-center on the monitor under the cursor.
fn position_bar(app: &AppHandle, win: &WebviewWindow) {
    let monitors = app.available_monitors().unwrap_or_default();
    let areas: Vec<window::WorkArea> = monitors
        .iter()
        .map(|m| {
            let w = m.work_area();
            window::WorkArea {
                x: w.position.x,
                y: w.position.y,
                width: w.size.width,
                height: w.size.height,
                scale: m.scale_factor(),
            }
        })
        .collect();
    if areas.is_empty() {
        return;
    }
    let idx = app
        .cursor_position()
        .ok()
        .and_then(|c| window::area_containing(&areas, c.x as i32, c.y as i32))
        .unwrap_or(0);
    let area = areas[idx];
    let win_w = win.outer_size().map(|s| s.width).unwrap_or(1360);
    let (x, y) = window::compute_position(area, win_w);
    let _ = win.set_position(PhysicalPosition::new(x, y));
}

fn show_bar(app: &AppHandle) {
    if let Some(win) = bar(app) {
        position_bar(app, &win);
        #[cfg(target_os = "macos")]
        {
            use tauri_nspanel::ManagerExt;
            if let Ok(p) = app.get_webview_panel(BAR_LABEL) {
                p.show_and_make_key();
            }
        }
        #[cfg(not(target_os = "macos"))]
        {
            let _ = win.show();
            let _ = win.set_focus();
        }
        let _ = app.emit("bar:shown", ());
    }
}

fn hide_bar(app: &AppHandle) {
    if let Some(win) = bar(app) {
        let _ = win.hide();
        let _ = app.emit("bar:hidden", ());
    }
}

fn toggle_bar(app: &AppHandle) {
    match bar(app).and_then(|w| w.is_visible().ok()) {
        Some(true) => hide_bar(app),
        _ => show_bar(app),
    }
}

#[derive(Default)]
struct RecorderState(Mutex<Option<recorder::Recorder>>);

#[derive(serde::Serialize)]
struct SttStatus {
    ready: bool,
    message: Option<String>,
}

fn stt_status_inner() -> Result<stt::WhisperCli, String> {
    let bin = stt::first_existing(&stt::whisper_bin_candidates())
        .ok_or("Whisper engine not installed. Run: brew install whisper-cpp")?;
    let model = stt::model_path().ok_or("Cannot resolve app data directory")?;
    if !model.is_file() {
        return Err(format!("Whisper model missing. Place {} at {}", stt::MODEL_FILE, model.display()));
    }
    Ok(stt::WhisperCli::new(bin, model))
}

#[tauri::command]
fn stt_status() -> SttStatus {
    match stt_status_inner() {
        Ok(_) => SttStatus { ready: true, message: None },
        Err(m) => SttStatus { ready: false, message: Some(m) },
    }
}

#[tauri::command]
fn start_recording(app: AppHandle, state: tauri::State<RecorderState>) -> Result<(), String> {
    let mut slot = state.0.lock().unwrap();
    if slot.is_some() {
        return Ok(());
    }
    let whisper = stt_status_inner()?;
    let tmp = std::env::temp_dir().join(format!("open-sales-agent-{}", std::process::id()));
    let engine = Arc::new(engine::Engine::new(Arc::new(whisper), tmp));
    let emit: engine::Emit = Arc::new(move |e| match e {
        engine::Event::Level(v) => {
            let _ = app.emit("audio:level", v);
        }
        engine::Event::Partial(t) => {
            let _ = app.emit("transcript:partial", t);
        }
        engine::Event::Final(t) => {
            let _ = app.emit("transcript:final", t);
        }
        engine::Event::Error(m) => {
            let _ = app.emit("transcript:error", m);
        }
    });
    *slot = Some(recorder::Recorder::start(engine, emit)?);
    Ok(())
}

#[tauri::command]
async fn stop_recording(state: tauri::State<'_, RecorderState>) -> Result<String, String> {
    let rec = state.0.lock().unwrap().take();
    match rec {
        // The final whisper pass can take a moment; keep it off the UI thread.
        Some(r) => tauri::async_runtime::spawn_blocking(move || r.stop())
            .await
            .map_err(|e| e.to_string()),
        None => Ok(String::new()),
    }
}

#[tauri::command]
fn hide_window(app: AppHandle) {
    hide_bar(&app);
}

#[tauri::command]
fn open_data_folder() -> Result<(), String> {
    let root = storage::default_root().ok_or("no Documents dir")?;
    storage::ensure_dirs(&root).map_err(|e| e.to_string())?;
    std::process::Command::new("open")
        .arg(root)
        .spawn()
        .map(|_| ())
        .map_err(|e| e.to_string())
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let builder = tauri::Builder::default();
    #[cfg(target_os = "macos")]
    let builder = builder.plugin(tauri_nspanel::init());
    builder
        .plugin(
            tauri_plugin_global_shortcut::Builder::new()
                .with_handler(|app, _shortcut, event| {
                    if event.state() == ShortcutState::Pressed {
                        toggle_bar(app);
                    }
                })
                .build(),
        )
        .manage(RecorderState::default())
        .invoke_handler(tauri::generate_handler![
            hide_window,
            open_data_folder,
            stt_status,
            start_recording,
            stop_recording
        ])
        .on_window_event(|window, event| {
            // Spotlight-style: clicking elsewhere dismisses the bar.
            if let WindowEvent::Focused(false) = event
                && window.label() == BAR_LABEL
            {
                hide_bar(window.app_handle());
            }
        })
        .setup(|app| {
            init_storage(None)?;

            #[cfg(target_os = "macos")]
            app.set_activation_policy(tauri::ActivationPolicy::Accessory);
            #[cfg(target_os = "macos")]
            make_panel(app.handle())?;

            let show = MenuItem::with_id(app, "show", "Show Open Sales Agent", true, None::<&str>)?;
            let quit = MenuItem::with_id(app, "quit", "Quit", true, None::<&str>)?;
            let menu = Menu::with_items(app, &[&show, &quit])?;
            TrayIconBuilder::new()
                .icon(app.default_window_icon().unwrap().clone())
                .menu(&menu)
                .on_menu_event(|app, e| match e.id.as_ref() {
                    "show" => show_bar(app),
                    "quit" => app.exit(0),
                    _ => {}
                })
                .build(app)?;

            if let Err(e) = app.global_shortcut().register(DEFAULT_HOTKEY) {
                eprintln!("hotkey {DEFAULT_HOTKEY} unavailable: {e}");
            }
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

#[cfg(test)]
mod tests {
    use super::*;
    use tauri_plugin_global_shortcut::Shortcut;

    #[test]
    fn boot_provisions_directory_tree() {
        let tmp = tempfile::tempdir().unwrap();
        let root = init_storage(Some(tmp.path().join("Open-Sales-Agent"))).unwrap();
        for d in ["MoM", "Inquiries", "Proposals", "logs"] {
            assert!(root.join(d).is_dir());
        }
    }

    #[test]
    fn default_hotkey_parses() {
        assert!(DEFAULT_HOTKEY.parse::<Shortcut>().is_ok());
    }
}
