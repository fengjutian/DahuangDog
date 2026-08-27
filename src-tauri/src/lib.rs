mod monitor;
mod security;
mod storage;
mod types;

use monitor::Monitor;
use std::{
    collections::HashSet,
    sync::{Arc, Mutex},
    thread,
    time::Duration,
};
use tauri::{
    menu::{Menu, MenuItem},
    tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent},
    Emitter, Manager,
};
use tauri_plugin_notification::NotificationExt;
use types::{
    ActionPreview, ActionResult, CurrentStatus, HistorySummary, LocalDiagnosis, SecurityReport,
    UserSettings,
};

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

#[tauri::command]
fn get_history(state: tauri::State<'_, SharedMonitor>) -> Result<HistorySummary, String> {
    state
        .lock()
        .map_err(|_| "监控状态暂时不可用".to_string())?
        .history()
}

#[tauri::command]
fn diagnose_performance(state: tauri::State<'_, SharedMonitor>) -> Result<LocalDiagnosis, String> {
    Ok(state
        .lock()
        .map_err(|_| "监控状态暂时不可用".to_string())?
        .diagnose())
}

#[tauri::command]
fn get_security_report() -> Result<SecurityReport, String> {
    security::scan_security()
}

#[tauri::command]
fn get_settings(state: tauri::State<'_, SharedMonitor>) -> Result<UserSettings, String> {
    Ok(state
        .lock()
        .map_err(|_| "监控状态暂时不可用".to_string())?
        .settings())
}

#[tauri::command]
fn update_settings(
    settings: UserSettings,
    state: tauri::State<'_, SharedMonitor>,
) -> Result<UserSettings, String> {
    state
        .lock()
        .map_err(|_| "监控状态暂时不可用".to_string())?
        .update_settings(settings)
}

#[tauri::command]
fn clear_local_memory(state: tauri::State<'_, SharedMonitor>) -> Result<(), String> {
    state
        .lock()
        .map_err(|_| "监控状态暂时不可用".to_string())?
        .clear_memory()
}

#[tauri::command]
fn open_process_location(
    pid: u32,
    started_at: u64,
    state: tauri::State<'_, SharedMonitor>,
) -> Result<ActionResult, String> {
    state
        .lock()
        .map_err(|_| "监控状态暂时不可用".to_string())?
        .open_process_location(pid, started_at)
}

#[tauri::command]
fn prepare_process_priority(
    pid: u32,
    started_at: u64,
    level: String,
    state: tauri::State<'_, SharedMonitor>,
) -> Result<ActionPreview, String> {
    state
        .lock()
        .map_err(|_| "监控状态暂时不可用".to_string())?
        .prepare_priority(pid, started_at, level)
}

fn show_main_window(app: &tauri::AppHandle) {
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.show();
        let _ = window.unminimize();
        let _ = window.set_focus();
    }
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
            confirm_action,
            get_history,
            diagnose_performance,
            get_security_report,
            get_settings,
            update_settings,
            clear_local_memory,
            open_process_location,
            prepare_process_priority
        ])
        .setup(move |app| {
            let open = MenuItem::with_id(app, "open", "打开大黄狗", true, None::<&str>)?;
            let quit = MenuItem::with_id(app, "quit", "退出", true, None::<&str>)?;
            let menu = Menu::with_items(app, &[&open, &quit])?;
            let mut tray = TrayIconBuilder::with_id("main-tray")
                .menu(&menu)
                .tooltip("大黄狗正在巡逻")
                .show_menu_on_left_click(false)
                .on_menu_event(|app, event| match event.id().as_ref() {
                    "open" => show_main_window(app),
                    "quit" => app.exit(0),
                    _ => {}
                })
                .on_tray_icon_event(|tray, event| {
                    if matches!(
                        event,
                        TrayIconEvent::Click {
                            button: MouseButton::Left,
                            button_state: MouseButtonState::Up,
                            ..
                        } | TrayIconEvent::DoubleClick {
                            button: MouseButton::Left,
                            ..
                        }
                    ) {
                        show_main_window(tray.app_handle());
                    }
                });
            if let Some(icon) = app.default_window_icon() {
                tray = tray.icon(icon.clone());
            }
            tray.build(app)?;

            let app_handle = app.handle().clone();
            let monitor = monitor.clone();
            thread::spawn(move || {
                let mut notified_findings = HashSet::new();
                loop {
                    let mut interval = 2;
                    if let Ok(mut guard) = monitor.lock() {
                        guard.refresh();
                        let status = guard.status();
                        for finding in &status.findings {
                            if guard.notifications_enabled()
                                && notified_findings.insert(finding.id.clone())
                            {
                                let _ = app_handle
                                    .notification()
                                    .builder()
                                    .title(format!("大黄狗：{}", finding.title))
                                    .body(&finding.message)
                                    .show();
                            }
                        }
                        let active: HashSet<_> = status
                            .findings
                            .iter()
                            .map(|finding| finding.id.clone())
                            .collect();
                        notified_findings.retain(|id| active.contains(id));
                        let _ = app_handle.emit("status://updated", status);
                        interval = guard.sampling_interval_seconds();
                    }
                    thread::sleep(Duration::from_secs(interval));
                }
            });
            Ok(())
        })
        .on_window_event(|window, event| {
            if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                api.prevent_close();
                let _ = window.hide();
            }
        })
        .run(tauri::generate_context!())
        .expect("failed to run DahuangDog");
}
