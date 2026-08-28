use serde::Serialize;
use std::{cmp::Reverse, fs, path::{Path, PathBuf}, sync::{atomic::{AtomicUsize, Ordering}, mpsc}, thread};

const MAX_DEPTH: usize = 64;
const MAX_VISIBLE_CHILDREN: usize = 160;

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StorageEntry {
    pub name: String,
    pub path: String,
    pub size_bytes: u64,
    pub kind: &'static str,
    pub children: Vec<StorageEntry>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StorageScanResult {
    pub root: StorageEntry,
    pub file_count: u64,
    pub directory_count: u64,
    pub skipped_count: u64,
}

#[derive(Default)]
struct ScanStats { files: u64, directories: u64, skipped: u64 }

fn display_name(path: &Path) -> String {
    path.file_name().and_then(|name| name.to_str()).map(str::to_owned)
        .unwrap_or_else(|| path.display().to_string())
}

fn scan_entry(path: &Path, depth: usize, stats: &mut ScanStats) -> Option<StorageEntry> {
    let metadata = match fs::symlink_metadata(path) {
        Ok(value) => value,
        Err(_) => { stats.skipped += 1; return None; }
    };
    if metadata.file_type().is_symlink() { stats.skipped += 1; return None; }
    if metadata.is_file() {
        stats.files += 1;
        return Some(StorageEntry { name: display_name(path), path: path.display().to_string(), size_bytes: metadata.len(), kind: "file", children: Vec::new() });
    }
    if !metadata.is_dir() || depth >= MAX_DEPTH { stats.skipped += 1; return None; }
    stats.directories += 1;
    let read_dir = match fs::read_dir(path) {
        Ok(value) => value,
        Err(_) => { stats.skipped += 1; return Some(StorageEntry { name: display_name(path), path: path.display().to_string(), size_bytes: 0, kind: "directory", children: Vec::new() }); }
    };
    let mut children: Vec<_> = read_dir.filter_map(|item| match item {
        Ok(item) => scan_entry(&item.path(), depth + 1, stats),
        Err(_) => { stats.skipped += 1; None }
    }).collect();
    children.sort_unstable_by_key(|child| Reverse(child.size_bytes));
    if children.len() > MAX_VISIBLE_CHILDREN {
        let hidden = children.split_off(MAX_VISIBLE_CHILDREN);
        let hidden_size = hidden.iter().map(|entry| entry.size_bytes).sum();
        children.push(StorageEntry { name: format!("其他 {} 个项目", hidden.len()), path: path.display().to_string(), size_bytes: hidden_size, kind: "aggregate", children: Vec::new() });
    }
    let size_bytes = children.iter().map(|entry| entry.size_bytes).sum();
    Some(StorageEntry { name: display_name(path), path: path.display().to_string(), size_bytes, kind: "directory", children })
}

pub fn scan_progressive<F>(root: String, mut on_entry: F) -> Result<StorageScanResult, String>
where F: FnMut(StorageEntry) {
    let path = PathBuf::from(&root);
    if !path.is_absolute() || !path.exists() || !path.is_dir() {
        return Err("请选择一个存在的磁盘根目录".into());
    }
    let mut stats = ScanStats::default();
    stats.directories += 1;
    let read_dir = fs::read_dir(&path).map_err(|error| format!("无法读取该目录：{error}"))?;
    let mut paths = Vec::new();
    for item in read_dir {
        match item { Ok(item) => paths.push(item.path()), Err(_) => stats.skipped += 1 }
    }
    let mut children = Vec::new();
    let worker_count = thread::available_parallelism().map(usize::from).unwrap_or(2).clamp(2, 6).min(paths.len().max(1));
    let next = AtomicUsize::new(0);
    let (sender, receiver) = mpsc::channel();
    thread::scope(|scope| {
        for _ in 0..worker_count {
            let sender = sender.clone();
            let paths = &paths;
            let next = &next;
            scope.spawn(move || loop {
                let index = next.fetch_add(1, Ordering::Relaxed);
                let Some(path) = paths.get(index) else { break };
                let mut child_stats = ScanStats::default();
                if let Some(entry) = scan_entry(path, 1, &mut child_stats) {
                    if sender.send((entry, child_stats)).is_err() { break; }
                }
            });
        }
        drop(sender);
        for (entry, child_stats) in receiver {
            stats.files += child_stats.files;
            stats.directories += child_stats.directories;
            stats.skipped += child_stats.skipped;
            on_entry(entry.clone());
            children.push(entry);
        }
    });
    children.sort_unstable_by_key(|child| Reverse(child.size_bytes));
    let size_bytes = children.iter().map(|entry| entry.size_bytes).sum();
    let root = StorageEntry { name: display_name(&path), path: path.display().to_string(), size_bytes, kind: "directory", children };
    Ok(StorageScanResult { root, file_count: stats.files, directory_count: stats.directories, skipped_count: stats.skipped })
}

pub fn list_directory(root: String) -> Result<StorageScanResult, String> {
    let path = PathBuf::from(&root);
    if !path.is_absolute() || !path.exists() || !path.is_dir() {
        return Err("请选择一个存在的目录".into());
    }
    let mut children = Vec::new();
    let mut stats = ScanStats { directories: 1, ..Default::default() };
    for item in fs::read_dir(&path).map_err(|error| format!("无法读取该目录：{error}"))? {
        let item = match item { Ok(value) => value, Err(_) => { stats.skipped += 1; continue; } };
        let child_path = item.path();
        let metadata = match fs::symlink_metadata(&child_path) { Ok(value) => value, Err(_) => { stats.skipped += 1; continue; } };
        if metadata.file_type().is_symlink() { stats.skipped += 1; continue; }
        if metadata.is_dir() {
            stats.directories += 1;
            children.push(StorageEntry { name: display_name(&child_path), path: child_path.display().to_string(), size_bytes: 0, kind: "directory", children: Vec::new() });
        } else if metadata.is_file() {
            stats.files += 1;
            children.push(StorageEntry { name: display_name(&child_path), path: child_path.display().to_string(), size_bytes: metadata.len(), kind: "file", children: Vec::new() });
        }
    }
    children.sort_unstable_by(|a, b| a.kind.cmp(b.kind).then_with(|| b.size_bytes.cmp(&a.size_bytes)).then_with(|| a.name.cmp(&b.name)));
    if children.len() > MAX_VISIBLE_CHILDREN {
        let hidden = children.split_off(MAX_VISIBLE_CHILDREN);
        let hidden_size = hidden.iter().map(|entry| entry.size_bytes).sum();
        children.push(StorageEntry { name: format!("其他 {} 个项目", hidden.len()), path: path.display().to_string(), size_bytes: hidden_size, kind: "aggregate", children: Vec::new() });
    }
    let size_bytes = children.iter().map(|entry| entry.size_bytes).sum();
    Ok(StorageScanResult {
        root: StorageEntry { name: display_name(&path), path: path.display().to_string(), size_bytes, kind: "directory", children },
        file_count: stats.files, directory_count: stats.directories, skipped_count: stats.skipped,
    })
}
