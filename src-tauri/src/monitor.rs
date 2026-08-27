use crate::storage::Storage;
use crate::types::{
    ActionPreview, ActionResult, AppUsageRecord, ApplicationGroup, CurrentStatus, Finding,
    BatteryMetric, CpuCoreMetric, DiskMetric, FanMetric, GpuMetric, HardwareSnapshot,
    HistorySummary, LocalDiagnosis, NetworkMetric, ProcessSample, SystemSnapshot, TemperatureMetric,
    TimelineEvent, UserSettings, VerificationStatus,
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

fn build_application_groups(processes: &[ProcessSample]) -> Vec<ApplicationGroup> {
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
            Some(ApplicationGroup {
                root_pid,
                name: root_process.name.clone(),
                member_count,
                cpu_percent,
                memory_bytes,
                disk_read_bps,
                disk_write_bps,
                network_bps: None,
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
        let mut monitor = Self {
            system: System::new_all(),
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
                }
            })
            .collect();
        processes.sort_by(|a, b| {
            let a_score = a.cpu_percent as f64 * 100_000_000.0 + a.memory_bytes as f64;
            let b_score = b.cpu_percent as f64 * 100_000_000.0 + b.memory_bytes as f64;
            b_score.total_cmp(&a_score)
        });
        let all_applications = build_application_groups(&processes);
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
            app_network_status: "Windows 需要 ETW 会话才能可靠归属应用网络流量，当前版本暂不伪造数据".into(),
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
        let before = self.findings.len();
        self.findings.retain(|f| {
            !(f.kind == "cpu.sustained_high" && cpu_recovered
                || f.kind == "memory.pressure" && memory_recovered
                || f.kind == "disk.io_spike" && self.disk_anomaly_samples == 0
                || f.kind == "network.traffic_spike" && self.network_anomaly_samples == 0
                || f.kind.starts_with("process.memory_growth:")
                    && f.process.as_ref().is_none_or(|process| {
                        !growing_keys.contains(&(process.pid, process.started_at))
                    }))
        });
        if before > self.findings.len() {
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
        let current = now();
        if self
            .last_alert_at
            .get(kind)
            .is_some_and(|last| current.saturating_sub(*last) < 10 * 60)
        {
            return;
        }
        self.last_alert_at.insert(kind.into(), current);
        self.findings.push(Finding {
            id: Uuid::new_v4().to_string(),
            kind: kind.into(),
            severity: "warning".into(),
            title: title.into(),
            message: message.clone(),
            first_seen_at: now() * 1000,
            evidence,
            process,
        });
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
        let pressure = snapshot
            .as_ref()
            .map(|s| s.cpu_percent.max(s.memory_percent))
            .unwrap_or(0.0);
        CurrentStatus {
            dog_state: dog_state.into(),
            summary: summary.into(),
            health_score: (100.0 - pressure * 0.55).clamp(0.0, 100.0) as u8,
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

    pub fn diagnose(&self) -> LocalDiagnosis {
        let Some(snapshot) = &self.snapshot else {
            return LocalDiagnosis {
                summary: "我还没有收集到足够的数据。".into(),
                details: vec![],
                suggestions: vec!["稍等几秒再试一次".into()],
                confidence: "low".into(),
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
        let (summary, suggestions, confidence) = if snapshot.memory_percent >= 90.0 {
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
        }
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
        if let Some(storage) = &self.storage {
            storage.save_settings(&settings)?;
        }
        self.settings = settings;
        self.high_cpu_samples = 0;
        self.high_memory_samples = 0;
        self.push_event("patrol", "巡逻设置已经更新");
        Ok(self.settings.clone())
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
        };
        let groups = build_application_groups(&[
            sample(10, None, "chrome.exe", 5.0, 100),
            sample(11, Some(10), "chrome.exe", 7.0, 200),
            sample(12, Some(11), "chrome.exe", 3.0, 300),
            sample(20, Some(10), "helper.exe", 1.0, 50),
        ]);
        let chrome = groups
            .iter()
            .find(|group| group.name == "chrome.exe")
            .unwrap();
        assert_eq!(chrome.root_pid, 10);
        assert_eq!(chrome.member_count, 3);
        assert_eq!(chrome.cpu_percent, 15.0);
        assert_eq!(chrome.memory_bytes, 600);
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
}
