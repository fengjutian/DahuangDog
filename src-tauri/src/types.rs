use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProcessSample {
    pub pid: u32,
    pub started_at: u64,
    pub name: String,
    pub cpu_percent: f32,
    pub memory_bytes: u64,
    pub is_critical: bool,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SystemSnapshot {
    pub captured_at: u64,
    pub cpu_percent: f32,
    pub memory_percent: f32,
    pub used_memory_bytes: u64,
    pub total_memory_bytes: u64,
    pub processes: Vec<ProcessSample>,
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
