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
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ApplicationGroup {
    pub root_pid: u32,
    pub name: String,
    pub member_count: usize,
    pub cpu_percent: f32,
    pub memory_bytes: u64,
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
    pub processes: Vec<ProcessSample>,
    pub applications: Vec<ApplicationGroup>,
}

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
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UserSettings {
    pub cpu_threshold: f32,
    pub memory_threshold: f32,
    pub sampling_seconds: u64,
    pub low_power_mode: bool,
    pub notifications_enabled: bool,
    pub retention_days: u32,
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
