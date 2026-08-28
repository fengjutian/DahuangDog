use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use std::{
    cmp::Reverse, fs,
    path::{Path, PathBuf},
    sync::{atomic::{AtomicBool, AtomicUsize, Ordering}, mpsc, Arc},
    thread,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

const MAX_DEPTH: usize = 64;
const MAX_VISIBLE_CHILDREN: usize = 160;

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StorageEntry {
    pub name: String,
    pub path: String,
    pub size_bytes: u64,
    pub kind: String,
    pub children: Vec<StorageEntry>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StorageScanResult {
    pub root: StorageEntry,
    pub file_count: u64,
    pub directory_count: u64,
    pub skipped_count: u64,
    pub cache_hit: bool,
    pub indexed_at: u64,
}

#[derive(Default)]
struct ScanStats { files: u64, directories: u64, skipped: u64 }

fn now_millis() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_millis() as u64
}

fn modified_millis(path: &Path) -> u64 {
    fs::metadata(path).and_then(|value| value.modified()).ok()
        .and_then(|value| value.duration_since(UNIX_EPOCH).ok())
        .map(|value| value.as_millis() as u64).unwrap_or(0)
}

fn index_connection() -> Result<Connection, String> {
    let root = std::env::var_os("LOCALAPPDATA").map(PathBuf::from)
        .unwrap_or_else(std::env::temp_dir).join("DahuangDog");
    fs::create_dir_all(&root).map_err(|error| format!("无法创建索引目录：{error}"))?;
    let connection = Connection::open(root.join("memory.db")).map_err(|error| format!("无法打开目录索引：{error}"))?;
    connection.busy_timeout(Duration::from_secs(2)).map_err(|error| error.to_string())?;
    connection.execute_batch(
        "PRAGMA journal_mode=WAL;
         CREATE TABLE IF NOT EXISTS storage_directory_index (
           path TEXT NOT NULL, exact INTEGER NOT NULL, modified_at INTEGER NOT NULL,
           indexed_at INTEGER NOT NULL, result_json TEXT NOT NULL, PRIMARY KEY(path, exact)
         );"
    ).map_err(|error| format!("无法初始化目录索引：{error}"))?;
    Ok(connection)
}

fn load_index(path: &Path, exact: bool, modified_at: u64) -> Option<StorageScanResult> {
    let connection = index_connection().ok()?;
    let json = connection.query_row(
        "SELECT result_json FROM storage_directory_index WHERE path=?1 AND exact=?2 AND modified_at=?3",
        params![path.display().to_string(), exact as i32, modified_at],
        |row| row.get::<_, String>(0),
    ).optional().ok()??;
    let mut result: StorageScanResult = serde_json::from_str(&json).ok()?;
    result.cache_hit = true;
    Some(result)
}

fn save_index(path: &Path, exact: bool, modified_at: u64, result: &StorageScanResult) {
    let Ok(connection) = index_connection() else { return };
    let Ok(json) = serde_json::to_string(result) else { return };
    let _ = connection.execute(
        "INSERT INTO storage_directory_index(path,exact,modified_at,indexed_at,result_json) VALUES(?1,?2,?3,?4,?5)
         ON CONFLICT(path,exact) DO UPDATE SET modified_at=excluded.modified_at,indexed_at=excluded.indexed_at,result_json=excluded.result_json",
        params![path.display().to_string(), exact as i32, modified_at, result.indexed_at, json],
    );
}

fn display_name(path: &Path) -> String {
    path.file_name().and_then(|name| name.to_str()).map(str::to_owned)
        .unwrap_or_else(|| path.display().to_string())
}

fn scan_entry(path: &Path, depth: usize, stats: &mut ScanStats, cancelled: &AtomicBool) -> Option<StorageEntry> {
    if cancelled.load(Ordering::Relaxed) { return None; }
    let metadata = match fs::symlink_metadata(path) {
        Ok(value) => value,
        Err(_) => { stats.skipped += 1; return None; }
    };
    if metadata.file_type().is_symlink() { stats.skipped += 1; return None; }
    if metadata.is_file() {
        stats.files += 1;
        return Some(StorageEntry { name: display_name(path), path: path.display().to_string(), size_bytes: metadata.len(), kind: "file".into(), children: Vec::new() });
    }
    if !metadata.is_dir() || depth >= MAX_DEPTH { stats.skipped += 1; return None; }
    stats.directories += 1;
    let read_dir = match fs::read_dir(path) {
        Ok(value) => value,
        Err(_) => { stats.skipped += 1; return Some(StorageEntry { name: display_name(path), path: path.display().to_string(), size_bytes: 0, kind: "directory".into(), children: Vec::new() }); }
    };
    let mut children = Vec::new();
    for item in read_dir {
        if cancelled.load(Ordering::Relaxed) { return None; }
        match item {
            Ok(item) => if let Some(entry) = scan_entry(&item.path(), depth + 1, stats, cancelled) { children.push(entry); },
            Err(_) => stats.skipped += 1,
        }
    }
    children.sort_unstable_by_key(|child| Reverse(child.size_bytes));
    if children.len() > MAX_VISIBLE_CHILDREN {
        let hidden = children.split_off(MAX_VISIBLE_CHILDREN);
        let hidden_size = hidden.iter().map(|entry| entry.size_bytes).sum();
        children.push(StorageEntry { name: format!("其他 {} 个项目", hidden.len()), path: path.display().to_string(), size_bytes: hidden_size, kind: "aggregate".into(), children: Vec::new() });
    }
    let size_bytes = children.iter().map(|entry| entry.size_bytes).sum();
    Some(StorageEntry { name: display_name(path), path: path.display().to_string(), size_bytes, kind: "directory".into(), children })
}

pub fn scan_progressive<F>(root: String, force: bool, cancelled: Arc<AtomicBool>, mut on_entry: F) -> Result<StorageScanResult, String>
where F: FnMut(StorageEntry) {
    let path = PathBuf::from(&root);
    if !path.is_absolute() || !path.exists() || !path.is_dir() { return Err("请选择一个存在的磁盘或目录".into()); }
    let modified_at = modified_millis(&path);
    if !force { if let Some(result) = load_index(&path, true, modified_at) { return Ok(result); } }
    let mut stats = ScanStats { directories: 1, ..Default::default() };
    let read_dir = fs::read_dir(&path).map_err(|error| format!("无法读取该目录：{error}"))?;
    let mut paths = Vec::new();
    for item in read_dir {
        if cancelled.load(Ordering::Relaxed) { return Err("扫描已取消".into()); }
        match item { Ok(item) => paths.push(item.path()), Err(_) => stats.skipped += 1 }
    }
    let mut children = Vec::new();
    let worker_count = thread::available_parallelism().map(usize::from).unwrap_or(2).clamp(2, 6).min(paths.len().max(1));
    let next = AtomicUsize::new(0);
    let (sender, receiver) = mpsc::channel();
    thread::scope(|scope| {
        for _ in 0..worker_count {
            let sender = sender.clone(); let paths = &paths; let next = &next; let cancelled = &cancelled;
            scope.spawn(move || loop {
                if cancelled.load(Ordering::Relaxed) { break; }
                let index = next.fetch_add(1, Ordering::Relaxed);
                let Some(path) = paths.get(index) else { break };
                let mut child_stats = ScanStats::default();
                if let Some(entry) = scan_entry(path, 1, &mut child_stats, cancelled) {
                    if sender.send((entry, child_stats)).is_err() { break; }
                }
            });
        }
        drop(sender);
        for (entry, child_stats) in receiver {
            if cancelled.load(Ordering::Relaxed) { break; }
            stats.files += child_stats.files; stats.directories += child_stats.directories; stats.skipped += child_stats.skipped;
            on_entry(entry.clone()); children.push(entry);
        }
    });
    if cancelled.load(Ordering::Relaxed) { return Err("扫描已取消".into()); }
    children.sort_unstable_by_key(|child| Reverse(child.size_bytes));
    let size_bytes = children.iter().map(|entry| entry.size_bytes).sum();
    let result = StorageScanResult {
        root: StorageEntry { name: display_name(&path), path: path.display().to_string(), size_bytes, kind: "directory".into(), children },
        file_count: stats.files, directory_count: stats.directories, skipped_count: stats.skipped, cache_hit: false, indexed_at: now_millis(),
    };
    save_index(&path, true, modified_at, &result);
    Ok(result)
}

pub fn list_directory(root: String, force: bool) -> Result<StorageScanResult, String> {
    let path = PathBuf::from(&root);
    if !path.is_absolute() || !path.exists() || !path.is_dir() { return Err("请选择一个存在的目录".into()); }
    let modified_at = modified_millis(&path);
    if !force { if let Some(result) = load_index(&path, false, modified_at) { return Ok(result); } }
    let mut children = Vec::new();
    let mut stats = ScanStats { directories: 1, ..Default::default() };
    for item in fs::read_dir(&path).map_err(|error| format!("无法读取该目录：{error}"))? {
        let item = match item { Ok(value) => value, Err(_) => { stats.skipped += 1; continue; } };
        let child_path = item.path();
        let metadata = match fs::symlink_metadata(&child_path) { Ok(value) => value, Err(_) => { stats.skipped += 1; continue; } };
        if metadata.file_type().is_symlink() { stats.skipped += 1; continue; }
        if metadata.is_dir() {
            stats.directories += 1;
            children.push(StorageEntry { name: display_name(&child_path), path: child_path.display().to_string(), size_bytes: 0, kind: "directory".into(), children: Vec::new() });
        } else if metadata.is_file() {
            stats.files += 1;
            children.push(StorageEntry { name: display_name(&child_path), path: child_path.display().to_string(), size_bytes: metadata.len(), kind: "file".into(), children: Vec::new() });
        }
    }
    children.sort_unstable_by(|a, b| a.kind.cmp(&b.kind).then_with(|| b.size_bytes.cmp(&a.size_bytes)).then_with(|| a.name.cmp(&b.name)));
    if children.len() > MAX_VISIBLE_CHILDREN {
        let hidden = children.split_off(MAX_VISIBLE_CHILDREN);
        let hidden_size = hidden.iter().map(|entry| entry.size_bytes).sum();
        children.push(StorageEntry { name: format!("其他 {} 个项目", hidden.len()), path: path.display().to_string(), size_bytes: hidden_size, kind: "aggregate".into(), children: Vec::new() });
    }
    let size_bytes = children.iter().map(|entry| entry.size_bytes).sum();
    let result = StorageScanResult {
        root: StorageEntry { name: display_name(&path), path: path.display().to_string(), size_bytes, kind: "directory".into(), children },
        file_count: stats.files, directory_count: stats.directories, skipped_count: stats.skipped, cache_hit: false, indexed_at: now_millis(),
    };
    save_index(&path, false, modified_at, &result);
    Ok(result)
}
