use crate::types::{
    AlertRecord, AppUsageAggregate, AppUsageRecord, AppUsageSummary, ApplicationHistory,
    ApplicationMetricPoint, DailyUsage, Finding, HistorySummary, MetricPoint, PeriodicPattern,
    SystemSnapshot, TimelineEvent, UserSettings,
};
use rusqlite::{params, Connection};
use std::{fs, path::PathBuf};

pub struct Storage {
    connection: Connection,
}

impl Storage {
    pub fn open_default() -> Result<Self, String> {
        let root = std::env::var_os("LOCALAPPDATA")
            .map(PathBuf::from)
            .unwrap_or_else(std::env::temp_dir)
            .join("DahuangDog");
        fs::create_dir_all(&root).map_err(|error| format!("无法创建数据目录：{error}"))?;
        Self::open(root.join("memory.db"))
    }

    fn open(path: PathBuf) -> Result<Self, String> {
        let connection =
            Connection::open(path).map_err(|error| format!("无法打开本地记忆：{error}"))?;
        connection
            .execute_batch(
                "PRAGMA journal_mode=WAL;
             PRAGMA foreign_keys=ON;
             CREATE TABLE IF NOT EXISTS system_snapshots (
                id INTEGER PRIMARY KEY,
                captured_at INTEGER NOT NULL,
                cpu_percent REAL NOT NULL,
                memory_percent REAL NOT NULL,
                used_memory_bytes INTEGER NOT NULL,
                total_memory_bytes INTEGER NOT NULL,
                disk_read_bps INTEGER NOT NULL,
                disk_write_bps INTEGER NOT NULL,
                network_receive_bps INTEGER NOT NULL,
                network_send_bps INTEGER NOT NULL
             );
             CREATE INDEX IF NOT EXISTS idx_snapshots_time ON system_snapshots(captured_at);
             CREATE TABLE IF NOT EXISTS domain_events (
                id TEXT PRIMARY KEY,
                occurred_at INTEGER NOT NULL,
                kind TEXT NOT NULL,
                message TEXT NOT NULL
             );
             CREATE INDEX IF NOT EXISTS idx_events_time ON domain_events(occurred_at DESC);
             CREATE TABLE IF NOT EXISTS action_audits (
                id TEXT PRIMARY KEY,
                occurred_at INTEGER NOT NULL,
                process_name TEXT NOT NULL,
                pid INTEGER NOT NULL,
                success INTEGER NOT NULL,
                result TEXT NOT NULL,
                verification TEXT
             );
             CREATE TABLE IF NOT EXISTS app_settings (
                id INTEGER PRIMARY KEY CHECK (id = 1),
                value_json TEXT NOT NULL
             );
             CREATE TABLE IF NOT EXISTS app_sessions (
                session_id TEXT PRIMARY KEY,
                name TEXT NOT NULL,
                root_pid INTEGER NOT NULL,
                started_at INTEGER NOT NULL,
                first_seen_at INTEGER NOT NULL,
                last_seen_at INTEGER NOT NULL,
                closed_at INTEGER,
                runtime_seconds INTEGER NOT NULL,
                foreground_seconds INTEGER NOT NULL,
                background_seconds INTEGER NOT NULL,
                member_peak INTEGER NOT NULL,
                is_running INTEGER NOT NULL
             );
             CREATE TABLE IF NOT EXISTS application_snapshots (
                id INTEGER PRIMARY KEY,
                captured_at INTEGER NOT NULL,
                app_name TEXT NOT NULL,
                root_pid INTEGER NOT NULL,
                cpu_percent REAL NOT NULL,
                memory_bytes INTEGER NOT NULL,
                disk_read_bps INTEGER NOT NULL,
                disk_write_bps INTEGER NOT NULL,
                network_bps INTEGER
             );
             CREATE INDEX IF NOT EXISTS idx_app_snapshots_name_time ON application_snapshots(app_name, captured_at);
             CREATE TABLE IF NOT EXISTS alerts (
                id TEXT PRIMARY KEY,
                kind TEXT NOT NULL,
                severity TEXT NOT NULL,
                title TEXT NOT NULL,
                message TEXT NOT NULL,
                first_seen_at INTEGER NOT NULL,
                updated_at INTEGER NOT NULL,
                status TEXT NOT NULL DEFAULT 'unread',
                note TEXT NOT NULL DEFAULT ''
             );
             CREATE INDEX IF NOT EXISTS idx_alerts_time ON alerts(updated_at DESC);
             CREATE TABLE IF NOT EXISTS storage_directory_index (
                path TEXT NOT NULL,
                exact INTEGER NOT NULL,
                modified_at INTEGER NOT NULL,
                indexed_at INTEGER NOT NULL,
                result_json TEXT NOT NULL,
                PRIMARY KEY(path, exact)
             );",
            )
            .map_err(|error| format!("无法初始化本地记忆：{error}"))?;
        Ok(Self { connection })
    }

    pub fn save_snapshot(
        &self,
        snapshot: &SystemSnapshot,
        retention_days: u32,
    ) -> rusqlite::Result<()> {
        self.connection.execute(
            "INSERT INTO system_snapshots (
                captured_at, cpu_percent, memory_percent, used_memory_bytes,
                total_memory_bytes, disk_read_bps, disk_write_bps,
                network_receive_bps, network_send_bps
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
            params![
                snapshot.captured_at,
                snapshot.cpu_percent,
                snapshot.memory_percent,
                snapshot.used_memory_bytes,
                snapshot.total_memory_bytes,
                snapshot.disk_read_bps,
                snapshot.disk_write_bps,
                snapshot.network_receive_bps,
                snapshot.network_send_bps
            ],
        )?;
        for application in &snapshot.applications {
            self.connection.execute(
                "INSERT INTO application_snapshots (
                    captured_at, app_name, root_pid, cpu_percent, memory_bytes,
                    disk_read_bps, disk_write_bps, network_bps
                 ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
                params![snapshot.captured_at, application.name, application.root_pid,
                    application.cpu_percent, application.memory_bytes, application.disk_read_bps,
                    application.disk_write_bps, application.network_bps],
            )?;
        }
        self.connection.execute(
            "DELETE FROM system_snapshots WHERE captured_at < ?1",
            params![snapshot
                .captured_at
                .saturating_sub(retention_days as u64 * 24 * 60 * 60 * 1000)],
        )?;
        self.connection.execute(
            "DELETE FROM application_snapshots WHERE captured_at < ?1",
            params![snapshot.captured_at.saturating_sub(retention_days as u64 * 86_400_000)],
        )?;
        Ok(())
    }

    pub fn application_history(&self, name: &str, range_minutes: u32) -> rusqlite::Result<ApplicationHistory> {
        let now_ms = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_millis() as u64;
        let range_minutes = range_minutes.clamp(1, 10_080);
        let since = now_ms.saturating_sub(range_minutes as u64 * 60_000);
        let bucket = (range_minutes as u64 * 60_000 / 300).max(2_000);
        let mut statement = self.connection.prepare(
            "SELECT MAX(captured_at), AVG(cpu_percent), CAST(AVG(memory_bytes) AS INTEGER),
                    CAST(AVG(disk_read_bps) AS INTEGER), CAST(AVG(disk_write_bps) AS INTEGER),
                    CASE WHEN COUNT(network_bps) > 0 THEN CAST(AVG(network_bps) AS INTEGER) END
             FROM application_snapshots WHERE lower(app_name) = lower(?1) AND captured_at >= ?2
             GROUP BY captured_at / ?3 ORDER BY 1")?;
        let points = statement.query_map(params![name, since, bucket], |row| Ok(ApplicationMetricPoint {
            captured_at: row.get(0)?, cpu_percent: row.get(1)?, memory_bytes: row.get(2)?,
            disk_read_bps: row.get(3)?, disk_write_bps: row.get(4)?, network_bps: row.get(5)?,
        }))?.collect::<rusqlite::Result<_>>()?;
        Ok(ApplicationHistory { name: name.into(), range_minutes, points })
    }

    pub fn save_alert(&self, finding: &Finding) -> rusqlite::Result<()> {
        self.connection.execute(
            "INSERT OR IGNORE INTO alerts (id, kind, severity, title, message, first_seen_at, updated_at, status)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?6, 'unread')",
            params![finding.id, finding.kind, finding.severity, finding.title, finding.message, finding.first_seen_at],
        )?;
        Ok(())
    }

    pub fn resolve_alert(&self, id: &str, updated_at: u64) -> rusqlite::Result<()> {
        self.connection.execute("UPDATE alerts SET status = 'resolved', updated_at = ?2 WHERE id = ?1 AND status != 'ignored'", params![id, updated_at])?;
        Ok(())
    }

    pub fn alerts(&self, status: Option<&str>) -> rusqlite::Result<Vec<AlertRecord>> {
        let mut statement = self.connection.prepare(
            "SELECT id, kind, severity, title, message, first_seen_at, updated_at, status, note
             FROM alerts WHERE ?1 IS NULL OR status = ?1 ORDER BY updated_at DESC LIMIT 500")?;
        let alerts = statement.query_map([status], |row| Ok(AlertRecord { id: row.get(0)?, kind: row.get(1)?, severity: row.get(2)?, title: row.get(3)?, message: row.get(4)?, first_seen_at: row.get(5)?, updated_at: row.get(6)?, status: row.get(7)?, note: row.get(8)? }))?.collect();
        alerts
    }

    pub fn update_alert(&self, id: &str, status: &str, note: &str, updated_at: u64) -> Result<(), String> {
        if !matches!(status, "unread" | "acknowledged" | "ignored" | "resolved") { return Err("未知告警状态".into()); }
        if note.chars().count() > 500 { return Err("处理备注不能超过 500 字".into()); }
        let changed = self.connection.execute("UPDATE alerts SET status = ?2, note = ?3, updated_at = ?4 WHERE id = ?1", params![id, status, note, updated_at]).map_err(|e| e.to_string())?;
        if changed == 0 { return Err("告警不存在".into()); }
        Ok(())
    }

    pub fn periodic_patterns(&self, days: u32) -> rusqlite::Result<Vec<PeriodicPattern>> {
        let now_ms = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_millis() as u64;
        let since = now_ms.saturating_sub(days.clamp(1, 90) as u64 * 86_400_000);
        let mut statement = self.connection.prepare(
            "SELECT CAST(strftime('%H', captured_at / 1000, 'unixepoch', 'localtime') AS INTEGER), COUNT(*), AVG(cpu_percent), AVG(memory_percent)
             FROM system_snapshots WHERE captured_at >= ?1 GROUP BY 1 HAVING COUNT(*) >= 10 ORDER BY 1")?;
        let rows = statement.query_map([since], |row| {
            let cpu: f32 = row.get(2)?; let memory: f32 = row.get(3)?;
            Ok(PeriodicPattern { hour: row.get(0)?, sample_count: row.get(1)?, average_cpu_percent: cpu, average_memory_percent: memory, signal: if cpu >= 70.0 && memory >= 80.0 { "CPU 与内存周期性偏高" } else if cpu >= 70.0 { "CPU 周期性偏高" } else if memory >= 80.0 { "内存周期性偏高" } else { "正常时段" }.into() })
        })?.collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows.into_iter().filter(|row| row.signal != "正常时段").collect())
    }

    pub fn save_event(&self, event: &TimelineEvent) -> rusqlite::Result<()> {
        self.connection.execute(
            "INSERT OR IGNORE INTO domain_events (id, occurred_at, kind, message) VALUES (?1, ?2, ?3, ?4)",
            params![event.id, event.occurred_at, event.kind, event.message],
        )?;
        Ok(())
    }

    pub fn recent_events(&self, limit: u32) -> rusqlite::Result<Vec<TimelineEvent>> {
        let mut statement = self.connection.prepare(
            "SELECT id, occurred_at, kind, message FROM domain_events ORDER BY occurred_at DESC LIMIT ?1",
        )?;
        let events = statement
            .query_map([limit], |row| {
                Ok(TimelineEvent {
                    id: row.get(0)?,
                    occurred_at: row.get(1)?,
                    kind: row.get(2)?,
                    message: row.get(3)?,
                })
            })?
            .collect();
        events
    }

    pub fn save_action(
        &self,
        id: &str,
        occurred_at: u64,
        process_name: &str,
        pid: u32,
        success: bool,
        result: &str,
    ) -> rusqlite::Result<()> {
        self.connection.execute(
            "INSERT INTO action_audits (id, occurred_at, process_name, pid, success, result) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![id, occurred_at, process_name, pid, success, result],
        )?;
        Ok(())
    }

    pub fn save_verification(&self, action_id: &str, verification: &str) -> rusqlite::Result<()> {
        self.connection.execute(
            "UPDATE action_audits SET verification = ?2 WHERE id = ?1",
            params![action_id, verification],
        )?;
        Ok(())
    }

    pub fn history(&self, limit: u32) -> rusqlite::Result<HistorySummary> {
        let mut statement = self.connection.prepare(
            "SELECT captured_at, cpu_percent, memory_percent,
                    disk_read_bps + disk_write_bps,
                    network_receive_bps + network_send_bps
             FROM system_snapshots ORDER BY captured_at DESC LIMIT ?1",
        )?;
        let mut points: Vec<MetricPoint> = statement
            .query_map([limit], |row| {
                Ok(MetricPoint {
                    captured_at: row.get(0)?,
                    cpu_percent: row.get(1)?,
                    memory_percent: row.get(2)?,
                    disk_bps: row.get(3)?,
                    network_bps: row.get(4)?,
                })
            })?
            .collect::<rusqlite::Result<_>>()?;
        points.reverse();
        let since = points
            .last()
            .map(|point| point.captured_at.saturating_sub(24 * 60 * 60 * 1000))
            .unwrap_or(0);
        let (cpu, memory) = self.connection.query_row(
            "SELECT COALESCE(AVG(cpu_percent), 0), COALESCE(AVG(memory_percent), 0)
             FROM system_snapshots WHERE captured_at >= ?1",
            [since],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )?;
        let peak_cpu_percent = points.iter().map(|point| point.cpu_percent).fold(0.0, f32::max);
        let peak_memory_percent = points.iter().map(|point| point.memory_percent).fold(0.0, f32::max);
        let sample_count = points.len().max(1) as u64;
        let average_disk_bps = points.iter().map(|point| point.disk_bps).sum::<u64>() / sample_count;
        let average_network_bps = points.iter().map(|point| point.network_bps).sum::<u64>() / sample_count;
        Ok(HistorySummary {
            points,
            baseline_cpu_percent: cpu,
            baseline_memory_percent: memory,
            peak_cpu_percent,
            peak_memory_percent,
            average_disk_bps,
            average_network_bps,
        })
    }

    pub fn history_range(&self, range_minutes: u32) -> rusqlite::Result<HistorySummary> {
        let now_ms = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_millis() as u64;
        let since = now_ms.saturating_sub(range_minutes.clamp(1, 10_080) as u64 * 60_000);
        let bucket = (range_minutes as u64 * 60_000 / 300).max(2_000);
        let mut statement = self.connection.prepare(
            "SELECT MAX(captured_at), AVG(cpu_percent), AVG(memory_percent),
                    CAST(AVG(disk_read_bps + disk_write_bps) AS INTEGER),
                    CAST(AVG(network_receive_bps + network_send_bps) AS INTEGER)
             FROM system_snapshots WHERE captured_at >= ?1 GROUP BY captured_at / ?2 ORDER BY 1",
        )?;
        let points: Vec<MetricPoint> = statement.query_map(params![since, bucket], |row| Ok(MetricPoint {
            captured_at: row.get(0)?, cpu_percent: row.get(1)?, memory_percent: row.get(2)?,
            disk_bps: row.get(3)?, network_bps: row.get(4)?,
        }))?.collect::<rusqlite::Result<_>>()?;
        let count = points.len().max(1) as u64;
        Ok(HistorySummary {
            baseline_cpu_percent: points.iter().map(|p| p.cpu_percent).sum::<f32>() / count as f32,
            baseline_memory_percent: points.iter().map(|p| p.memory_percent).sum::<f32>() / count as f32,
            peak_cpu_percent: points.iter().map(|p| p.cpu_percent).fold(0.0, f32::max),
            peak_memory_percent: points.iter().map(|p| p.memory_percent).fold(0.0, f32::max),
            average_disk_bps: points.iter().map(|p| p.disk_bps).sum::<u64>() / count,
            average_network_bps: points.iter().map(|p| p.network_bps).sum::<u64>() / count,
            points,
        })
    }

    pub fn load_settings(&self) -> UserSettings {
        self.connection
            .query_row(
                "SELECT value_json FROM app_settings WHERE id = 1",
                [],
                |row| row.get::<_, String>(0),
            )
            .ok()
            .and_then(|value| serde_json::from_str(&value).ok())
            .unwrap_or_default()
    }

    pub fn save_settings(&self, settings: &UserSettings) -> Result<(), String> {
        let json = serde_json::to_string(settings).map_err(|error| error.to_string())?;
        self.connection
            .execute(
                "INSERT INTO app_settings (id, value_json) VALUES (1, ?1)
             ON CONFLICT(id) DO UPDATE SET value_json = excluded.value_json",
                [json],
            )
            .map_err(|error| error.to_string())?;
        Ok(())
    }

    pub fn clear_memory(&self) -> rusqlite::Result<()> {
        self.connection.execute_batch(
            "DELETE FROM system_snapshots; DELETE FROM domain_events; DELETE FROM action_audits; DELETE FROM app_sessions; DELETE FROM application_snapshots; DELETE FROM alerts; DELETE FROM storage_directory_index;",
        )
    }

    pub fn close_stale_app_sessions(&self, closed_at: u64) -> rusqlite::Result<()> {
        self.connection.execute(
            "UPDATE app_sessions SET closed_at = last_seen_at, is_running = 0,
                runtime_seconds = MAX(runtime_seconds, (last_seen_at - first_seen_at) / 1000),
                background_seconds = MAX(0, runtime_seconds - foreground_seconds)
             WHERE is_running = 1",
            [],
        )?;
        let _ = closed_at;
        Ok(())
    }

    pub fn save_app_session(&self, record: &AppUsageRecord) -> rusqlite::Result<()> {
        self.connection.execute(
            "INSERT INTO app_sessions (
                session_id, name, root_pid, started_at, first_seen_at, last_seen_at,
                closed_at, runtime_seconds, foreground_seconds, background_seconds,
                member_peak, is_running
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)
             ON CONFLICT(session_id) DO UPDATE SET
                last_seen_at = excluded.last_seen_at, closed_at = excluded.closed_at,
                runtime_seconds = excluded.runtime_seconds,
                foreground_seconds = excluded.foreground_seconds,
                background_seconds = excluded.background_seconds,
                member_peak = excluded.member_peak, is_running = excluded.is_running",
            params![
                record.session_id,
                record.name,
                record.root_pid,
                record.started_at,
                record.first_seen_at,
                record.last_seen_at,
                record.closed_at,
                record.runtime_seconds,
                record.foreground_seconds,
                record.background_seconds,
                record.member_peak,
                record.is_running
            ],
        )?;
        Ok(())
    }

    pub fn recent_app_sessions(&self, limit: u32) -> rusqlite::Result<Vec<AppUsageRecord>> {
        let mut statement = self.connection.prepare(
            "SELECT session_id, name, root_pid, started_at, first_seen_at, last_seen_at,
                    closed_at,
                    CASE WHEN runtime_seconds <= 31536000 THEN runtime_seconds ELSE MAX(0, (last_seen_at - first_seen_at) / 1000) END,
                    foreground_seconds,
                    MIN(background_seconds, MAX(0, (last_seen_at - first_seen_at) / 1000)),
                    member_peak, is_running
             FROM app_sessions ORDER BY is_running DESC, last_seen_at DESC LIMIT ?1",
        )?;
        let records = statement
            .query_map([limit], |row| {
                Ok(AppUsageRecord {
                    session_id: row.get(0)?,
                    name: row.get(1)?,
                    root_pid: row.get(2)?,
                    started_at: row.get(3)?,
                    first_seen_at: row.get(4)?,
                    last_seen_at: row.get(5)?,
                    closed_at: row.get(6)?,
                    runtime_seconds: row.get(7)?,
                    foreground_seconds: row.get(8)?,
                    background_seconds: row.get(9)?,
                    member_peak: row.get(10)?,
                    is_running: row.get(11)?,
                })
            })?
            .collect();
        records
    }

    pub fn app_usage_summary(&self, period_days: u32) -> rusqlite::Result<AppUsageSummary> {
        let since = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as u64
            - period_days as u64 * 24 * 60 * 60 * 1000;
        let mut statement = self.connection.prepare(
            "SELECT name, COUNT(*),
                    SUM(CASE WHEN runtime_seconds <= 31536000 THEN runtime_seconds ELSE MAX(0, (last_seen_at - first_seen_at) / 1000) END),
                    SUM(foreground_seconds),
                    SUM(MIN(background_seconds, MAX(0, (last_seen_at - first_seen_at) / 1000)))
             FROM app_sessions WHERE last_seen_at >= ?1 AND foreground_seconds > 0
             GROUP BY LOWER(name) ORDER BY SUM(foreground_seconds) DESC",
        )?;
        let top_apps: Vec<AppUsageAggregate> = statement
            .query_map([since], |row| {
                Ok(AppUsageAggregate {
                    name: row.get(0)?,
                    session_count: row.get(1)?,
                    runtime_seconds: row.get(2)?,
                    foreground_seconds: row.get(3)?,
                    background_seconds: row.get(4)?,
                })
            })?
            .collect::<rusqlite::Result<_>>()?;
        let mut daily_statement = self.connection.prepare(
            "SELECT strftime('%Y-%m-%d', last_seen_at / 1000, 'unixepoch', 'localtime'),
                    SUM(foreground_seconds), COUNT(*)
             FROM app_sessions WHERE last_seen_at >= ?1 AND foreground_seconds > 0 GROUP BY 1 ORDER BY 1",
        )?;
        let daily_usage = daily_statement.query_map([since], |row| Ok(DailyUsage {
            date: row.get(0)?, foreground_seconds: row.get(1)?, launch_count: row.get(2)?,
        }))?.collect::<rusqlite::Result<_>>()?;
        Ok(AppUsageSummary {
            period_days,
            application_count: top_apps.len(),
            session_count: top_apps.iter().map(|app| app.session_count).sum(),
            total_runtime_seconds: top_apps.iter().map(|app| app.runtime_seconds).sum(),
            total_foreground_seconds: top_apps.iter().map(|app| app.foreground_seconds).sum(),
            total_background_seconds: top_apps.iter().map(|app| app.background_seconds).sum(),
            longest_used_app: top_apps.first().map(|app| app.name.clone()),
            top_apps: top_apps.into_iter().take(8).collect(),
            daily_usage,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn persists_and_loads_events() {
        let storage = Storage::open(PathBuf::from(":memory:")).unwrap();
        let event = TimelineEvent {
            id: "event-1".into(),
            occurred_at: 42,
            kind: "patrol".into(),
            message: "开始巡逻".into(),
        };
        storage.save_event(&event).unwrap();
        let loaded = storage.recent_events(10).unwrap();
        assert_eq!(loaded.len(), 1);
        assert_eq!(loaded[0].message, "开始巡逻");
    }

    #[test]
    fn persists_user_settings() {
        let storage = Storage::open(PathBuf::from(":memory:")).unwrap();
        let settings = UserSettings {
            cpu_threshold: 82.0,
            low_power_mode: true,
            ..UserSettings::default()
        };
        storage.save_settings(&settings).unwrap();
        let loaded = storage.load_settings();
        assert_eq!(loaded.cpu_threshold, 82.0);
        assert!(loaded.low_power_mode);
    }

    #[test]
    fn persists_application_lifecycle() {
        let storage = Storage::open(PathBuf::from(":memory:")).unwrap();
        let record = AppUsageRecord {
            session_id: "42-100".into(),
            name: "example.exe".into(),
            root_pid: 42,
            started_at: 100_000,
            first_seen_at: 101_000,
            last_seen_at: 161_000,
            closed_at: Some(161_000),
            runtime_seconds: 61,
            foreground_seconds: 40,
            background_seconds: 21,
            member_peak: 3,
            is_running: false,
        };
        storage.save_app_session(&record).unwrap();
        let loaded = storage.recent_app_sessions(10).unwrap();
        assert_eq!(loaded.len(), 1);
        assert_eq!(loaded[0].foreground_seconds, 40);
        assert_eq!(loaded[0].closed_at, Some(161_000));
    }

    #[test]
    fn history_range_reads_averaged_byte_rates_as_integers() {
        let storage = Storage::open(PathBuf::from(":memory:")).unwrap();
        let now_ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_millis() as u64;
        for (offset, disk_read, disk_write) in [(1_000, 100_u64, 300_u64), (2_000, 200, 500)] {
            storage.connection.execute(
                "INSERT INTO system_snapshots (
                    captured_at, cpu_percent, memory_percent, used_memory_bytes,
                    total_memory_bytes, disk_read_bps, disk_write_bps,
                    network_receive_bps, network_send_bps
                 ) VALUES (?1, 20, 60, 1, 2, ?2, ?3, 40, 60)",
                params![now_ms - offset, disk_read, disk_write],
            ).unwrap();
        }

        let history = storage.history_range(10).unwrap();
        assert!(!history.points.is_empty());
        assert!(history.points.iter().all(|point| point.disk_bps > 0));
        assert!(history.points.iter().all(|point| point.network_bps == 100));
    }

    #[test]
    fn persists_application_metrics_and_alert_workflow() {
        let storage = Storage::open(PathBuf::from(":memory:")).unwrap();
        let now_ms = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_millis() as u64;
        storage.connection.execute("INSERT INTO application_snapshots (captured_at, app_name, root_pid, cpu_percent, memory_bytes, disk_read_bps, disk_write_bps, network_bps) VALUES (?1, 'demo.exe', 7, 25, 1024, 10, 20, NULL)", [now_ms]).unwrap();
        let history = storage.application_history("demo.exe", 10).unwrap();
        assert_eq!(history.points.len(), 1);
        assert_eq!(history.points[0].memory_bytes, 1024);

        let finding = Finding { id: "alert-1".into(), kind: "cpu.sustained_high".into(), severity: "warning".into(), title: "CPU 很忙".into(), message: "测试".into(), first_seen_at: now_ms, evidence: vec![], process: None };
        storage.save_alert(&finding).unwrap();
        assert_eq!(storage.alerts(Some("unread")).unwrap().len(), 1);
        storage.update_alert("alert-1", "acknowledged", "已检查", now_ms + 1).unwrap();
        let alert = storage.alerts(Some("acknowledged")).unwrap().pop().unwrap();
        assert_eq!(alert.note, "已检查");
    }
}
