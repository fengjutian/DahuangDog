mod monitor;
mod security;
mod storage;
mod types;

use monitor::Monitor;
use std::{
    collections::HashSet,
    path::PathBuf,
    process::Command,
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
use winreg::{enums::HKEY_CURRENT_USER, RegKey};
use types::{
    ActionPreview, ActionResult, AppUsageRecord, AppUsageSummary, CurrentStatus, HistorySummary,
    LocalDiagnosis, SecurityReport, UserSettings,
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
fn get_history_range(range_minutes: u32, state: tauri::State<'_, SharedMonitor>) -> Result<HistorySummary, String> {
    state.lock().map_err(|_| "监控状态暂时不可用".to_string())?.history_range(range_minutes)
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
fn open_file_location(path: String) -> Result<ActionResult, String> {
    let target = PathBuf::from(path);
    if !target.is_absolute() {
        return Err("只能打开绝对文件路径".into());
    }
    if !target.is_file() {
        return Err("文件已经不存在或暂时无法访问".into());
    }
    Command::new("explorer.exe")
        .arg(format!("/select,{}", target.display()))
        .spawn()
        .map_err(|error| format!("无法打开文件位置：{error}"))?;
    Ok(ActionResult {
        action_id: uuid::Uuid::new_v4().to_string(),
        success: true,
        message: "已经在资源管理器中定位文件".into(),
    })
}

#[tauri::command]
fn export_usage_csv(content: String) -> Result<ActionResult, String> {
    if content.len() > 10 * 1024 * 1024 { return Err("导出内容超过 10 MB 限制".into()); }
    let downloads = std::env::var_os("USERPROFILE").map(PathBuf::from)
        .ok_or("无法定位当前用户目录")?.join("Downloads");
    if !downloads.is_dir() { return Err("下载目录不存在或无法访问".into()); }
    let timestamp = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_secs();
    let target = downloads.join(format!("大黄狗-应用使用记录-{timestamp}.csv"));
    std::fs::write(&target, format!("\u{feff}{content}")).map_err(|error| format!("导出失败：{error}"))?;
    Command::new("explorer.exe").arg(format!("/select,{}", target.display())).spawn()
        .map_err(|error| format!("文件已导出，但无法打开资源管理器：{error}"))?;
    Ok(ActionResult { action_id: uuid::Uuid::new_v4().to_string(), success: true, message: format!("已导出到 {}", target.display()) })
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
    configure_auto_start(settings.auto_start)?;
    state
        .lock()
        .map_err(|_| "监控状态暂时不可用".to_string())?
        .update_settings(settings)
}

fn configure_auto_start(enabled: bool) -> Result<(), String> {
    let current = std::env::current_exe().map_err(|error| format!("无法读取程序路径：{error}"))?;
    let root = RegKey::predef(HKEY_CURRENT_USER);
    let (run, _) = root.create_subkey(r"Software\Microsoft\Windows\CurrentVersion\Run")
        .map_err(|error| format!("无法打开开机启动设置：{error}"))?;
    if enabled {
        run.set_value("DahuangDog", &format!("\"{}\" --background", current.display()))
            .map_err(|error| format!("无法启用开机自启动：{error}"))?;
    } else if run.get_raw_value("DahuangDog").is_ok() {
        run.delete_value("DahuangDog").map_err(|error| format!("无法关闭开机自启动：{error}"))?;
    }
    Ok(())
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

#[tauri::command]
fn get_app_usage_history(
    state: tauri::State<'_, SharedMonitor>,
) -> Result<Vec<AppUsageRecord>, String> {
    state
        .lock()
        .map_err(|_| "监控状态暂时不可用".to_string())?
        .app_usage_history()
}

#[tauri::command]
fn get_app_usage_summary(
    period_days: u32,
    state: tauri::State<'_, SharedMonitor>,
) -> Result<AppUsageSummary, String> {
    state
        .lock()
        .map_err(|_| "监控状态暂时不可用".to_string())?
        .app_usage_summary(period_days)
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
            get_history_range,
            diagnose_performance,
            get_security_report,
            open_file_location,
            export_usage_csv,
            get_settings,
            update_settings,
            clear_local_memory,
            open_process_location,
            prepare_process_priority,
            get_app_usage_history,
            get_app_usage_summary
        ])
        .setup(move |app| {
            let open = MenuItem::with_id(app, "open", "打开大黄狗", true, None::<&str>)?;
            let hardware = MenuItem::with_id(app, "hardware", "硬件监控", true, None::<&str>)?;
            let usage = MenuItem::with_id(app, "usage", "使用记录", true, None::<&str>)?;
            let security = MenuItem::with_id(app, "security", "看门报告", true, None::<&str>)?;
            let quit = MenuItem::with_id(app, "quit", "退出", true, None::<&str>)?;
            let menu = Menu::with_items(app, &[&open, &hardware, &usage, &security, &quit])?;
            let mut tray = TrayIconBuilder::with_id("main-tray")
                .menu(&menu)
                .tooltip("大黄狗正在巡逻")
                .show_menu_on_left_click(false)
                .on_menu_event(|app, event| match event.id().as_ref() {
                    "open" => show_main_window(app),
                    "hardware" | "usage" | "security" => {
                        show_main_window(app);
                        let _ = app.emit("ui://open-panel", event.id().as_ref());
                    }
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
            if std::env::args().any(|argument| argument == "--background") {
                if let Some(window) = app.get_webview_window("main") { let _ = window.hide(); }
            }

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
