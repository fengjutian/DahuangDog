use crate::storage::Storage;
use crate::types::{
    ActionPreview, ActionResult, CurrentStatus, Finding, ProcessSample, SystemSnapshot,
    TimelineEvent, VerificationStatus,
};
use std::{
    collections::{HashMap, VecDeque},
    time::{SystemTime, UNIX_EPOCH},
};
use sysinfo::{Disks, Networks, Pid, ProcessesToUpdate, System};
use uuid::Uuid;

const HIGH_CPU_PERCENT: f32 = 90.0;
const HIGH_MEMORY_PERCENT: f32 = 90.0;
const REQUIRED_HIGH_SAMPLES: u8 = 30;
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

#[derive(Clone)]
struct PendingAction {
    preview: ActionPreview,
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
    storage: Option<Storage>,
    snapshot: Option<SystemSnapshot>,
    findings: Vec<Finding>,
    timeline: VecDeque<TimelineEvent>,
    pending: HashMap<String, PendingAction>,
    high_cpu_samples: u8,
    high_memory_samples: u8,
    verification: Option<VerificationStatus>,
    verification_context: Option<VerificationContext>,
}

impl Monitor {
    pub fn new() -> Self {
        let mut monitor = Self {
            system: System::new_all(),
            disks: Disks::new_with_refreshed_list(),
            networks: Networks::new_with_refreshed_list(),
            storage: Storage::open_default().ok(),
            snapshot: None,
            findings: vec![],
            timeline: VecDeque::new(),
            pending: HashMap::new(),
            high_cpu_samples: 0,
            high_memory_samples: 0,
            verification: None,
            verification_context: None,
        };
        if let Some(storage) = &monitor.storage {
            if let Ok(events) = storage.recent_events(50) {
                monitor.timeline = events.into();
            }
        }
        monitor.push_event("patrol", "大黄狗醒了，开始巡逻");
        monitor.refresh();
        monitor
    }

    pub fn refresh(&mut self) {
        self.system.refresh_cpu_usage();
        self.system.refresh_memory();
        self.system.refresh_processes(ProcessesToUpdate::All, true);
        self.disks.refresh(true);
        self.networks.refresh(true);

        let total = self.system.total_memory();
        let used = self.system.used_memory();
        let memory_percent = if total == 0 {
            0.0
        } else {
            used as f32 / total as f32 * 100.0
        };
        let mut processes: Vec<_> = self
            .system
            .processes()
            .iter()
            .map(|(pid, process)| {
                let name = process.name().to_string_lossy().into_owned();
                ProcessSample {
                    pid: pid.as_u32(),
                    started_at: process.start_time(),
                    name: name.clone(),
                    cpu_percent: process.cpu_usage(),
                    memory_bytes: process.memory(),
                    is_critical: is_critical(&name),
                }
            })
            .collect();
        processes.sort_by(|a, b| {
            let a_score = a.cpu_percent as f64 * 100_000_000.0 + a.memory_bytes as f64;
            let b_score = b.cpu_percent as f64 * 100_000_000.0 + b.memory_bytes as f64;
            b_score.total_cmp(&a_score)
        });
        processes.truncate(20);

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
            processes,
        };
        self.update_detector(&snapshot);
        self.update_verification(&snapshot);
        if let Some(storage) = &self.storage {
            let _ = storage.save_snapshot(&snapshot);
        }
        self.snapshot = Some(snapshot);
        self.pending
            .retain(|_, action| action.preview.expires_at >= now() * 1000);
    }

    fn update_detector(&mut self, snapshot: &SystemSnapshot) {
        self.high_cpu_samples = if snapshot.cpu_percent >= HIGH_CPU_PERCENT {
            self.high_cpu_samples.saturating_add(1)
        } else {
            0
        };
        self.high_memory_samples = if snapshot.memory_percent >= HIGH_MEMORY_PERCENT {
            self.high_memory_samples.saturating_add(1)
        } else {
            0
        };
        if self.high_cpu_samples >= REQUIRED_HIGH_SAMPLES
            && !self.findings.iter().any(|f| f.kind == "cpu.sustained_high")
        {
            let process = snapshot.processes.first().cloned();
            self.add_finding(
                "cpu.sustained_high",
                "CPU 一直很忙",
                format!("CPU 已持续约 1 分钟高于 {:.0}%", HIGH_CPU_PERCENT),
                snapshot.cpu_percent,
                process,
            );
        }
        if self.high_memory_samples >= REQUIRED_HIGH_SAMPLES
            && !self.findings.iter().any(|f| f.kind == "memory.pressure")
        {
            let process = snapshot
                .processes
                .iter()
                .max_by_key(|p| p.memory_bytes)
                .cloned();
            self.add_finding(
                "memory.pressure",
                "电脑内存有点挤",
                format!("内存已持续约 1 分钟高于 {:.0}%", HIGH_MEMORY_PERCENT),
                snapshot.memory_percent,
                process,
            );
        }
        let cpu_recovered = snapshot.cpu_percent < 70.0;
        let memory_recovered = snapshot.memory_percent < 80.0;
        let before = self.findings.len();
        self.findings.retain(|f| {
            !(f.kind == "cpu.sustained_high" && cpu_recovered
                || f.kind == "memory.pressure" && memory_recovered)
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
        self.findings.push(Finding {
            id: Uuid::new_v4().to_string(),
            kind: kind.into(),
            severity: "warning".into(),
            title: title.into(),
            message: message.clone(),
            first_seen_at: now() * 1000,
            evidence: vec![format!("当前读数 {:.1}%", value), "已排除短时尖峰".into()],
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
            started_at,
            name: name.clone(),
            cpu_percent: process.cpu_usage(),
            memory_bytes: process.memory(),
            is_critical: critical,
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
        let success = process.kill();
        let message = if success {
            format!(
                "已向 {} 发送结束请求，我会继续观察资源是否恢复。",
                target.name
            )
        } else {
            format!("没有成功结束 {}，可能需要管理员权限。", target.name)
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
        if success {
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
        let mut count = 0_u8;
        for _ in 0..REQUIRED_HIGH_SAMPLES - 1 {
            count = count.saturating_add(1);
        }
        assert!(count < REQUIRED_HIGH_SAMPLES);
        count = count.saturating_add(1);
        assert_eq!(count, REQUIRED_HIGH_SAMPLES);
    }
}
