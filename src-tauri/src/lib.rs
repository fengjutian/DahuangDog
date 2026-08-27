mod monitor;
mod storage;
mod types;

use monitor::Monitor;
use std::{
    sync::{Arc, Mutex},
    thread,
    time::Duration,
};
use tauri::Emitter;
use types::{ActionPreview, ActionResult, CurrentStatus};

type SharedMonitor = Arc<Mutex<Monitor>>;

#[tauri::command]
fn get_current_status(state: tauri::State<'_, SharedMonitor>) -> Result<CurrentStatus, String> {
    state
        .lock()
        .map_err(|_| "监控状态暂时不可用".to_string())
        .map(|m| m.status())
}

#[tauri::command]
fn prepare_terminate_process(
    pid: u32,
    started_at: u64,
    state: tauri::State<'_, SharedMonitor>,
) -> Result<ActionPreview, String> {
    state
        .lock()
        .map_err(|_| "监控状态暂时不可用".to_string())?
        .prepare_terminate(pid, started_at)
}

#[tauri::command]
fn confirm_action(
    preview_id: String,
    state: tauri::State<'_, SharedMonitor>,
) -> Result<ActionResult, String> {
    state
        .lock()
        .map_err(|_| "监控状态暂时不可用".to_string())?
        .confirm_action(&preview_id)
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let monitor = Arc::new(Mutex::new(Monitor::new()));
    tauri::Builder::default()
        .plugin(tauri_plugin_notification::init())
        .manage(monitor.clone())
        .invoke_handler(tauri::generate_handler![
            get_current_status,
            prepare_terminate_process,
            confirm_action
        ])
        .setup(move |app| {
            let app_handle = app.handle().clone();
            let monitor = monitor.clone();
            thread::spawn(move || loop {
                if let Ok(mut guard) = monitor.lock() {
                    guard.refresh();
                    let _ = app_handle.emit("status://updated", guard.status());
                }
                thread::sleep(Duration::from_secs(2));
            });
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("failed to run DahuangDog");
}
