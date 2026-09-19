#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
mod engine_client;
use engine_client::EngineClient;
use serde_json::Value;
#[cfg(debug_assertions)]
use std::path::PathBuf;
use std::sync::{
    atomic::{AtomicU64, Ordering},
    Mutex,
};
use tauri::Manager;

struct EngineState(Mutex<Option<EngineClient>>);

const DESIGN_WIDTH: f64 = 1920.0;
const DESIGN_HEIGHT: f64 = 1080.0;
const MIN_READABLE_SCALE: f64 = 0.9;
const WORK_AREA_MARGIN: f64 = 16.0;
const WINDOW_POLICY_DEBOUNCE_MS: u64 = 160;
static WINDOW_POLICY_REVISION: AtomicU64 = AtomicU64::new(0);

#[cfg(target_os = "windows")]
fn existing_known_folder(folder_id: &windows_sys::core::GUID) -> Result<std::path::PathBuf, String> {
    use std::os::windows::ffi::OsStringExt;
    use windows_sys::Win32::System::Com::CoTaskMemFree;
    use windows_sys::Win32::UI::Shell::SHGetKnownFolderPath;

    let mut raw_path = std::ptr::null_mut();
    let result = unsafe { SHGetKnownFolderPath(folder_id, 0, std::ptr::null_mut(), &mut raw_path) };
    if result < 0 || raw_path.is_null() {
        return Err(format!("SHGetKnownFolderPath failed with HRESULT 0x{:08X}", result as u32));
    }

    let length = unsafe {
        let mut length = 0usize;
        while *raw_path.add(length) != 0 {
            length += 1;
        }
        length
    };
    let path = std::path::PathBuf::from(std::ffi::OsString::from_wide(unsafe {
        std::slice::from_raw_parts(raw_path, length)
    }));
    unsafe { CoTaskMemFree(raw_path.cast()) };

    if path.is_dir() {
        Ok(path)
    } else {
        Err(format!("Configured image directory does not exist: {}", path.display()))
    }
}

#[cfg(target_os = "windows")]
fn is_descendant_path(path: &std::path::Path, parent: &std::path::Path) -> bool {
    let normalize = |value: &std::path::Path| {
        value
            .to_string_lossy()
            .replace('/', "\\")
            .trim_end_matches('\\')
            .to_lowercase()
    };
    let path = normalize(path);
    let parent = normalize(parent);
    path.len() > parent.len()
        && path.starts_with(&parent)
        && path.as_bytes().get(parent.len()) == Some(&b'\\')
}

#[cfg(target_os = "windows")]
fn select_image_directory(
    pictures: Result<std::path::PathBuf, String>,
    camera_roll: Result<std::path::PathBuf, String>,
) -> Result<std::path::PathBuf, String> {
    match (pictures, camera_roll) {
        (Ok(pictures), Ok(camera_roll)) if is_descendant_path(&camera_roll, &pictures) => {
            Ok(camera_roll)
        }
        (Ok(pictures), _) => Ok(pictures),
        (Err(_), Ok(camera_roll)) => Ok(camera_roll),
        (Err(pictures_error), Err(camera_roll_error)) => Err(format!(
            "Unable to resolve an existing Windows image directory. Pictures: {pictures_error}; Camera Roll: {camera_roll_error}"
        )),
    }
}

#[cfg(target_os = "windows")]
#[tauri::command]
fn resolve_default_image_directory() -> Result<String, String> {
    use windows_sys::Win32::UI::Shell::{FOLDERID_CameraRoll, FOLDERID_Pictures};

    let selected = select_image_directory(
        existing_known_folder(&FOLDERID_Pictures),
        existing_known_folder(&FOLDERID_CameraRoll),
    )?;
    Ok(selected.to_string_lossy().into_owned())
}

#[cfg(not(target_os = "windows"))]
#[tauri::command]
fn resolve_default_image_directory() -> Result<String, String> {
    Err("Default image directory resolution is only supported on Windows".to_string())
}

/// Opens an existing directory in Windows Explorer so the operator can inspect the
/// projection images. Anything that is not an existing directory is rejected: the
/// path comes from the scan setup or from the native picker only.
#[tauri::command]
fn open_directory_in_shell(path: String) -> Result<String, String> {
    let directory = std::path::PathBuf::from(&path);
    if !directory.is_dir() {
        return Err(format!("Not an existing directory: {path}"));
    }
    #[cfg(target_os = "windows")]
    {
        std::process::Command::new("explorer")
            .arg(&directory)
            .spawn()
            .map_err(|error| format!("Failed to open Windows Explorer: {error}"))?;
        Ok(directory.to_string_lossy().into_owned())
    }
    #[cfg(not(target_os = "windows"))]
    {
        Err("Opening a folder is only supported on Windows".to_string())
    }
}

/// Writes the exported session log. The target is chosen through the native save
/// dialog; only log/txt files inside an existing folder are accepted so the menu
/// entry cannot be used to write arbitrary files.
#[tauri::command]
fn export_session_log(path: String, contents: String) -> Result<String, String> {
    let target = std::path::PathBuf::from(&path);
    let extension = target
        .extension()
        .and_then(|value| value.to_str())
        .map(|value| value.to_ascii_lowercase())
        .unwrap_or_default();
    if extension != "log" && extension != "txt" {
        return Err("Only .log or .txt session logs can be exported".to_string());
    }
    let parent = target
        .parent()
        .ok_or_else(|| "Export target has no parent directory".to_string())?;
    if !parent.is_dir() {
        return Err("Export target folder does not exist".to_string());
    }
    std::fs::write(&target, contents)
        .map(|_| target.to_string_lossy().into_owned())
        .map_err(|error| format!("Failed to write session log: {error}"))
}

fn calculate_min_inner_size(
    work_area_width: f64,
    work_area_height: f64,
    decoration_width: f64,
    decoration_height: f64,
) -> (f64, f64) {
    let available_width =
        (work_area_width - WORK_AREA_MARGIN - decoration_width.max(0.0)).max(1.0);
    let available_height =
        (work_area_height - WORK_AREA_MARGIN - decoration_height.max(0.0)).max(1.0);
    let target_width = DESIGN_WIDTH * MIN_READABLE_SCALE;
    let target_height = DESIGN_HEIGHT * MIN_READABLE_SCALE;

    (
        target_width.min(available_width).max(1.0),
        target_height.min(available_height).max(1.0),
    )
}

/// Applies the readable minimum only while the window is restored.
///
/// `set_min_size` ends up in `SetWindowPos`, and positioning a maximized window
/// drops the maximized state while keeping the maximized size: the window then
/// looks maximized but is offset and `IsZoomed` stays false. Maximized windows
/// therefore keep the startup state untouched; the minimum is applied on the
/// first resize that leaves maximized mode.
fn apply_dynamic_min_size(window: &tauri::WebviewWindow) -> tauri::Result<()> {
    if window.is_maximized().unwrap_or(false) {
        return Ok(());
    }
    if let Some(monitor) = window.current_monitor()? {
        let scale = monitor.scale_factor();
        let work_area = monitor.work_area().size.to_logical::<f64>(scale);
        let outer_size = window.outer_size()?.to_logical::<f64>(scale);
        let inner_size = window.inner_size()?.to_logical::<f64>(scale);
        let decoration_width = (outer_size.width - inner_size.width).max(0.0);
        let decoration_height = (outer_size.height - inner_size.height).max(0.0);
        let (min_width, min_height) = calculate_min_inner_size(
            work_area.width,
            work_area.height,
            decoration_width,
            decoration_height,
        );
        window.set_min_size(Some(tauri::LogicalSize::new(min_width, min_height)))?;
    }
    Ok(())
}

/// Startup order matters: the window is created restored so Windows can record
/// restore bounds, the readable minimum is applied while it is still restored,
/// and only then is the window maximized. Maximizing from the configuration
/// skips the restored state, which leaves the shell at maximized size with
/// `IsZoomed` false and no placement to restore to.
fn configure_window(window: &tauri::WebviewWindow) -> tauri::Result<()> {
    apply_dynamic_min_size(window)?;
    window.maximize()
}

fn schedule_dynamic_min_size(window: tauri::WebviewWindow) {
    let revision = WINDOW_POLICY_REVISION.fetch_add(1, Ordering::Relaxed) + 1;
    std::thread::spawn(move || {
        std::thread::sleep(std::time::Duration::from_millis(WINDOW_POLICY_DEBOUNCE_MS));
        if WINDOW_POLICY_REVISION.load(Ordering::Relaxed) != revision {
            return;
        }
        let app = window.app_handle().clone();
        let _ = app.run_on_main_thread(move || {
            if let Err(error) = apply_dynamic_min_size(&window) {
                eprintln!("Unable to refresh dynamic window minimum: {error}");
            }
        });
    });
}

#[cfg(debug_assertions)]
fn append_diagnostic(line: &str) {
    use std::io::Write;
    let path: PathBuf = std::env::temp_dir().join("micro-ct-diagnostics.log");
    if let Ok(mut file) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
    {
        let _ = writeln!(file, "{line}");
    }
}

#[cfg(debug_assertions)]
fn write_startup_snapshot(snapshot: &Value) -> Result<(), String> {
    let path: PathBuf = std::env::temp_dir().join("micro-ct-startup-snapshot.json");
    let document = serde_json::to_vec_pretty(snapshot).map_err(|error| error.to_string())?;
    std::fs::write(&path, document).map_err(|error| {
        format!(
            "Unable to write startup snapshot evidence at {}: {error}",
            path.display()
        )
    })?;
    Ok(())
}

/// Connects to `ct-engine` off the UI thread.
///
/// Tauri creates the webview before `setup` runs but only starts pumping the
/// event loop afterwards, so any blocking work in `setup` starves WebView2
/// while it is bringing up its browser process. That shows up as a window whose
/// content area goes black a few seconds after launch.
fn connect_engine(app: &tauri::AppHandle) -> Result<(), String> {
    let mut engine = EngineClient::spawn()
        .map_err(|error| format!("Unable to start ct-engine: {error}"))?;
    let initial_snapshot = engine
        .request("snapshot", serde_json::json!({}))
        .map_err(|error| format!("ct-engine startup snapshot failed: {error}"))?;
    let startup_snapshot = if initial_snapshot.get("mode").and_then(Value::as_str)
        == Some("production_locked")
        && initial_snapshot
            .get("connectionState")
            .and_then(Value::as_str)
            == Some("disconnected")
    {
        engine
            .request(
                "connect",
                serde_json::json!({"type":"connect","adapter":"real_hardware"}),
            )
            .map_err(|error| format!("Nano startup connection failed: {error}"))?
    } else {
        initial_snapshot
    };
    #[cfg(debug_assertions)]
    {
        eprintln!("CT_ENGINE_STARTUP_SNAPSHOT={startup_snapshot}");
        write_startup_snapshot(&startup_snapshot)?;
    }
    #[cfg(not(debug_assertions))]
    let _ = &startup_snapshot;

    let state = app.state::<EngineState>();
    let mut guard = state
        .0
        .lock()
        .map_err(|_| "Engine client unavailable".to_string())?;
    *guard = Some(engine);
    Ok(())
}

fn bootstrap_engine(app: tauri::AppHandle) {
    std::thread::spawn(move || {
        let started = std::time::Instant::now();
        match connect_engine(&app) {
            Ok(()) => {
                #[cfg(debug_assertions)]
                append_diagnostic(&format!(
                    "ENGINE_READY_MS={}",
                    started.elapsed().as_millis()
                ));
                #[cfg(not(debug_assertions))]
                let _ = started;
            }
            Err(error) => {
                eprintln!("ct-engine bootstrap failed: {error}");
                #[cfg(debug_assertions)]
                append_diagnostic(&format!("ENGINE_BOOTSTRAP_FAILED {error}"));
            }
        }
    });
}

/// Commands are async so Tauri runs them on a worker thread: each request does a
/// blocking round-trip to `ct-engine` and the UI polls this every 850 ms.
#[tauri::command]
async fn engine_snapshot(state: tauri::State<'_, EngineState>) -> Result<Value, String> {
    let mut guard = state.0.lock().map_err(|_| "Engine client unavailable")?;
    guard
        .as_mut()
        .ok_or_else(|| "ct-engine is still starting".to_string())?
        .request("snapshot", serde_json::json!({}))
}

#[tauri::command]
async fn engine_command(
    command: Value,
    state: tauri::State<'_, EngineState>,
) -> Result<Value, String> {
    let name = command
        .get("type")
        .and_then(Value::as_str)
        .ok_or("Command type is required")?;
    let mut guard = state.0.lock().map_err(|_| "Engine client unavailable")?;
    guard
        .as_mut()
        .ok_or_else(|| "ct-engine is still starting".to_string())?
        .request(name, command.clone())
}

#[cfg(all(test, target_os = "windows"))]
mod tests {
    use super::{calculate_min_inner_size, export_session_log, open_directory_in_shell,
                select_image_directory};
    use std::path::PathBuf;

    #[test]
    fn minimum_inner_size_uses_readable_design_scale_when_space_allows() {
        assert_eq!(
            calculate_min_inner_size(1920.0, 1080.0, 16.0, 40.0),
            (1728.0, 972.0)
        );
    }

    #[test]
    fn minimum_inner_size_clamps_to_low_resolution_available_area() {
        assert_eq!(
            calculate_min_inner_size(1024.0, 768.0, 16.0, 40.0),
            (992.0, 712.0)
        );
    }

    #[test]
    fn minimum_inner_size_never_exceeds_available_area_or_drops_below_one() {
        let (width, height) = calculate_min_inner_size(10.0, 10.0, 100.0, 100.0);
        assert_eq!((width, height), (1.0, 1.0));
    }

    #[test]
    fn image_directory_selection_rejects_legacy_camera_roll_on_another_drive() {
        let pictures = PathBuf::from(r"E:\Main\Pictures");
        let camera_roll = PathBuf::from(r"C:\Users\x\Pictures\Camera Roll");

        assert_eq!(
            select_image_directory(Ok(pictures.clone()), Ok(camera_roll)),
            Ok(pictures)
        );
    }

    #[test]
    fn image_directory_selection_prefers_camera_roll_below_current_pictures() {
        let pictures = PathBuf::from(r"E:\Main\Pictures");
        let camera_roll = PathBuf::from(r"E:\Main\Pictures\Camera Roll");

        assert_eq!(
            select_image_directory(Ok(pictures), Ok(camera_roll.clone())),
            Ok(camera_roll)
        );
    }

    #[test]
    fn image_directory_selection_uses_pictures_without_camera_roll() {
        let pictures = PathBuf::from(r"E:\Main\Pictures");

        assert_eq!(
            select_image_directory(Ok(pictures.clone()), Err("Camera Roll missing".to_string())),
            Ok(pictures)
        );
    }

    #[test]
    fn image_directory_selection_uses_camera_roll_when_pictures_is_missing() {
        let camera_roll = PathBuf::from(r"E:\Main\Pictures\Camera Roll");

        assert_eq!(
            select_image_directory(Err("Pictures missing".to_string()), Ok(camera_roll.clone())),
            Ok(camera_roll)
        );
    }

    #[test]
    fn image_directory_selection_reports_both_lookup_failures() {
        let error = select_image_directory(
            Err("Pictures missing".to_string()),
            Err("Camera Roll missing".to_string()),
        )
        .expect_err("both lookup failures must be reported");

        assert!(error.contains("Pictures: Pictures missing"));
        assert!(error.contains("Camera Roll: Camera Roll missing"));
    }

    #[test]
    fn export_session_log_writes_log_targets_and_rejects_other_extensions() {
        // The parent is TEMP itself, which always exists: this contract never
        // creates directories.
        let folder = std::env::temp_dir();
        let target = folder.join("micro-ct-session-contract.log");
        std::fs::remove_file(&target).ok();
        let written = export_session_log(
            target.to_string_lossy().into_owned(),
            "line one\n".to_string(),
        )
        .expect("a .log target inside an existing folder must be writable");
        assert_eq!(written, target.to_string_lossy().into_owned());
        assert_eq!(
            std::fs::read_to_string(&target).expect("written log must be readable"),
            "line one\n"
        );
        std::fs::remove_file(&target).ok();

        assert_eq!(
            export_session_log(
                folder.join("session.exe").to_string_lossy().into_owned(),
                "x".to_string(),
            )
            .expect_err("non-log extensions must be rejected"),
            "Only .log or .txt session logs can be exported"
        );
    }

    #[test]
    fn export_session_log_rejects_a_missing_parent_directory() {
        let missing = std::env::temp_dir()
            .join("micro-ct-export-missing-folder")
            .join("session-log.txt");

        assert_eq!(
            export_session_log(missing.to_string_lossy().into_owned(), "x".to_string())
                .expect_err("a missing folder must be rejected"),
            "Export target folder does not exist"
        );
    }

    #[test]
    fn open_directory_in_shell_rejects_paths_that_are_not_directories() {
        let error = open_directory_in_shell(
            std::env::temp_dir()
                .join("micro-ct-open-missing-folder")
                .to_string_lossy()
                .into_owned(),
        )
        .expect_err("a missing directory must be rejected before Explorer is launched");

        assert!(error.starts_with("Not an existing directory:"));
    }
}

fn main() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .on_page_load(|webview, payload| {
            #[cfg(debug_assertions)]
            append_diagnostic(&format!(
                "PAGE_LOAD label={} event={:?} url={}",
                webview.label(),
                payload.event(),
                payload.url()
            ));
            #[cfg(not(debug_assertions))]
            let _ = (webview, payload);
        })
        .on_window_event(|window, event| {
            if window.label() != "main" {
                return;
            }
            if matches!(
                event,
                tauri::WindowEvent::Moved(_)
                    | tauri::WindowEvent::Resized(_)
                    | tauri::WindowEvent::ScaleFactorChanged { .. }
            ) {
                if let Some(webview_window) =
                    window.app_handle().get_webview_window(window.label())
                {
                    schedule_dynamic_min_size(webview_window);
                }
            }
        })
        .manage(EngineState(Mutex::new(None)))
        .setup(|app| {
            bootstrap_engine(app.app_handle().clone());
            if let Some(window) = app.get_webview_window("main") {
                configure_window(&window)?;
            }
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            engine_snapshot,
            engine_command,
            resolve_default_image_directory,
            open_directory_in_shell,
            export_session_log
        ])
        .build(tauri::generate_context!())
        .expect("Unable to initialize Micro-CT Workstation")
        .run(|app, event| {
            if matches!(event, tauri::RunEvent::Exit) {
                if let Ok(mut guard) = app.state::<EngineState>().0.lock() {
                    guard.take();
                }
            }
        });
}
