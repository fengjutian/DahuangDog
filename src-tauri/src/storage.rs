use crate::types::{HistorySummary, MetricPoint, SystemSnapshot, TimelineEvent};
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
             );",
            )
            .map_err(|error| format!("无法初始化本地记忆：{error}"))?;
        Ok(Self { connection })
    }

    pub fn save_snapshot(&self, snapshot: &SystemSnapshot) -> rusqlite::Result<()> {
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
        // 第一版保留 7 天原始快照。
        self.connection.execute(
            "DELETE FROM system_snapshots WHERE captured_at < ?1",
            params![snapshot.captured_at.saturating_sub(7 * 24 * 60 * 60 * 1000)],
        )?;
        Ok(())
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
        Ok(HistorySummary {
            points,
            baseline_cpu_percent: cpu,
            baseline_memory_percent: memory,
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
}
