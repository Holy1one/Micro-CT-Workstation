#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
mod engine_client;
use engine_client::EngineClient;
use serde_json::Value;
use std::sync::Mutex;
use tauri::Manager;

struct EngineState(Mutex<Option<EngineClient>>);

#[tauri::command]
fn engine_snapshot(state: tauri::State<EngineState>) -> Result<Value, String> {
    let mut guard = state.0.lock().map_err(|_| "Engine client unavailable")?;
    guard
        .as_mut()
        .ok_or("Engine is not running; restart the app")?
        .request("snapshot", serde_json::json!({}))
}

#[tauri::command]
fn engine_command(command: Value, state: tauri::State<EngineState>) -> Result<Value, String> {
    let name = command
        .get("type")
        .and_then(Value::as_str)
        .ok_or("Command type is required")?;
    let mut guard = state.0.lock().map_err(|_| "Engine client unavailable")?;
    guard
        .as_mut()
        .ok_or("Engine is not running; restart the app")?
        .request(name, command.clone())
}

fn main() {
    tauri::Builder::default()
        .manage(EngineState(Mutex::new(None)))
        .setup(|app| {
            let engine = EngineClient::spawn()?;
            *app.state::<EngineState>()
                .0
                .lock()
                .map_err(|_| "Engine client unavailable")? = Some(engine);
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![engine_snapshot, engine_command])
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
