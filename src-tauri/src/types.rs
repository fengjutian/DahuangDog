use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProcessSample {
    pub pid: u32,
    pub parent_pid: Option<u32>,
    pub started_at: u64,
    pub name: String,
    pub cpu_percent: f32,
    pub memory_bytes: u64,
    pub is_critical: bool,
    pub thread_count: usize,
    pub handle_count: Option<usize>,
    pub disk_read_bps: u64,
    pub disk_write_bps: u64,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ApplicationGroup {
    pub root_pid: u32,
    pub name: String,
    pub member_count: usize,
    pub cpu_percent: f32,
    pub memory_bytes: u64,
    pub disk_read_bps: u64,
    pub disk_write_bps: u64,
    pub network_bps: Option<u64>,
    pub root_process: ProcessSample,
    pub members: Vec<ProcessSample>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SystemSnapshot {
    pub captured_at: u64,
    pub cpu_percent: f32,
    pub memory_percent: f32,
    pub used_memory_bytes: u64,
    pub total_memory_bytes: u64,
    pub disk_read_bps: u64,
    pub disk_write_bps: u64,
    pub network_receive_bps: u64,
    pub network_send_bps: u64,
    pub disk_total_bytes: u64,
    pub disk_available_bytes: u64,
    pub uptime_seconds: u64,
    pub hardware: HardwareSnapshot,
    pub processes: Vec<ProcessSample>,
    pub applications: Vec<ApplicationGroup>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HardwareSnapshot {
    pub cpu_cores: Vec<CpuCoreMetric>,
    pub gpus: Vec<GpuMetric>,
    pub battery: Option<BatteryMetric>,
    pub temperatures: Vec<TemperatureMetric>,
    pub fans: Vec<FanMetric>,
    pub disks: Vec<DiskMetric>,
    pub networks: Vec<NetworkMetric>,
    pub gpu_status: String,
    pub fan_status: String,
    pub app_network_status: String,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CpuCoreMetric { pub name: String, pub usage_percent: f32, pub frequency_mhz: u64 }
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GpuMetric { pub name: String, pub usage_percent: f32, pub memory_used_bytes: u64, pub memory_total_bytes: u64 }
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BatteryMetric { pub charge_percent: u8, pub charging: bool, pub ac_connected: bool, pub life_seconds: Option<u64> }
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TemperatureMetric { pub label: String, pub celsius: f32, pub max_celsius: Option<f32> }
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FanMetric { pub label: String, pub rpm: u32 }
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DiskMetric { pub name: String, pub mount_point: String, pub total_bytes: u64, pub available_bytes: u64, pub read_bps: u64, pub write_bps: u64 }
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NetworkMetric { pub name: String, pub received_bps: u64, pub transmitted_bps: u64 }

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Finding {
    pub id: String,
    pub kind: String,
    pub severity: String,
    pub title: String,
    pub message: String,
    pub first_seen_at: u64,
    pub evidence: Vec<String>,
    pub process: Option<ProcessSample>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TimelineEvent {
    pub id: String,
    pub occurred_at: u64,
    pub kind: String,
    pub message: String,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CurrentStatus {
    pub dog_state: String,
    pub summary: String,
    pub health_score: u8,
    pub snapshot: Option<SystemSnapshot>,
    pub findings: Vec<Finding>,
    pub timeline: Vec<TimelineEvent>,
    pub verification: Option<VerificationStatus>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct VerificationStatus {
    pub target_name: String,
    pub status: String,
    pub message: String,
    pub started_at: u64,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MetricPoint {
    pub captured_at: u64,
    pub cpu_percent: f32,
    pub memory_percent: f32,
    pub disk_bps: u64,
    pub network_bps: u64,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HistorySummary {
    pub points: Vec<MetricPoint>,
    pub baseline_cpu_percent: f32,
    pub baseline_memory_percent: f32,
    pub peak_cpu_percent: f32,
    pub peak_memory_percent: f32,
    pub average_disk_bps: u64,
    pub average_network_bps: u64,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LocalDiagnosis {
    pub summary: String,
    pub details: Vec<String>,
    pub suggestions: Vec<String>,
    pub confidence: String,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProgramRisk {
    pub pid: u32,
    pub name: String,
    pub path: String,
    pub signature_status: String,
    pub risk_level: String,
    pub reasons: Vec<String>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StartupEntry {
    pub name: String,
    pub command: String,
    pub source: String,
    pub risk_level: String,
    pub reasons: Vec<String>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SecurityReport {
    pub scanned_at: u64,
    pub scanned_programs: usize,
    pub signed_programs: usize,
    pub programs: Vec<ProgramRisk>,
    pub startup_entries: Vec<StartupEntry>,
    pub summary: String,
    pub security_score: u8,
    pub medium_risk_count: usize,
    pub low_risk_count: usize,
    pub network_connections: Vec<NetworkConnection>,
    pub scheduled_tasks: Vec<ScheduledTask>,
    pub windows_services: Vec<WindowsService>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NetworkConnection {
    pub protocol: String,
    pub local_address: String,
    pub remote_address: String,
    pub state: String,
    pub pid: u32,
    pub process_name: String,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ScheduledTask { pub name: String, pub path: String }

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WindowsService { pub name: String, pub start_mode: String, pub image_path: String }

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AppUsageSummary {
    pub period_days: u32,
    pub application_count: usize,
    pub session_count: usize,
    pub total_runtime_seconds: u64,
    pub total_foreground_seconds: u64,
    pub total_background_seconds: u64,
    pub longest_used_app: Option<String>,
    pub top_apps: Vec<AppUsageAggregate>,
    pub daily_usage: Vec<DailyUsage>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DailyUsage { pub date: String, pub foreground_seconds: u64, pub launch_count: usize }

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AppUsageAggregate {
    pub name: String,
    pub session_count: usize,
    pub runtime_seconds: u64,
    pub foreground_seconds: u64,
    pub background_seconds: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[serde(default)]
pub struct UserSettings {
    pub cpu_threshold: f32,
    pub memory_threshold: f32,
    pub sampling_seconds: u64,
    pub low_power_mode: bool,
    pub notifications_enabled: bool,
    pub retention_days: u32,
    pub auto_start: bool,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AppUsageRecord {
    pub session_id: String,
    pub name: String,
    pub root_pid: u32,
    pub started_at: u64,
    pub first_seen_at: u64,
    pub last_seen_at: u64,
    pub closed_at: Option<u64>,
    pub runtime_seconds: u64,
    pub foreground_seconds: u64,
    pub background_seconds: u64,
    pub member_peak: usize,
    pub is_running: bool,
}

impl Default for UserSettings {
    fn default() -> Self {
        Self {
            cpu_threshold: 90.0,
            memory_threshold: 90.0,
            sampling_seconds: 2,
            low_power_mode: false,
            notifications_enabled: true,
            retention_days: 7,
            auto_start: false,
        }
    }
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ActionPreview {
    pub preview_id: String,
    pub action: String,
    pub risk_level: String,
    pub allowed: bool,
    pub title: String,
    pub warning: String,
    pub target: ProcessSample,
    pub expires_at: u64,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ActionResult {
    pub action_id: String,
    pub success: bool,
    pub message: String,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ApplicationMetricPoint {
    pub captured_at: u64,
    pub cpu_percent: f32,
    pub memory_bytes: u64,
    pub disk_read_bps: u64,
    pub disk_write_bps: u64,
    pub network_bps: Option<u64>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ApplicationHistory {
    pub name: String,
    pub range_minutes: u32,
    pub points: Vec<ApplicationMetricPoint>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AlertRecord {
    pub id: String,
    pub kind: String,
    pub severity: String,
    pub title: String,
    pub message: String,
    pub first_seen_at: u64,
    pub updated_at: u64,
    pub status: String,
    pub note: String,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PeriodicPattern {
    pub hour: u8,
    pub sample_count: u64,
    pub average_cpu_percent: f32,
    pub average_memory_percent: f32,
    pub signal: String,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CleanupCandidate { pub path: String, pub category: String, pub size_bytes: u64, pub modified_at: u64, pub cleanable: bool }

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CleanupReport { pub scanned_at: u64, pub reclaimable_bytes: u64, pub candidates: Vec<CleanupCandidate> }

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MaintenancePreview { pub preview_id: String, pub title: String, pub warning: String, pub item_count: usize, pub total_bytes: u64, pub expires_at: u64 }
