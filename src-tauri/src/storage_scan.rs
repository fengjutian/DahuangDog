use rusqlite::{params, Connection, OptionalExtension, Transaction};
use serde::{Deserialize, Serialize};
use std::{
    cmp::Reverse, collections::HashSet, fs,
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
    #[serde(default)]
    pub resumed: bool,
    #[serde(default)]
    pub completed_items: usize,
    #[serde(default)]
    pub total_items: usize,
}

#[derive(Clone, Default, Deserialize, Serialize)]
struct ScanStats { files: u64, directories: u64, skipped: u64 }

#[derive(Deserialize, Serialize)]
struct ScanCheckpoint {
    root_modified_at: u64,
    children: Vec<StorageEntry>,
    stats: ScanStats,
    total_items: usize,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CapacitySnapshot {
    pub captured_at: u64,
    pub size_bytes: u64,
    pub file_count: u64,
    pub directory_count: u64,
    pub skipped_count: u64,
}

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
         );
         CREATE TABLE IF NOT EXISTS storage_nodes (
           root_path TEXT NOT NULL, path TEXT NOT NULL, parent_path TEXT,
           name TEXT NOT NULL, kind TEXT NOT NULL, size_bytes INTEGER NOT NULL,
           modified_at INTEGER NOT NULL, indexed_at INTEGER NOT NULL,
           PRIMARY KEY(root_path, path)
         );
         CREATE INDEX IF NOT EXISTS idx_storage_nodes_parent ON storage_nodes(root_path, parent_path);
         CREATE TABLE IF NOT EXISTS storage_scan_tasks (
           root_path TEXT PRIMARY KEY, root_modified_at INTEGER NOT NULL,
           updated_at INTEGER NOT NULL, checkpoint_json TEXT NOT NULL
         );
         CREATE TABLE IF NOT EXISTS storage_capacity_snapshots (
           id INTEGER PRIMARY KEY, root_path TEXT NOT NULL, captured_at INTEGER NOT NULL,
           size_bytes INTEGER NOT NULL, file_count INTEGER NOT NULL,
           directory_count INTEGER NOT NULL, skipped_count INTEGER NOT NULL
         );
         CREATE INDEX IF NOT EXISTS idx_capacity_root_time ON storage_capacity_snapshots(root_path, captured_at DESC);"
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

fn load_checkpoint(path: &Path, modified_at: u64) -> Option<ScanCheckpoint> {
    let connection = index_connection().ok()?;
    let json = connection.query_row(
        "SELECT checkpoint_json FROM storage_scan_tasks WHERE root_path=?1 AND root_modified_at=?2",
        params![path.display().to_string(), modified_at], |row| row.get::<_, String>(0),
    ).optional().ok()??;
    serde_json::from_str(&json).ok()
}

fn save_checkpoint(path: &Path, checkpoint: &ScanCheckpoint) {
    let Ok(connection) = index_connection() else { return };
    let Ok(json) = serde_json::to_string(checkpoint) else { return };
    let _ = connection.execute(
        "INSERT INTO storage_scan_tasks(root_path,root_modified_at,updated_at,checkpoint_json) VALUES(?1,?2,?3,?4)
         ON CONFLICT(root_path) DO UPDATE SET root_modified_at=excluded.root_modified_at,updated_at=excluded.updated_at,checkpoint_json=excluded.checkpoint_json",
        params![path.display().to_string(), checkpoint.root_modified_at, now_millis(), json],
    );
}

fn remove_checkpoint(path: &Path) {
    if let Ok(connection) = index_connection() {
        let _ = connection.execute("DELETE FROM storage_scan_tasks WHERE root_path=?1", [path.display().to_string()]);
    }
}

fn insert_node(transaction: &Transaction<'_>, root_path: &str, parent_path: Option<&str>, entry: &StorageEntry, indexed_at: u64) {
    let modified_at = modified_millis(Path::new(&entry.path));
    let _ = transaction.execute(
        "INSERT INTO storage_nodes(root_path,path,parent_path,name,kind,size_bytes,modified_at,indexed_at) VALUES(?1,?2,?3,?4,?5,?6,?7,?8)
         ON CONFLICT(root_path,path) DO UPDATE SET parent_path=excluded.parent_path,name=excluded.name,kind=excluded.kind,size_bytes=excluded.size_bytes,modified_at=excluded.modified_at,indexed_at=excluded.indexed_at",
        params![root_path, entry.path, parent_path, entry.name, entry.kind, entry.size_bytes, modified_at, indexed_at],
    );
    for child in &entry.children { insert_node(transaction, root_path, Some(&entry.path), child, indexed_at); }
}

fn save_nodes(root: &Path, entry: &StorageEntry) {
    let Ok(mut connection) = index_connection() else { return };
    let Ok(transaction) = connection.transaction() else { return };
    insert_node(&transaction, &root.display().to_string(), Some(&root.display().to_string()), entry, now_millis());
    let _ = transaction.commit();
}

fn save_capacity_snapshot(path: &Path, result: &StorageScanResult) {
    let Ok(connection) = index_connection() else { return };
    let _ = connection.execute(
        "INSERT INTO storage_capacity_snapshots(root_path,captured_at,size_bytes,file_count,directory_count,skipped_count) VALUES(?1,?2,?3,?4,?5,?6)",
        params![path.display().to_string(), result.indexed_at, result.root.size_bytes, result.file_count, result.directory_count, result.skipped_count],
    );
    let _ = connection.execute(
        "DELETE FROM storage_capacity_snapshots WHERE root_path=?1 AND id NOT IN (SELECT id FROM storage_capacity_snapshots WHERE root_path=?1 ORDER BY captured_at DESC LIMIT 180)",
        [path.display().to_string()],
    );
}

pub fn capacity_history(root: String) -> Result<Vec<CapacitySnapshot>, String> {
    let connection = index_connection()?;
    let mut statement = connection.prepare(
        "SELECT captured_at,size_bytes,file_count,directory_count,skipped_count FROM storage_capacity_snapshots WHERE root_path=?1 ORDER BY captured_at DESC LIMIT 30"
    ).map_err(|error| error.to_string())?;
    let rows = statement.query_map([root], |row| Ok(CapacitySnapshot {
        captured_at: row.get(0)?, size_bytes: row.get(1)?, file_count: row.get(2)?, directory_count: row.get(3)?, skipped_count: row.get(4)?,
    })).map_err(|error| error.to_string())?;
    rows.collect::<Result<Vec<_>, _>>().map_err(|error| error.to_string())
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
    if force { remove_checkpoint(&path); }
    let checkpoint = (!force).then(|| load_checkpoint(&path, modified_at)).flatten();
    let resumed = checkpoint.is_some();
    let mut stats = checkpoint.as_ref().map(|value| value.stats.clone())
        .unwrap_or(ScanStats { directories: 1, ..Default::default() });
    let mut children = checkpoint.as_ref().map(|value| value.children.clone()).unwrap_or_default();
    for entry in &children { on_entry(entry.clone()); }
    let completed_paths: HashSet<_> = children.iter().map(|entry| entry.path.clone()).collect();
    let read_dir = fs::read_dir(&path).map_err(|error| format!("无法读取该目录：{error}"))?;
    let mut paths = Vec::new();
    for item in read_dir {
        if cancelled.load(Ordering::Relaxed) { return Err("扫描已取消".into()); }
        match item {
            Ok(item) if !completed_paths.contains(&item.path().display().to_string()) => paths.push(item.path()),
            Ok(_) => {},
            Err(_) => stats.skipped += 1,
        }
    }
    let total_items = checkpoint.as_ref().map(|value| value.total_items).unwrap_or(paths.len() + children.len());
    save_checkpoint(&path, &ScanCheckpoint { root_modified_at: modified_at, children: children.clone(), stats: stats.clone(), total_items });
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
            save_nodes(&path, &entry);
            on_entry(entry.clone()); children.push(entry);
            save_checkpoint(&path, &ScanCheckpoint { root_modified_at: modified_at, children: children.clone(), stats: stats.clone(), total_items });
        }
    });
    if cancelled.load(Ordering::Relaxed) { return Err("扫描已取消".into()); }
    children.sort_unstable_by_key(|child| Reverse(child.size_bytes));
    let size_bytes = children.iter().map(|entry| entry.size_bytes).sum();
    let result = StorageScanResult {
        root: StorageEntry { name: display_name(&path), path: path.display().to_string(), size_bytes, kind: "directory".into(), children },
        file_count: stats.files, directory_count: stats.directories, skipped_count: stats.skipped, cache_hit: false, indexed_at: now_millis(),
        resumed, completed_items: total_items, total_items,
    };
    save_nodes(&path, &result.root);
    save_index(&path, true, modified_at, &result);
    save_capacity_snapshot(&path, &result);
    remove_checkpoint(&path);
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
    let visible_items = children.len();
    let result = StorageScanResult {
        root: StorageEntry { name: display_name(&path), path: path.display().to_string(), size_bytes, kind: "directory".into(), children },
        file_count: stats.files, directory_count: stats.directories, skipped_count: stats.skipped, cache_hit: false, indexed_at: now_millis(),
        resumed: false, completed_items: visible_items, total_items: visible_items,
    };
    save_nodes(&path, &result.root);
    save_index(&path, false, modified_at, &result);
    Ok(result)
}
