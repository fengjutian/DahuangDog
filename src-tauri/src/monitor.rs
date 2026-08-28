use crate::storage::Storage;
use crate::network_etw::{NetworkCollector, NetworkRate};
use crate::types::{
    ActionPreview, ActionResult, AlertRecord, AppUsageRecord, ApplicationGroup, ApplicationHistory, CurrentStatus, Finding,
    BatteryMetric, CpuCoreMetric, DiskMetric, FanMetric, GpuMetric, HardwareSnapshot,
    HistorySummary, LocalDiagnosis, NetworkMetric, ProcessSample, SystemSnapshot, TemperatureMetric,
    PeriodicPattern, TimelineEvent, UserSettings, VerificationStatus,
};
use std::{
    collections::{HashMap, VecDeque},
    time::{SystemTime, UNIX_EPOCH},
};
use sysinfo::{Components, Disks, Networks, Pid, ProcessesToUpdate, System};
use uuid::Uuid;

const PREVIEW_TTL_SECONDS: u64 = 30;
const CRITICAL_PROCESSES: &[&str] = &[
    "system",
    "system idle process",
    "registry",
    "smss.exe",
    "csrss.exe",
    "wininit.exe",
    "services.exe",
    "lsass.exe",
    "winlogon.exe",
    "dwm.exe",
];

fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

fn is_critical(name: &str) -> bool {
    CRITICAL_PROCESSES
        .iter()
        .any(|candidate| candidate.eq_ignore_ascii_case(name))
}

fn required_high_samples(interval_seconds: u64) -> u8 {
    (60 / interval_seconds.max(1)).clamp(1, u8::MAX as u64) as u8
}

fn build_application_groups(
    processes: &[ProcessSample],
    network_rates: &HashMap<u32, NetworkRate>,
    network_available: bool,
) -> Vec<ApplicationGroup> {
    let index: HashMap<u32, &ProcessSample> = processes
        .iter()
        .map(|process| (process.pid, process))
        .collect();
    let mut grouped: HashMap<u32, Vec<ProcessSample>> = HashMap::new();
    for process in processes {
        let mut root_pid = process.pid;
        let mut parent_pid = process.parent_pid;
        let mut depth = 0;
        while let Some(pid) = parent_pid {
            let Some(parent) = index.get(&pid) else {
                break;
            };
            if !parent.name.eq_ignore_ascii_case(&process.name) {
                break;
            }
            root_pid = parent.pid;
            parent_pid = parent.parent_pid;
            depth += 1;
            if depth >= 32 {
                break;
            }
        }
        grouped.entry(root_pid).or_default().push(process.clone());
    }
    let mut applications: Vec<_> = grouped
        .into_iter()
        .filter_map(|(root_pid, mut members)| {
            members.sort_by(|a, b| b.cpu_percent.total_cmp(&a.cpu_percent));
            let root_process = index
                .get(&root_pid)
                .map(|process| (*process).clone())
                .or_else(|| members.first().cloned())?;
            let member_count = members.len();
            let cpu_percent = members.iter().map(|process| process.cpu_percent).sum();
            let memory_bytes = members.iter().map(|process| process.memory_bytes).sum();
            let disk_read_bps = members.iter().map(|process| process.disk_read_bps).sum();
            let disk_write_bps = members.iter().map(|process| process.disk_write_bps).sum();
            let network_receive_bps = members.iter().map(|process| network_rates.get(&process.pid).map(|rate| rate.receive_bps).unwrap_or(0)).sum::<u64>();
            let network_send_bps = members.iter().map(|process| network_rates.get(&process.pid).map(|rate| rate.send_bps).unwrap_or(0)).sum::<u64>();
            Some(ApplicationGroup {
                root_pid,
                name: root_process.name.clone(),
                member_count,
                cpu_percent,
                memory_bytes,
                disk_read_bps,
                disk_write_bps,
                network_bps: network_available.then_some(network_receive_bps.saturating_add(network_send_bps)),
                network_receive_bps: network_available.then_some(network_receive_bps),
                network_send_bps: network_available.then_some(network_send_bps),
                product_name: None,
                description: None,
                publisher: None,
                executable_path: root_process.executable_path.clone(),
                root_process,
                members,
            })
        })
        .collect();
    applications.sort_by(|a, b| {
        let a_score = a.cpu_percent as f64 * 100_000_000.0 + a.memory_bytes as f64;
        let b_score = b.cpu_percent as f64 * 100_000_000.0 + b.memory_bytes as f64;
        b_score.total_cmp(&a_score)
    });
    applications
}

fn foreground_process_id() -> Option<u32> {
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        GetForegroundWindow, GetWindowThreadProcessId,
    };
    unsafe {
        let window = GetForegroundWindow();
        if window.is_null() {
            return None;
        }
        let mut pid = 0;
        GetWindowThreadProcessId(window, &mut pid);
        (pid != 0).then_some(pid)
    }
}

fn process_thread_counts() -> HashMap<u32, usize> {
    use std::mem::size_of;
    use windows_sys::Win32::{
        Foundation::{CloseHandle, INVALID_HANDLE_VALUE},
        System::Diagnostics::ToolHelp::{CreateToolhelp32Snapshot, Thread32First, Thread32Next, THREADENTRY32, TH32CS_SNAPTHREAD},
    };
    let mut counts = HashMap::new();
    unsafe {
        let snapshot = CreateToolhelp32Snapshot(TH32CS_SNAPTHREAD, 0);
        if snapshot == INVALID_HANDLE_VALUE { return counts; }
        let mut entry: THREADENTRY32 = std::mem::zeroed();
        entry.dwSize = size_of::<THREADENTRY32>() as u32;
        if Thread32First(snapshot, &mut entry) != 0 {
            loop {
                *counts.entry(entry.th32OwnerProcessID).or_insert(0) += 1;
                if Thread32Next(snapshot, &mut entry) == 0 { break; }
            }
        }
        CloseHandle(snapshot);
    }
    counts
}

fn battery_metric() -> Option<BatteryMetric> {
    use windows_sys::Win32::System::Power::{GetSystemPowerStatus, SYSTEM_POWER_STATUS};
    unsafe {
        let mut status: SYSTEM_POWER_STATUS = std::mem::zeroed();
        if GetSystemPowerStatus(&mut status) == 0 || status.BatteryFlag == 128 { return None; }
        Some(BatteryMetric {
            charge_percent: status.BatteryLifePercent.min(100),
            charging: status.BatteryFlag & 8 != 0,
            ac_connected: status.ACLineStatus == 1,
            life_seconds: (status.BatteryLifeTime != u32::MAX).then_some(status.BatteryLifeTime as u64),
        })
    }
}

#[derive(Clone, Default)]
struct ApplicationMetadata {
    product_name: Option<String>,
    description: Option<String>,
    publisher: Option<String>,
}

#[cfg(windows)]
fn application_metadata(path: &str) -> ApplicationMetadata {
    use std::{ffi::c_void, slice};
    use windows_sys::Win32::Storage::FileSystem::{GetFileVersionInfoSizeW, GetFileVersionInfoW, VerQueryValueW};

    unsafe fn query_string(data: &[u8], key: &str, translations: &[(u16, u16)]) -> Option<String> {
        for &(language, codepage) in translations {
            let query = format!(r"\StringFileInfo\{language:04x}{codepage:04x}\{key}")
                .encode_utf16().chain(Some(0)).collect::<Vec<_>>();
            let mut value: *mut c_void = std::ptr::null_mut();
            let mut length = 0_u32;
            if unsafe { VerQueryValueW(data.as_ptr().cast(), query.as_ptr(), &mut value, &mut length) } == 0 || value.is_null() || length <= 1 { continue; }
            let text = String::from_utf16_lossy(unsafe { slice::from_raw_parts(value.cast::<u16>(), length as usize) })
                .trim_end_matches('\0').trim().to_string();
            if !text.is_empty() { return Some(text); }
        }
        None
    }

    let wide_path = path.encode_utf16().chain(Some(0)).collect::<Vec<_>>();
    let mut ignored = 0_u32;
    let size = unsafe { GetFileVersionInfoSizeW(wide_path.as_ptr(), &mut ignored) };
    if size == 0 { return ApplicationMetadata::default(); }
    let mut data = vec![0_u8; size as usize];
    if unsafe { GetFileVersionInfoW(wide_path.as_ptr(), 0, size, data.as_mut_ptr().cast()) } == 0 { return ApplicationMetadata::default(); }

    let translation_query = "\\VarFileInfo\\Translation\0".encode_utf16().collect::<Vec<_>>();
    let mut translation_data: *mut c_void = std::ptr::null_mut();
    let mut translation_length = 0_u32;
    let mut translations = Vec::new();
    if unsafe { VerQueryValueW(data.as_ptr().cast(), translation_query.as_ptr(), &mut translation_data, &mut translation_length) } != 0 && !translation_data.is_null() {
        let bytes = unsafe { slice::from_raw_parts(translation_data.cast::<u8>(), translation_length as usize) };
        for pair in bytes.chunks_exact(4) { translations.push((u16::from_le_bytes([pair[0], pair[1]]), u16::from_le_bytes([pair[2], pair[3]]))); }
    }
    for fallback in [(0x0804, 0x04b0), (0x0409, 0x04b0), (0x0409, 0x04e4)] {
        if !translations.contains(&fallback) { translations.push(fallback); }
    }
    ApplicationMetadata {
        product_name: unsafe { query_string(&data, "ProductName", &translations) },
        description: unsafe { query_string(&data, "FileDescription", &translations) },
        publisher: unsafe { query_string(&data, "CompanyName", &translations) },
    }
}

#[cfg(not(windows))]
fn application_metadata(_: &str) -> ApplicationMetadata { ApplicationMetadata::default() }

fn gpu_metrics() -> (Vec<GpuMetric>, String) {
    #[cfg(windows)] use std::os::windows::process::CommandExt;
    let mut command = std::process::Command::new("nvidia-smi.exe");
    command.args(["--query-gpu=name,utilization.gpu,memory.used,memory.total", "--format=csv,noheader,nounits"]);
    #[cfg(windows)] command.creation_flags(0x08000000);
    let Ok(output) = command.output() else { return (vec![], "当前显卡驱动未提供可用的 GPU 性能接口".into()) };
    let text = String::from_utf8_lossy(&output.stdout);
    let values = text.lines().filter_map(|line| {
        let columns: Vec<_> = line.split(',').map(str::trim).collect();
        if columns.len() != 4 { return None; }
        Some(GpuMetric { name: columns[0].into(), usage_percent: columns[1].parse().ok()?, memory_used_bytes: columns[2].parse::<u64>().ok()? * 1024 * 1024, memory_total_bytes: columns[3].parse::<u64>().ok()? * 1024 * 1024 })
    }).collect::<Vec<_>>();
    let status = if values.is_empty() { "GPU 数据暂时不可用" } else { "由显卡驱动实时提供" }.into();
    (values, status)
}

#[derive(Default)]
struct RollingBaseline {
    values: VecDeque<f64>,
}

impl RollingBaseline {
    fn assess(&self, value: f64, absolute_floor: f64) -> Option<(f64, f64)> {
        if self.values.len() < 30 || value < absolute_floor {
            return None;
        }
        let mut sorted: Vec<_> = self.values.iter().copied().collect();
        sorted.sort_by(f64::total_cmp);
        let median = sorted[sorted.len() / 2];
        let mut deviations: Vec<_> = sorted.iter().map(|item| (item - median).abs()).collect();
        deviations.sort_by(f64::total_cmp);
        let mad = deviations[deviations.len() / 2].max(1.0);
        (value > median + 6.0 * mad && value > median * 2.5).then_some((median, mad))
    }

    fn push(&mut self, value: f64) {
        self.values.push_back(value);
        self.values.truncate(900);
    }
}

fn memory_growth(history: &VecDeque<(u64, u64)>) -> Option<(u64, u64)> {
    let (first_at, first) = history.front().copied()?;
    let (last_at, last) = history.back().copied()?;
    let growth = last.saturating_sub(first);
    (last_at.saturating_sub(first_at) >= 20 * 60
        && growth >= 1024 * 1024 * 1024
        && last >= first.saturating_add(first / 2))
    .then_some((growth, last_at.saturating_sub(first_at)))
}

fn disk_space_pressure(available_bytes: u64, total_bytes: u64) -> Option<(f64, &'static str)> {
    if total_bytes == 0 {
        return None;
    }
    let free_percent = available_bytes as f64 / total_bytes as f64 * 100.0;
    if free_percent < 5.0 {
        Some((free_percent, "critical"))
    } else if free_percent < 10.0 {
        Some((free_percent, "warning"))
    } else {
        None
    }
}

fn lowest_disk_pressure(snapshot: &SystemSnapshot) -> Option<(&DiskMetric, f64, &'static str)> {
    snapshot
        .hardware
        .disks
        .iter()
        .filter_map(|disk| {
            disk_space_pressure(disk.available_bytes, disk.total_bytes)
                .map(|(percent, severity)| (disk, percent, severity))
        })
        .min_by(|left, right| left.1.total_cmp(&right.1))
}

fn snapshot_health_score(snapshot: &SystemSnapshot) -> u8 {
    let resource_score = (100.0 - snapshot.cpu_percent.max(snapshot.memory_percent) * 0.55)
        .clamp(0.0, 100.0) as u8;
    match lowest_disk_pressure(snapshot) {
        Some((_, _, "critical")) => resource_score.min(45),
        Some(_) => resource_score.min(70),
        None => resource_score,
    }
}

#[derive(Clone)]
struct PendingAction {
    preview: ActionPreview,
    kind: PendingActionKind,
}

#[derive(Clone)]
enum PendingActionKind {
    Terminate,
    SetPriority(String),
}

struct VerificationContext {
    action_id: String,
    target_pid: u32,
    baseline_pressure: f32,
    check_after: u64,
}

pub struct Monitor {
    system: System,
    disks: Disks,
    networks: Networks,
    components: Option<Components>,
    storage: Option<Storage>,
    snapshot: Option<SystemSnapshot>,
    findings: Vec<Finding>,
    timeline: VecDeque<TimelineEvent>,
    pending: HashMap<String, PendingAction>,
    high_cpu_samples: u8,
    high_memory_samples: u8,
    verification: Option<VerificationStatus>,
    verification_context: Option<VerificationContext>,
    settings: UserSettings,
    disk_baseline: RollingBaseline,
    network_baseline: RollingBaseline,
    disk_anomaly_samples: u8,
    network_anomaly_samples: u8,
    memory_history: HashMap<(u32, u64), VecDeque<(u64, u64)>>,
    last_alert_at: HashMap<String, u64>,
    app_sessions: HashMap<(u32, u64), AppUsageRecord>,
    last_lifecycle_tick: u64,
    last_patrol_at: u64,
    network_collector: NetworkCollector,
    application_metadata: HashMap<String, ApplicationMetadata>,
}

impl Monitor {
    pub fn new() -> Self {
        let storage = Storage::open_default().ok();
        if let Some(storage) = &storage {
            let _ = storage.close_stale_app_sessions(now() * 1000);
        }
        let settings = storage
            .as_ref()
            .map(Storage::load_settings)
            .unwrap_or_default();
        let application_network_monitoring = settings.application_network_monitoring;
        let mut monitor = Self {
            // 首次完整采样由后台巡逻线程执行，避免在创建 WebView 前同步枚举所有进程。
            system: System::new(),
            disks: Disks::new_with_refreshed_list(),
            networks: Networks::new_with_refreshed_list(),
            components: None,
            storage,
            snapshot: None,
            findings: vec![],
            timeline: VecDeque::new(),
            pending: HashMap::new(),
            high_cpu_samples: 0,
            high_memory_samples: 0,
            verification: None,
            verification_context: None,
            settings,
            disk_baseline: RollingBaseline::default(),
            network_baseline: RollingBaseline::default(),
            disk_anomaly_samples: 0,
            network_anomaly_samples: 0,
            memory_history: HashMap::new(),
            last_alert_at: HashMap::new(),
            app_sessions: HashMap::new(),
            last_lifecycle_tick: now(),
            last_patrol_at: now(),
            network_collector: NetworkCollector::new(application_network_monitoring),
            application_metadata: HashMap::new(),
        };
        if let Some(storage) = &monitor.storage {
            if let Ok(events) = storage.recent_events(50) {
                monitor.timeline = events.into();
            }
        }
        monitor.push_event("patrol", "大黄狗醒了，开始巡逻");
        monitor
    }

    pub fn refresh(&mut self) {
        self.system.refresh_cpu_usage();
        self.system.refresh_memory();
        self.system.refresh_processes(ProcessesToUpdate::All, true);
        self.disks.refresh(true);
        self.networks.refresh(true);
        if let Some(components) = &mut self.components {
            components.refresh(true);
        } else {
            self.components = Some(Components::new_with_refreshed_list());
        }

        let total = self.system.total_memory();
        let used = self.system.used_memory();
        let memory_percent = if total == 0 {
            0.0
        } else {
            used as f32 / total as f32 * 100.0
        };
        let thread_counts = process_thread_counts();
        let mut processes: Vec<_> = self
            .system
            .processes()
            .iter()
            .map(|(pid, process)| {
                let name = process.name().to_string_lossy().into_owned();
                ProcessSample {
                    pid: pid.as_u32(),
                    parent_pid: process.parent().map(|parent| parent.as_u32()),
                    started_at: process.start_time(),
                    name: name.clone(),
                    cpu_percent: process.cpu_usage(),
                    memory_bytes: process.memory(),
                    is_critical: is_critical(&name),
                    thread_count: thread_counts.get(&pid.as_u32()).copied().unwrap_or(0),
                    handle_count: process.open_files(),
                    disk_read_bps: process.disk_usage().read_bytes / self.sampling_interval_seconds().max(1),
                    disk_write_bps: process.disk_usage().written_bytes / self.sampling_interval_seconds().max(1),
                    executable_path: process.exe().map(|path| path.to_string_lossy().into_owned()),
                }
            })
            .collect();
        processes.sort_by(|a, b| {
            let a_score = a.cpu_percent as f64 * 100_000_000.0 + a.memory_bytes as f64;
            let b_score = b.cpu_percent as f64 * 100_000_000.0 + b.memory_bytes as f64;
            b_score.total_cmp(&a_score)
        });
        let app_network_status = self.network_collector.status();
        let network_rates = self.network_collector.rates(self.sampling_interval_seconds());
        let mut all_applications = build_application_groups(
            &processes,
            &network_rates,
            app_network_status.state == "running",
        );
        for application in &mut all_applications {
            let Some(path) = application.executable_path.as_ref() else { continue };
            let metadata = self.application_metadata.entry(path.clone()).or_insert_with(|| application_metadata(path));
            application.product_name = metadata.product_name.clone();
            application.description = metadata.description.clone();
            application.publisher = metadata.publisher.clone();
        }
        self.update_app_lifecycle(&all_applications);
        let applications = all_applications
            .into_iter()
            .take(30)
            .map(|mut application| {
                application.members.truncate(30);
                application
            })
            .collect();
        processes.truncate(40);

        let (disk_read_bytes, disk_write_bytes) =
            self.disks
                .list()
                .iter()
                .fold((0_u64, 0_u64), |(read, write), disk| {
                    let usage = disk.usage();
                    (
                        read.saturating_add(usage.read_bytes),
                        write.saturating_add(usage.written_bytes),
                    )
                });
        let (network_receive_bytes, network_send_bytes) =
            self.networks
                .iter()
                .fold((0_u64, 0_u64), |(received, sent), (_, data)| {
                    (
                        received.saturating_add(data.received()),
                        sent.saturating_add(data.transmitted()),
                    )
                });

        let (gpus, gpu_status) = gpu_metrics();
        let hardware = HardwareSnapshot {
            cpu_cores: self.system.cpus().iter().enumerate().map(|(index, cpu)| CpuCoreMetric { name: format!("核心 {}", index + 1), usage_percent: cpu.cpu_usage(), frequency_mhz: cpu.frequency() }).collect(),
            gpus,
            battery: battery_metric(),
            temperatures: self.components.as_ref().map(|components| components.list().iter().filter_map(|item| item.temperature().map(|celsius| TemperatureMetric { label: item.label().into(), celsius, max_celsius: item.max() })).collect()).unwrap_or_default(),
            fans: Vec::<FanMetric>::new(),
            disks: self.disks.list().iter().map(|disk| { let usage = disk.usage(); DiskMetric { name: disk.name().to_string_lossy().into_owned(), mount_point: disk.mount_point().to_string_lossy().into_owned(), total_bytes: disk.total_space(), available_bytes: disk.available_space(), read_bps: usage.read_bytes / self.sampling_interval_seconds().max(1), write_bps: usage.written_bytes / self.sampling_interval_seconds().max(1) } }).collect(),
            networks: self.networks.iter().map(|(name, data)| NetworkMetric { name: name.clone(), received_bps: data.received() / self.sampling_interval_seconds().max(1), transmitted_bps: data.transmitted() / self.sampling_interval_seconds().max(1) }).collect(),
            gpu_status,
            fan_status: "Windows 未向当前进程公开风扇转速传感器".into(),
            app_network_status: if app_network_status.events_lost > 0 {
                format!("{}；已有 {} 个事件丢失，本段数据可能不完整", app_network_status.message, app_network_status.events_lost)
            } else { app_network_status.message },
        };
        let snapshot = SystemSnapshot {
            captured_at: now() * 1000,
            cpu_percent: self.system.global_cpu_usage(),
            memory_percent,
            used_memory_bytes: used,
            total_memory_bytes: total,
            disk_read_bps: disk_read_bytes / 2,
            disk_write_bps: disk_write_bytes / 2,
            network_receive_bps: network_receive_bytes / 2,
            network_send_bps: network_send_bytes / 2,
            disk_total_bytes: self.disks.list().iter().map(|disk| disk.total_space()).sum(),
            disk_available_bytes: self.disks.list().iter().map(|disk| disk.available_space()).sum(),
            uptime_seconds: System::uptime(),
            hardware,
            processes,
            applications,
        };
        self.update_detector(&snapshot);
        self.update_verification(&snapshot);
        if let Some(storage) = &self.storage {
            let _ = storage.save_snapshot(&snapshot, self.settings.retention_days);
        }
        self.snapshot = Some(snapshot);
        let current = now();
        if current.saturating_sub(self.last_patrol_at) >= 10 * 60 {
            self.push_event("patrol", "巡逻状态已刷新，大黄狗正在继续巡逻");
            self.last_patrol_at = current;
        }
        self.pending
            .retain(|_, action| action.preview.expires_at >= now() * 1000);
    }

    fn update_app_lifecycle(&mut self, applications: &[ApplicationGroup]) {
        let current_seconds = now();
        let current_ms = current_seconds * 1000;
        let elapsed = current_seconds
            .saturating_sub(self.last_lifecycle_tick)
            .min(self.sampling_interval_seconds().saturating_mul(2));
        self.last_lifecycle_tick = current_seconds;
        let foreground_pid = foreground_process_id();
        let foreground_key = foreground_pid
            .and_then(|pid| {
                applications
                    .iter()
                    .find(|application| application.members.iter().any(|member| member.pid == pid))
            })
            .map(|application| (application.root_pid, application.root_process.started_at));
        let current_keys: std::collections::HashSet<_> = applications
            .iter()
            .filter(|application| !application.root_process.is_critical)
            .map(|application| (application.root_pid, application.root_process.started_at))
            .collect();
        let mut records = Vec::new();
        for application in applications
            .iter()
            .filter(|application| !application.root_process.is_critical)
        {
            let key = (application.root_pid, application.root_process.started_at);
            let record = self
                .app_sessions
                .entry(key)
                .or_insert_with(|| AppUsageRecord {
                    session_id: format!(
                        "{}-{}",
                        application.root_pid, application.root_process.started_at
                    ),
                    name: application.name.clone(),
                    root_pid: application.root_pid,
                    started_at: application.root_process.started_at * 1000,
                    first_seen_at: current_ms,
                    last_seen_at: current_ms,
                    closed_at: None,
                    runtime_seconds: current_seconds
                        .saturating_sub(application.root_process.started_at),
                    foreground_seconds: 0,
                    background_seconds: 0,
                    member_peak: application.member_count,
                    is_running: true,
                });
            record.last_seen_at = current_ms;
            record.runtime_seconds =
                current_seconds.saturating_sub(application.root_process.started_at);
            record.member_peak = record.member_peak.max(application.member_count);
            if foreground_key == Some(key) {
                record.foreground_seconds = record.foreground_seconds.saturating_add(elapsed);
            }
            record.background_seconds = record
                .runtime_seconds
                .saturating_sub(record.foreground_seconds);
            records.push(record.clone());
        }
        let closed_keys: Vec<_> = self
            .app_sessions
            .keys()
            .filter(|key| !current_keys.contains(key))
            .copied()
            .collect();
        for key in closed_keys {
            if let Some(mut record) = self.app_sessions.remove(&key) {
                record.closed_at = Some(current_ms);
                record.last_seen_at = current_ms;
                record.runtime_seconds = current_seconds.saturating_sub(record.started_at / 1000);
                record.background_seconds = record
                    .runtime_seconds
                    .saturating_sub(record.foreground_seconds);
                record.is_running = false;
                records.push(record);
            }
        }
        if let Some(storage) = &self.storage {
            for record in records {
                let _ = storage.save_app_session(&record);
            }
        }
    }

    pub fn app_usage_history(&self) -> Result<Vec<AppUsageRecord>, String> {
        self.storage
            .as_ref()
            .ok_or("本地记忆暂时不可用".to_string())?
            .recent_app_sessions(2_000)
            .map_err(|error| format!("读取应用使用记录失败：{error}"))
    }

    pub fn app_usage_summary(&self, period_days: u32) -> Result<crate::types::AppUsageSummary, String> {
        self.storage
            .as_ref()
            .ok_or("本地记忆暂时不可用".to_string())?
            .app_usage_summary(period_days.clamp(1, 90))
            .map_err(|error| format!("读取应用使用分析失败：{error}"))
    }

    fn update_detector(&mut self, snapshot: &SystemSnapshot) {
        let required_samples = required_high_samples(self.sampling_interval_seconds());
        self.high_cpu_samples = if snapshot.cpu_percent >= self.settings.cpu_threshold {
            self.high_cpu_samples.saturating_add(1)
        } else {
            0
        };
        self.high_memory_samples = if snapshot.memory_percent >= self.settings.memory_threshold {
            self.high_memory_samples.saturating_add(1)
        } else {
            0
        };
        if self.high_cpu_samples >= required_samples
            && !self.findings.iter().any(|f| f.kind == "cpu.sustained_high")
        {
            let process = snapshot
                .applications
                .first()
                .map(|application| application.root_process.clone());
            self.add_finding(
                "cpu.sustained_high",
                "CPU 一直很忙",
                format!(
                    "CPU 已持续约 1 分钟高于 {:.0}%",
                    self.settings.cpu_threshold
                ),
                snapshot.cpu_percent,
                process,
            );
        }
        if self.high_memory_samples >= required_samples
            && !self.findings.iter().any(|f| f.kind == "memory.pressure")
        {
            let process = snapshot
                .applications
                .iter()
                .max_by_key(|application| application.memory_bytes)
                .map(|application| application.root_process.clone());
            self.add_finding(
                "memory.pressure",
                "电脑内存有点挤",
                format!(
                    "内存已持续约 1 分钟高于 {:.0}%",
                    self.settings.memory_threshold
                ),
                snapshot.memory_percent,
                process,
            );
        }

        if let Some((disk, free_percent, severity)) = lowest_disk_pressure(snapshot) {
            if let Some(finding) = self
                .findings
                .iter_mut()
                .find(|finding| finding.kind == "disk.space_low")
            {
                finding.severity = severity.into();
                finding.message = format!("{} 只剩 {:.1}% 可用空间。", disk.mount_point, free_percent);
            } else {
                self.add_finding_with_severity(
                    "disk.space_low",
                    severity,
                    "磁盘空间快用完了",
                    format!("{} 只剩 {:.1}% 可用空间。", disk.mount_point, free_percent),
                    vec![
                        format!("剩余 {:.1}%", free_percent),
                        format!(
                            "可用 {:.1} GB / 总计 {:.1} GB",
                            disk.available_bytes as f64 / 1024.0 / 1024.0 / 1024.0,
                            disk.total_bytes as f64 / 1024.0 / 1024.0 / 1024.0
                        ),
                        "低于 10% 时会影响更新、缓存和部分应用写入".into(),
                    ],
                    None,
                );
            }
        }

        let disk_bps = snapshot
            .disk_read_bps
            .saturating_add(snapshot.disk_write_bps) as f64;
        let disk_deviation = self.disk_baseline.assess(disk_bps, 20.0 * 1024.0 * 1024.0);
        self.disk_anomaly_samples = if disk_deviation.is_some() {
            self.disk_anomaly_samples.saturating_add(1)
        } else {
            0
        };
        self.disk_baseline.push(disk_bps);
        if self.disk_anomaly_samples >= 3
            && !self
                .findings
                .iter()
                .any(|finding| finding.kind == "disk.io_spike")
        {
            let (median, mad) = disk_deviation.unwrap_or_default();
            self.add_finding_with_evidence(
                "disk.io_spike",
                "磁盘读写突然变忙",
                "磁盘流量连续偏离了近期正常水平。".into(),
                vec![
                    format!("当前 {:.1} MB/s", disk_bps / 1024.0 / 1024.0),
                    format!("滚动中位数 {:.1} MB/s", median / 1024.0 / 1024.0),
                    format!("偏差尺度 MAD {:.1} MB/s", mad / 1024.0 / 1024.0),
                    "已连续确认 3 个样本".into(),
                ],
                snapshot
                    .applications
                    .first()
                    .map(|application| application.root_process.clone()),
            );
        }

        let network_bps = snapshot
            .network_receive_bps
            .saturating_add(snapshot.network_send_bps) as f64;
        let network_deviation = self
            .network_baseline
            .assess(network_bps, 10.0 * 1024.0 * 1024.0);
        self.network_anomaly_samples = if network_deviation.is_some() {
            self.network_anomaly_samples.saturating_add(1)
        } else {
            0
        };
        self.network_baseline.push(network_bps);
        if self.network_anomaly_samples >= 3
            && !self
                .findings
                .iter()
                .any(|finding| finding.kind == "network.traffic_spike")
        {
            let (median, mad) = network_deviation.unwrap_or_default();
            self.add_finding_with_evidence(
                "network.traffic_spike",
                "网络流量突然增大",
                "网络收发连续偏离了近期正常水平。".into(),
                vec![
                    format!("当前 {:.1} MB/s", network_bps / 1024.0 / 1024.0),
                    format!("滚动中位数 {:.1} MB/s", median / 1024.0 / 1024.0),
                    format!("偏差尺度 MAD {:.1} MB/s", mad / 1024.0 / 1024.0),
                    "已连续确认 3 个样本".into(),
                ],
                None,
            );
        }

        let current_time = now();
        let active_keys: std::collections::HashSet<_> = snapshot
            .applications
            .iter()
            .map(|application| (application.root_pid, application.root_process.started_at))
            .collect();
        let mut growth_candidates = Vec::new();
        for application in &snapshot.applications {
            let key = (application.root_pid, application.root_process.started_at);
            let history = self.memory_history.entry(key).or_default();
            history.push_back((current_time, application.memory_bytes));
            while history
                .front()
                .is_some_and(|(captured, _)| current_time.saturating_sub(*captured) > 30 * 60)
            {
                history.pop_front();
            }
            if let Some((growth, duration)) = memory_growth(history) {
                growth_candidates.push((application.clone(), growth, duration));
            }
        }
        self.memory_history
            .retain(|key, _| active_keys.contains(key));
        let growing_keys: std::collections::HashSet<_> = growth_candidates
            .iter()
            .map(|(application, _, _)| (application.root_pid, application.root_process.started_at))
            .collect();
        for (application, growth, duration) in growth_candidates {
            let kind = format!(
                "process.memory_growth:{}:{}",
                application.root_pid, application.root_process.started_at
            );
            if self.findings.iter().any(|finding| finding.kind == kind) {
                continue;
            }
            self.add_finding_with_evidence(
                &kind,
                &format!("{} 的内存持续增长", application.name),
                "不像一次普通波动，可能存在缓存积累或内存泄漏。".into(),
                vec![
                    format!(
                        "{} 分钟增长 {:.1} GB",
                        duration / 60,
                        growth as f64 / 1024.0 / 1024.0 / 1024.0
                    ),
                    format!(
                        "当前应用总内存 {:.1} GB",
                        application.memory_bytes as f64 / 1024.0 / 1024.0 / 1024.0
                    ),
                    format!("包含 {} 个进程", application.member_count),
                ],
                Some(application.root_process),
            );
        }

        let cpu_recovered = snapshot.cpu_percent < self.settings.cpu_threshold - 15.0;
        let memory_recovered = snapshot.memory_percent < self.settings.memory_threshold - 10.0;
        let disk_space_recovered = lowest_disk_pressure(snapshot).is_none();
        let previous_findings = self.findings.clone();
        self.findings.retain(|f| {
            !(f.kind == "cpu.sustained_high" && cpu_recovered
                || f.kind == "memory.pressure" && memory_recovered
                || f.kind == "disk.space_low" && disk_space_recovered
                || f.kind == "disk.io_spike" && self.disk_anomaly_samples == 0
                || f.kind == "network.traffic_spike" && self.network_anomaly_samples == 0
                || f.kind.starts_with("process.memory_growth:")
                    && f.process.as_ref().is_none_or(|process| {
                        !growing_keys.contains(&(process.pid, process.started_at))
                    }))
        });
        if previous_findings.len() > self.findings.len() {
            if let Some(storage) = &self.storage {
                for finding in previous_findings.iter().filter(|old| !self.findings.iter().any(|current| current.id == old.id)) {
                    let _ = storage.resolve_alert(&finding.id, now() * 1000);
                }
            }
            self.push_event("resolved", "刚才的资源压力已经恢复");
        }
    }

    fn add_finding(
        &mut self,
        kind: &str,
        title: &str,
        message: String,
        value: f32,
        process: Option<ProcessSample>,
    ) {
        self.add_finding_with_evidence(
            kind,
            title,
            message,
            vec![format!("当前读数 {:.1}%", value), "已排除短时尖峰".into()],
            process,
        );
    }

    fn add_finding_with_evidence(
        &mut self,
        kind: &str,
        title: &str,
        message: String,
        evidence: Vec<String>,
        process: Option<ProcessSample>,
    ) {
        self.add_finding_with_severity(kind, "warning", title, message, evidence, process);
    }

    fn add_finding_with_severity(
        &mut self,
        kind: &str,
        severity: &str,
        title: &str,
        message: String,
        evidence: Vec<String>,
        process: Option<ProcessSample>,
    ) {
        let current = now();
        if self
            .last_alert_at
            .get(kind)
            .is_some_and(|last| current.saturating_sub(*last) < 10 * 60)
        {
            return;
        }
        self.last_alert_at.insert(kind.into(), current);
        let finding = Finding {
            id: Uuid::new_v4().to_string(),
            kind: kind.into(),
            severity: severity.into(),
            title: title.into(),
            message: message.clone(),
            first_seen_at: now() * 1000,
            evidence,
            process,
        };
        if let Some(storage) = &self.storage { let _ = storage.save_alert(&finding); }
        self.findings.push(finding);
        self.push_event("finding", &format!("发现异常：{title}。正在调查原因"));
    }

    fn push_event(&mut self, kind: &str, message: &str) {
        let event = TimelineEvent {
            id: Uuid::new_v4().to_string(),
            occurred_at: now() * 1000,
            kind: kind.into(),
            message: message.into(),
        };
        if let Some(storage) = &self.storage {
            let _ = storage.save_event(&event);
        }
        self.timeline.push_front(event);
        self.timeline.truncate(50);
    }

    fn update_verification(&mut self, snapshot: &SystemSnapshot) {
        let Some(context) = &self.verification_context else {
            return;
        };
        if now() < context.check_after {
            return;
        }
        let pressure = snapshot.cpu_percent.max(snapshot.memory_percent);
        let target_gone = self
            .system
            .process(Pid::from_u32(context.target_pid))
            .is_none();
        let improved =
            pressure + 10.0 <= context.baseline_pressure || (target_gone && pressure < 85.0);
        let action_id = context.action_id.clone();
        let message = if improved {
            "处理完成后，系统资源压力已经明显下降。"
        } else {
            "目标已经处理，但系统压力没有明显下降，可能还有其他原因。"
        };
        self.verification = Some(VerificationStatus {
            target_name: self
                .verification
                .as_ref()
                .map(|item| item.target_name.clone())
                .unwrap_or_default(),
            status: if improved {
                "improved"
            } else {
                "noImprovement"
            }
            .into(),
            message: message.into(),
            started_at: now() * 1000,
        });
        self.verification_context = None;
        if let Some(storage) = &self.storage {
            let _ = storage.save_verification(&action_id, message);
        }
        self.push_event(if improved { "resolved" } else { "finding" }, message);
    }

    pub fn status(&self) -> CurrentStatus {
        let (dog_state, summary) = if self.findings.is_empty() {
            ("patrol", "我正在巡逻，一切看起来都好。")
        } else {
            ("investigating", "我闻到一点异常，已经找到最可疑的目标。")
        };
        let snapshot = self.snapshot.clone();
        let health_score = snapshot.as_ref().map(snapshot_health_score).unwrap_or(100);
        CurrentStatus {
            dog_state: dog_state.into(),
            summary: summary.into(),
            health_score,
            snapshot,
            findings: self.findings.clone(),
            timeline: self.timeline.iter().cloned().collect(),
            verification: self.verification.clone(),
        }
    }

    pub fn history(&self) -> Result<HistorySummary, String> {
        self.storage
            .as_ref()
            .ok_or("本地记忆暂时不可用".to_string())?
            .history(120)
            .map_err(|error| format!("读取历史失败：{error}"))
    }

    pub fn history_range(&self, range_minutes: u32) -> Result<HistorySummary, String> {
        self.storage.as_ref().ok_or("本地记忆暂时不可用".to_string())?
            .history_range(range_minutes).map_err(|error| format!("读取趋势失败：{error}"))
    }

    pub fn application_history(&self, name: &str, range_minutes: u32) -> Result<ApplicationHistory, String> {
        self.storage.as_ref().ok_or("本地记忆暂时不可用".to_string())?.application_history(name, range_minutes).map_err(|error| format!("读取应用历史失败：{error}"))
    }

    pub fn alerts(&self, status: Option<&str>) -> Result<Vec<AlertRecord>, String> {
        self.storage.as_ref().ok_or("本地记忆暂时不可用".to_string())?.alerts(status).map_err(|error| format!("读取告警失败：{error}"))
    }

    pub fn update_alert(&mut self, id: &str, status: &str, note: &str) -> Result<(), String> {
        self.storage.as_ref().ok_or("本地记忆暂时不可用".to_string())?.update_alert(id, status, note, now() * 1000)
    }

    pub fn periodic_patterns(&self, days: u32) -> Result<Vec<PeriodicPattern>, String> {
        self.storage.as_ref().ok_or("本地记忆暂时不可用".to_string())?.periodic_patterns(days).map_err(|error| format!("分析周期规律失败：{error}"))
    }

    pub fn diagnose(&self) -> LocalDiagnosis {
        let Some(snapshot) = &self.snapshot else {
            return LocalDiagnosis {
                summary: "我还没有收集到足够的数据。".into(),
                details: vec![],
                suggestions: vec!["稍等几秒再试一次".into()],
                confidence: "low".into(),
                source: "local".into(),
                model: None,
            };
        };
        let mut details = vec![
            format!("CPU 当前使用率 {:.0}%", snapshot.cpu_percent),
            format!("内存当前使用率 {:.0}%", snapshot.memory_percent),
        ];
        for application in snapshot.applications.iter().take(3) {
            details.push(format!(
                "{}（{} 个进程）：CPU {:.1}%，内存 {:.1} GB",
                application.name,
                application.member_count,
                application.cpu_percent,
                application.memory_bytes as f64 / 1024.0 / 1024.0 / 1024.0
            ));
        }
        let (summary, suggestions, confidence) = if let Some((disk, free_percent, _)) =
            lowest_disk_pressure(snapshot)
        {
            details.push(format!("{} 只剩 {:.1}% 可用空间", disk.mount_point, free_percent));
            (
                "主人，我找到一个明确问题：磁盘空间快用完了。",
                vec![
                    "先清理下载目录、回收站和不再使用的大文件".into(),
                    "为系统盘保留至少 10% 可用空间".into(),
                ],
                "high",
            )
        } else if snapshot.memory_percent >= 90.0 {
            (
                "主人，我找到原因了：电脑现在主要是内存压力太大。",
                vec![
                    "先保存工作，再检查内存占用最高的普通应用".into(),
                    "不要直接结束 Windows 系统进程".into(),
                ],
                "high",
            )
        } else if snapshot.cpu_percent >= 90.0 {
            (
                "主人，电脑现在主要是 CPU 一直很忙。",
                vec![
                    "观察 CPU 占用最高的进程是否持续异常".into(),
                    "短时尖峰可以先等一会儿".into(),
                ],
                "high",
            )
        } else if snapshot.disk_read_bps + snapshot.disk_write_bps > 100 * 1024 * 1024 {
            (
                "主人，电脑可能正在大量读写磁盘。",
                vec![
                    "等待安装、同步或扫描任务结束".into(),
                    "查看资源占用靠前的进程".into(),
                ],
                "medium",
            )
        } else {
            (
                "我看了一圈，当前没有明显的资源瓶颈。",
                vec!["如果卡顿再次出现，我会继续记录当时的状态".into()],
                "medium",
            )
        };
        LocalDiagnosis {
            summary: summary.into(),
            details,
            suggestions,
            confidence: confidence.into(),
            source: "local".into(),
            model: None,
        }
    }

    pub fn ai_context(&self) -> serde_json::Value {
        let Some(snapshot) = &self.snapshot else { return serde_json::json!({ "status": "尚无采样" }) };
        serde_json::json!({
            "capturedAt": snapshot.captured_at,
            "system": {
                "cpuPercent": snapshot.cpu_percent,
                "memoryPercent": snapshot.memory_percent,
                "usedMemoryBytes": snapshot.used_memory_bytes,
                "totalMemoryBytes": snapshot.total_memory_bytes,
                "diskReadBps": snapshot.disk_read_bps,
                "diskWriteBps": snapshot.disk_write_bps,
                "networkReceiveBps": snapshot.network_receive_bps,
                "networkSendBps": snapshot.network_send_bps,
                "uptimeSeconds": snapshot.uptime_seconds
            },
            "disks": snapshot.hardware.disks.iter().map(|disk| serde_json::json!({
                "mountPoint": disk.mount_point,
                "totalBytes": disk.total_bytes,
                "availableBytes": disk.available_bytes,
                "readBps": disk.read_bps,
                "writeBps": disk.write_bps
            })).collect::<Vec<_>>(),
            "topApplications": snapshot.applications.iter().take(8).map(|application| serde_json::json!({
                "name": application.product_name.as_deref().unwrap_or(&application.name),
                "processCount": application.member_count,
                "cpuPercent": application.cpu_percent,
                "memoryBytes": application.memory_bytes,
                "diskReadBps": application.disk_read_bps,
                "diskWriteBps": application.disk_write_bps,
                "networkBps": application.network_bps
            })).collect::<Vec<_>>(),
            "findings": self.findings.iter().map(|finding| serde_json::json!({
                "severity": finding.severity,
                "title": finding.title,
                "evidence": finding.evidence
            })).collect::<Vec<_>>()
        })
    }

    pub fn settings(&self) -> UserSettings {
        self.settings.clone()
    }

    pub fn sampling_interval_seconds(&self) -> u64 {
        if self.settings.low_power_mode {
            15
        } else {
            self.settings.sampling_seconds.clamp(2, 30)
        }
    }

    pub fn notifications_enabled(&self) -> bool {
        self.settings.notifications_enabled
    }

    pub fn update_settings(&mut self, settings: UserSettings) -> Result<UserSettings, String> {
        if !(70.0..=99.0).contains(&settings.cpu_threshold)
            || !(70.0..=99.0).contains(&settings.memory_threshold)
            || !(2..=30).contains(&settings.sampling_seconds)
            || !(1..=90).contains(&settings.retention_days)
        {
            return Err("设置超出安全范围".into());
        }
        if !matches!(settings.minimax_model.as_str(), "MiniMax-M2.7" | "MiniMax-M2.7-highspeed" | "MiniMax-M2.5" | "MiniMax-M2.5-highspeed") {
            return Err("不支持的 MiniMax 模型".into());
        }
        if !matches!(settings.companion_personality.as_str(), "quiet" | "warm" | "playful") {
            return Err("不支持的小狗性格".into());
        }
        if let Some(storage) = &self.storage {
            storage.save_settings(&settings)?;
        }
        self.network_collector.set_enabled(settings.application_network_monitoring);
        self.settings = settings;
        self.high_cpu_samples = 0;
        self.high_memory_samples = 0;
        self.push_event("patrol", "巡逻设置已经更新");
        Ok(self.settings.clone())
    }

    pub fn shutdown(&mut self) {
        self.network_collector.stop();
    }

    pub fn clear_memory(&mut self) -> Result<(), String> {
        if let Some(storage) = &self.storage {
            storage
                .clear_memory()
                .map_err(|error| format!("清除本地记忆失败：{error}"))?;
        }
        self.timeline.clear();
        self.findings.clear();
        self.push_event("patrol", "旧的本地记忆已经清除，从现在重新开始巡逻");
        Ok(())
    }

    pub fn open_process_location(
        &mut self,
        pid: u32,
        started_at: u64,
    ) -> Result<ActionResult, String> {
        self.system.refresh_processes(ProcessesToUpdate::All, true);
        let process = self
            .system
            .process(Pid::from_u32(pid))
            .ok_or("目标进程已经退出")?;
        if process.start_time() != started_at {
            return Err("目标身份已经变化，请重新查看".into());
        }
        let path = process.exe().ok_or("无法读取这个进程的位置")?;
        let success = std::process::Command::new("explorer.exe")
            .arg(format!("/select,{}", path.to_string_lossy()))
            .spawn()
            .is_ok();
        let message = if success {
            format!("已经帮你定位到 {}。", process.name().to_string_lossy())
        } else {
            "无法打开文件位置。".into()
        };
        Ok(ActionResult {
            action_id: Uuid::new_v4().to_string(),
            success,
            message,
        })
    }

    pub fn prepare_priority(
        &mut self,
        pid: u32,
        started_at: u64,
        level: String,
    ) -> Result<ActionPreview, String> {
        if !matches!(level.as_str(), "belowNormal" | "normal" | "aboveNormal") {
            return Err("不支持的优先级".into());
        }
        self.system.refresh_processes(ProcessesToUpdate::All, true);
        let process = self
            .system
            .process(Pid::from_u32(pid))
            .ok_or("目标进程已经退出")?;
        if process.start_time() != started_at {
            return Err("目标身份已经变化，请重新查看".into());
        }
        let name = process.name().to_string_lossy().into_owned();
        let critical = is_critical(&name);
        let target = ProcessSample {
            pid,
            parent_pid: process.parent().map(|parent| parent.as_u32()),
            started_at,
            name: name.clone(),
            cpu_percent: process.cpu_usage(),
            memory_bytes: process.memory(),
            is_critical: critical,
            thread_count: 0,
            handle_count: process.open_files(),
            disk_read_bps: 0,
            disk_write_bps: 0,
            executable_path: process.exe().map(|path| path.to_string_lossy().into_owned()),
        };
        let label = match level.as_str() {
            "belowNormal" => "低于正常",
            "aboveNormal" => "高于正常",
            _ => "正常",
        };
        let preview = ActionPreview {
            preview_id: Uuid::new_v4().to_string(),
            action: "setProcessPriority".into(),
            risk_level: if critical { "R4" } else { "R1" }.into(),
            allowed: !critical,
            title: if critical {
                "不能调整关键系统进程".into()
            } else {
                format!("将 {name} 的优先级设为{label}？")
            },
            warning: "优先级会影响 Windows 分配 CPU 的顺序，进程重启后通常恢复默认。".into(),
            target,
            expires_at: (now() + PREVIEW_TTL_SECONDS) * 1000,
        };
        self.pending.insert(
            preview.preview_id.clone(),
            PendingAction {
                preview: preview.clone(),
                kind: PendingActionKind::SetPriority(level),
            },
        );
        Ok(preview)
    }

    pub fn prepare_terminate(
        &mut self,
        pid: u32,
        started_at: u64,
    ) -> Result<ActionPreview, String> {
        self.system.refresh_processes(ProcessesToUpdate::All, true);
        let process = self
            .system
            .process(Pid::from_u32(pid))
            .ok_or("目标进程已经退出")?;
        if process.start_time() != started_at {
            return Err("目标身份已经变化，请重新查看后再操作".into());
        }
        let name = process.name().to_string_lossy().into_owned();
        let critical = is_critical(&name);
        let target = ProcessSample {
            pid,
            parent_pid: process.parent().map(|parent| parent.as_u32()),
            started_at,
            name: name.clone(),
            cpu_percent: process.cpu_usage(),
            memory_bytes: process.memory(),
            is_critical: critical,
            thread_count: 0,
            handle_count: process.open_files(),
            disk_read_bps: 0,
            disk_write_bps: 0,
            executable_path: process.exe().map(|path| path.to_string_lossy().into_owned()),
        };
        let preview = ActionPreview {
            preview_id: Uuid::new_v4().to_string(),
            action: "terminateProcess".into(),
            risk_level: if critical { "R4" } else { "R2" }.into(),
            allowed: !critical,
            title: if critical {
                "这个进程不能直接结束".into()
            } else {
                format!("要结束 {name} 吗？")
            },
            warning: if critical {
                "它属于 Windows 关键组件，强行结束可能导致系统不稳定。".into()
            } else {
                "未保存的内容可能丢失。大黄狗只会处理这一个经过校验的进程。".into()
            },
            target,
            expires_at: (now() + PREVIEW_TTL_SECONDS) * 1000,
        };
        self.pending.insert(
            preview.preview_id.clone(),
            PendingAction {
                preview: preview.clone(),
                kind: PendingActionKind::Terminate,
            },
        );
        Ok(preview)
    }

    pub fn confirm_action(&mut self, preview_id: &str) -> Result<ActionResult, String> {
        let pending = self
            .pending
            .remove(preview_id)
            .ok_or("确认已失效，请重新发起操作")?;
        if pending.preview.expires_at < now() * 1000 {
            return Err("确认已过期，请重新检查目标".into());
        }
        if !pending.preview.allowed || pending.preview.target.is_critical {
            return Err("安全策略拒绝了这个操作".into());
        }
        let target = &pending.preview.target;
        self.system.refresh_processes(ProcessesToUpdate::All, true);
        let process = self
            .system
            .process(Pid::from_u32(target.pid))
            .ok_or("目标进程已经退出")?;
        if process.start_time() != target.started_at {
            return Err("目标身份已经变化，操作已取消".into());
        }
        let (success, message) = match &pending.kind {
            PendingActionKind::Terminate => {
                let success = process.kill();
                let message = if success {
                    format!(
                        "已向 {} 发送结束请求，我会继续观察资源是否恢复。",
                        target.name
                    )
                } else {
                    format!("没有成功结束 {}，可能需要管理员权限。", target.name)
                };
                (success, message)
            }
            PendingActionKind::SetPriority(level) => {
                use windows_sys::Win32::{
                    Foundation::CloseHandle,
                    System::Threading::{
                        OpenProcess, SetPriorityClass, ABOVE_NORMAL_PRIORITY_CLASS,
                        BELOW_NORMAL_PRIORITY_CLASS, NORMAL_PRIORITY_CLASS,
                        PROCESS_QUERY_LIMITED_INFORMATION, PROCESS_SET_INFORMATION,
                    },
                };
                let priority = match level.as_str() {
                    "belowNormal" => BELOW_NORMAL_PRIORITY_CLASS,
                    "aboveNormal" => ABOVE_NORMAL_PRIORITY_CLASS,
                    _ => NORMAL_PRIORITY_CLASS,
                };
                let success = unsafe {
                    let handle = OpenProcess(
                        PROCESS_SET_INFORMATION | PROCESS_QUERY_LIMITED_INFORMATION,
                        0,
                        target.pid,
                    );
                    if handle.is_null() {
                        false
                    } else {
                        let changed = SetPriorityClass(handle, priority) != 0;
                        CloseHandle(handle);
                        changed
                    }
                };
                let message = if success {
                    format!("已调整 {} 的优先级。", target.name)
                } else {
                    format!("无法调整 {}，可能需要管理员权限。", target.name)
                };
                (success, message)
            }
        };
        self.push_event("action", &message);
        let action_id = Uuid::new_v4().to_string();
        if let Some(storage) = &self.storage {
            let _ = storage.save_action(
                &action_id,
                now() * 1000,
                &target.name,
                target.pid,
                success,
                &message,
            );
        }
        if success && matches!(pending.kind, PendingActionKind::Terminate) {
            let baseline_pressure = self
                .snapshot
                .as_ref()
                .map(|snapshot| snapshot.cpu_percent.max(snapshot.memory_percent))
                .unwrap_or(0.0);
            self.verification = Some(VerificationStatus {
                target_name: target.name.clone(),
                status: "observing".into(),
                message: "我会观察 15 秒，确认电脑是否真的轻松下来。".into(),
                started_at: now() * 1000,
            });
            self.verification_context = Some(VerificationContext {
                action_id: action_id.clone(),
                target_pid: target.pid,
                baseline_pressure,
                check_after: now() + 15,
            });
        }
        Ok(ActionResult {
            action_id,
            success,
            message,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn critical_process_names_are_case_insensitive() {
        assert!(is_critical("LSASS.EXE"));
        assert!(!is_critical("my-test-worker.exe"));
    }

    #[test]
    fn high_samples_require_a_full_window() {
        let required = required_high_samples(2);
        let mut count = 0_u8;
        for _ in 0..required - 1 {
            count = count.saturating_add(1);
        }
        assert!(count < required);
        count = count.saturating_add(1);
        assert_eq!(count, required);
    }

    #[test]
    fn groups_same_name_child_processes() {
        let sample = |pid, parent_pid, name: &str, cpu, memory| ProcessSample {
            pid,
            parent_pid,
            started_at: 1,
            name: name.into(),
            cpu_percent: cpu,
            memory_bytes: memory,
            is_critical: false,
            thread_count: 1,
            handle_count: Some(1),
            disk_read_bps: 0,
            disk_write_bps: 0,
            executable_path: None,
        };
        let mut network_rates = HashMap::new();
        network_rates.insert(10, NetworkRate { receive_bps: 1_000, send_bps: 200 });
        network_rates.insert(11, NetworkRate { receive_bps: 2_000, send_bps: 300 });
        let groups = build_application_groups(&[
            sample(10, None, "chrome.exe", 5.0, 100),
            sample(11, Some(10), "chrome.exe", 7.0, 200),
            sample(12, Some(11), "chrome.exe", 3.0, 300),
            sample(20, Some(10), "helper.exe", 1.0, 50),
        ], &network_rates, true);
        let chrome = groups
            .iter()
            .find(|group| group.name == "chrome.exe")
            .unwrap();
        assert_eq!(chrome.root_pid, 10);
        assert_eq!(chrome.member_count, 3);
        assert_eq!(chrome.cpu_percent, 15.0);
        assert_eq!(chrome.memory_bytes, 600);
        assert_eq!(chrome.network_receive_bps, Some(3_000));
        assert_eq!(chrome.network_send_bps, Some(500));
        assert_eq!(chrome.network_bps, Some(3_500));
        assert!(groups
            .iter()
            .any(|group| group.name == "helper.exe" && group.member_count == 1));
    }

    #[test]
    fn rolling_baseline_uses_median_and_mad() {
        let mut baseline = RollingBaseline::default();
        for value in 1..=40 {
            baseline.push(1_000_000.0 + (value % 3) as f64 * 10_000.0);
        }
        assert!(baseline.assess(5_000_000.0, 500_000.0).is_some());
        assert!(baseline.assess(1_020_000.0, 500_000.0).is_none());
    }

    #[test]
    fn detects_sustained_application_memory_growth() {
        let mut history = VecDeque::new();
        history.push_back((0, 1024 * 1024 * 1024));
        history.push_back((20 * 60, 3 * 1024 * 1024 * 1024));
        let (growth, duration) = memory_growth(&history).unwrap();
        assert_eq!(growth, 2 * 1024 * 1024 * 1024);
        assert_eq!(duration, 20 * 60);
    }

    #[test]
    fn disk_space_pressure_has_warning_and_critical_levels() {
        assert_eq!(disk_space_pressure(9, 100), Some((9.0, "warning")));
        assert_eq!(disk_space_pressure(4, 100), Some((4.0, "critical")));
        assert_eq!(disk_space_pressure(10, 100), None);
        assert_eq!(disk_space_pressure(0, 0), None);
    }
}
