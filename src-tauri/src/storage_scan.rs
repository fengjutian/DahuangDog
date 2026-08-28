use serde::Serialize;
use std::{cmp::Reverse, fs, path::{Path, PathBuf}};

const MAX_DEPTH: usize = 64;
const MAX_VISIBLE_CHILDREN: usize = 160;

#[derive(Debug, Serialize)]
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

pub fn scan(root: String) -> Result<StorageScanResult, String> {
    let path = PathBuf::from(&root);
    if !path.is_absolute() || !path.exists() || !path.is_dir() {
        return Err("请选择一个存在的磁盘根目录".into());
    }
    let mut stats = ScanStats::default();
    let root = scan_entry(&path, 0, &mut stats).ok_or_else(|| "无法读取该磁盘".to_string())?;
    Ok(StorageScanResult { root, file_count: stats.files, directory_count: stats.directories, skipped_count: stats.skipped })
}
