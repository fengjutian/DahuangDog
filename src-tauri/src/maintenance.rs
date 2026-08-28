use crate::types::{ActionResult, CleanupCandidate, CleanupReport, MaintenancePreview};
use std::{collections::{hash_map::DefaultHasher, HashMap, HashSet}, fs, hash::Hasher, io::Read, path::{Path, PathBuf}, time::{SystemTime, UNIX_EPOCH}};
use uuid::Uuid;
use winreg::{enums::HKEY_CURRENT_USER, RegKey};

fn now_ms() -> u64 { SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_millis() as u64 }

fn visit_files(root: &Path, category: &str, cleanable: bool, result: &mut Vec<CleanupCandidate>) {
    let mut stack = vec![root.to_path_buf()];
    while let Some(folder) = stack.pop() {
        let Ok(entries) = fs::read_dir(&folder) else { continue };
        for entry in entries.flatten() {
            if result.len() >= 2_000 { return; }
            let path = entry.path();
            let Ok(metadata) = entry.metadata() else { continue };
            if metadata.is_dir() { stack.push(path); continue; }
            if !metadata.is_file() { continue; }
            let modified_at = metadata.modified().ok().and_then(|value| value.duration_since(UNIX_EPOCH).ok()).map(|value| value.as_millis() as u64).unwrap_or(0);
            if cleanable && now_ms().saturating_sub(modified_at) < 24 * 60 * 60 * 1000 { continue; }
            if !cleanable && metadata.len() < 100 * 1024 * 1024 { continue; }
            result.push(CleanupCandidate { path: path.to_string_lossy().into_owned(), category: category.into(), size_bytes: metadata.len(), modified_at, cleanable });
        }
    }
}

fn visit_matching_files(root: &Path, category: &str, result: &mut Vec<CleanupCandidate>, matches: impl Fn(&Path) -> bool) {
    let Ok(entries) = fs::read_dir(root) else { return };
    for entry in entries.flatten() {
        if result.len() >= 5_000 { return; }
        let path = entry.path();
        if !matches(&path) { continue; }
        let Ok(metadata) = entry.metadata() else { continue };
        if !metadata.is_file() { continue; }
        let modified_at = metadata.modified().ok().and_then(|value| value.duration_since(UNIX_EPOCH).ok()).map(|value| value.as_millis() as u64).unwrap_or(0);
        if now_ms().saturating_sub(modified_at) < 24 * 60 * 60 * 1000 { continue; }
        result.push(CleanupCandidate { path: path.to_string_lossy().into_owned(), category: category.into(), size_bytes: metadata.len(), modified_at, cleanable: true });
    }
}

fn profile_cache_roots(user_data: &Path, product: &'static str) -> Vec<(PathBuf, &'static str)> {
    let Ok(entries) = fs::read_dir(user_data) else { return vec![] };
    entries.flatten().filter_map(|entry| {
        let name = entry.file_name().to_string_lossy().into_owned();
        if name != "Default" && !name.starts_with("Profile ") { return None; }
        Some(entry.path())
    }).flat_map(|profile| [
        (profile.join("Cache"), product),
        (profile.join("Code Cache"), product),
        (profile.join("GPUCache"), product),
    ]).collect()
}

fn cleanup_roots() -> Vec<(PathBuf, &'static str)> {
    let mut roots = vec![(std::env::temp_dir(), "临时文件")];
    if let Some(local) = std::env::var_os("LOCALAPPDATA").map(PathBuf::from) {
        roots.push((local.join(r"Microsoft\Windows\INetCache"), "Windows 网络缓存"));
        roots.push((local.join(r"CrashDumps"), "应用崩溃转储"));
        roots.push((local.join(r"Microsoft\Windows\WER\ReportArchive"), "Windows 错误报告"));
        roots.push((local.join(r"Microsoft\Windows\WER\ReportQueue"), "Windows 错误报告"));
        roots.extend(profile_cache_roots(&local.join(r"Microsoft\Edge\User Data"), "Edge 多配置缓存"));
        roots.extend(profile_cache_roots(&local.join(r"Google\Chrome\User Data"), "Chrome 多配置缓存"));
        roots.extend(profile_cache_roots(&local.join(r"BraveSoftware\Brave-Browser\User Data"), "Brave 多配置缓存"));
        if let Ok(entries) = fs::read_dir(local.join(r"Mozilla\Firefox\Profiles")) {
            roots.extend(entries.flatten().map(|entry| (entry.path().join("cache2"), "Firefox 多配置缓存")));
        }
    }
    if let Some(roaming) = std::env::var_os("APPDATA").map(PathBuf::from) {
        roots.push((roaming.join(r"Opera Software\Opera Stable\Cache"), "Opera 缓存"));
    }
    if let Some(windows) = std::env::var_os("WINDIR").map(PathBuf::from) {
        roots.push((windows.join("Temp"), "Windows 临时文件"));
    }
    roots
}

fn directory_size(root: &Path, maximum_entries: usize) -> u64 {
    let mut size = 0_u64;
    let mut visited = 0_usize;
    let mut stack = vec![root.to_path_buf()];
    while let Some(folder) = stack.pop() {
        let Ok(entries) = fs::read_dir(folder) else { continue };
        for entry in entries.flatten() {
            if visited >= maximum_entries { return size; }
            visited += 1;
            let Ok(metadata) = entry.metadata() else { continue };
            if metadata.is_dir() { stack.push(entry.path()); }
            else if metadata.is_file() { size = size.saturating_add(metadata.len()); }
        }
    }
    size
}

fn add_directory_summary(path: PathBuf, category: &str, result: &mut Vec<CleanupCandidate>) {
    if !path.is_dir() { return; }
    let size_bytes = directory_size(&path, 100_000);
    if size_bytes == 0 { return; }
    let modified_at = path.metadata().ok().and_then(|value| value.modified().ok()).and_then(|value| value.duration_since(UNIX_EPOCH).ok()).map(|value| value.as_millis() as u64).unwrap_or(0);
    result.push(CleanupCandidate { path: path.to_string_lossy().into_owned(), category: category.into(), size_bytes, modified_at, cleanable: false });
}

fn candidate_duplicate_files() -> Vec<PathBuf> {
    let Some(profile) = std::env::var_os("USERPROFILE").map(PathBuf::from) else { return vec![] };
    let mut files = Vec::new();
    let mut stack = vec![profile.join("Downloads"), profile.join("Desktop"), profile.join("Documents")];
    while let Some(folder) = stack.pop() {
        let Ok(entries) = fs::read_dir(folder) else { continue };
        for entry in entries.flatten() {
            if files.len() >= 10_000 { return files; }
            let path = entry.path();
            let Ok(metadata) = entry.metadata() else { continue };
            if metadata.is_dir() { stack.push(path); }
            else if metadata.is_file() && metadata.len() >= 10 * 1024 * 1024 { files.push(path); }
        }
    }
    files
}

fn content_hash(path: &Path) -> Option<u64> {
    let mut file = fs::File::open(path).ok()?;
    let mut hasher = DefaultHasher::new();
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        let read = file.read(&mut buffer).ok()?;
        if read == 0 { break; }
        hasher.write(&buffer[..read]);
    }
    Some(hasher.finish())
}

fn duplicate_candidates(result: &mut Vec<CleanupCandidate>) {
    let mut by_size: HashMap<u64, Vec<PathBuf>> = HashMap::new();
    for path in candidate_duplicate_files() {
        if let Ok(metadata) = path.metadata() { by_size.entry(metadata.len()).or_default().push(path); }
    }
    let mut group = 0_usize;
    for (size, paths) in by_size.into_iter().filter(|(_, paths)| paths.len() > 1) {
        let mut by_hash: HashMap<u64, Vec<PathBuf>> = HashMap::new();
        for path in paths { if let Some(hash) = content_hash(&path) { by_hash.entry(hash).or_default().push(path); } }
        for paths in by_hash.into_values().filter(|paths| paths.len() > 1) {
            group += 1;
            for path in paths {
                if result.len() >= 5_000 { return; }
                let modified_at = path.metadata().ok().and_then(|value| value.modified().ok()).and_then(|value| value.duration_since(UNIX_EPOCH).ok()).map(|value| value.as_millis() as u64).unwrap_or(0);
                result.push(CleanupCandidate { path: path.to_string_lossy().into_owned(), category: format!("重复文件组 {group}（只读）"), size_bytes: size, modified_at, cleanable: false });
            }
        }
    }
}

pub fn scan_cleanup() -> CleanupReport {
    let mut candidates = Vec::new();
    for (root, category) in cleanup_roots() { visit_files(&root, category, true, &mut candidates); }
    if let Some(local) = std::env::var_os("LOCALAPPDATA").map(PathBuf::from) {
        visit_matching_files(&local.join(r"Microsoft\Windows\Explorer"), "缩略图缓存", &mut candidates, |path| path.file_name().map(|name| name.to_string_lossy().to_ascii_lowercase().starts_with("thumbcache_") && name.to_string_lossy().to_ascii_lowercase().ends_with(".db")).unwrap_or(false));
    }
    if let Some(profile) = std::env::var_os("USERPROFILE") { visit_files(&PathBuf::from(profile).join("Downloads"), "下载目录大文件（只读）", false, &mut candidates); }
    if let Some(windows) = std::env::var_os("WINDIR").map(PathBuf::from) {
        add_directory_summary(windows.join(r"SoftwareDistribution\Download"), "Windows 更新缓存（只读，需系统维护）", &mut candidates);
    }
    for letter in b'A'..=b'Z' {
        add_directory_summary(PathBuf::from(format!("{}:\\$Recycle.Bin", letter as char)), "Windows 回收站（只读）", &mut candidates);
    }
    duplicate_candidates(&mut candidates);
    let mut seen = HashSet::new();
    candidates.retain(|item| seen.insert((item.category.clone(), item.path.to_ascii_lowercase())));
    candidates.sort_by_key(|item| std::cmp::Reverse(item.size_bytes));
    let reclaimable_bytes = candidates.iter().filter(|item| item.cleanable).map(|item| item.size_bytes).sum();
    CleanupReport { scanned_at: now_ms(), reclaimable_bytes, candidates }
}

enum PendingMaintenance { Cleanup(Vec<PathBuf>), Startup { source: String, name: String, command: String, enable: bool } }

pub struct MaintenanceManager { pending: HashMap<String, (u64, PendingMaintenance)> }
impl MaintenanceManager {
    pub fn new() -> Self { Self { pending: HashMap::new() } }
    pub fn prepare_cleanup(&mut self, paths: Vec<String>) -> Result<MaintenancePreview, String> {
        let allowed: HashSet<PathBuf> = scan_cleanup().candidates.into_iter().filter(|item| item.cleanable).filter_map(|item| fs::canonicalize(item.path).ok()).collect();
        if allowed.is_empty() { return Err("没有发现仍可安全清理的文件".into()); }
        let mut valid = Vec::new(); let mut total = 0_u64;
        for value in paths.into_iter().take(1_000) {
            let path = fs::canonicalize(&value).map_err(|_| format!("文件已不存在：{value}"))?;
            if !allowed.contains(&path) || !path.is_file() { return Err("安全策略只允许处理本次扫描确认过的缓存文件".into()); }
            total = total.saturating_add(path.metadata().map(|m| m.len()).unwrap_or(0)); valid.push(path);
        }
        if valid.is_empty() { return Err("没有选择可清理文件".into()); }
        let id = Uuid::new_v4().to_string(); let expires_at = now_ms() + 30_000;
        self.pending.insert(id.clone(), (expires_at, PendingMaintenance::Cleanup(valid.clone())));
        Ok(MaintenancePreview { preview_id: id, title: "清理缓存文件".into(), warning: "文件将移入 Windows 回收站，可在回收站中恢复；正在使用或无法回收的文件会跳过。".into(), item_count: valid.len(), total_bytes: total, expires_at })
    }
    pub fn prepare_startup(&mut self, source: String, name: String, command: String, enable: bool) -> Result<MaintenancePreview, String> {
        if name.is_empty() || name.len() > 260 || (!source.contains("HKEY_CURRENT_USER") && source != "大黄狗\\已禁用") { return Err("首版只允许管理当前用户启动项".into()); }
        let id = Uuid::new_v4().to_string(); let expires_at = now_ms() + 30_000;
        self.pending.insert(id.clone(), (expires_at, PendingMaintenance::Startup { source, name: name.clone(), command, enable }));
        Ok(MaintenancePreview { preview_id: id, title: if enable { format!("启用启动项 {name}") } else { format!("禁用启动项 {name}") }, warning: "操作会修改当前用户注册表，并保留可恢复备份。".into(), item_count: 1, total_bytes: 0, expires_at })
    }
    pub fn confirm(&mut self, id: &str) -> Result<ActionResult, String> {
        let (expires, action) = self.pending.remove(id).ok_or("确认已失效")?;
        if expires < now_ms() { return Err("确认已过期".into()); }
        let message = match action {
            PendingMaintenance::Cleanup(paths) => recycle_files(paths)?,
            PendingMaintenance::Startup { source, name, command, enable } => { change_startup(&source, &name, &command, enable)?; format!("已{}启动项 {name}", if enable {"启用"} else {"禁用"}) }
        };
        Ok(ActionResult { action_id: Uuid::new_v4().to_string(), success: true, message })
    }
}

#[cfg(windows)]
fn recycle_files(paths: Vec<PathBuf>) -> Result<String, String> {
    use std::ptr::null;
    use windows_sys::Win32::UI::Shell::{SHFileOperationW, SHFILEOPSTRUCTW, FOF_ALLOWUNDO, FOF_NOCONFIRMATION, FOF_NOERRORUI, FOF_SILENT, FOF_WANTNUKEWARNING, FO_DELETE};
    let sizes: HashMap<PathBuf, u64> = paths.iter().map(|path| (path.clone(), path.metadata().map(|value| value.len()).unwrap_or(0))).collect();
    let mut source = Vec::<u16>::new();
    for path in &paths {
        source.extend(path.as_os_str().to_string_lossy().encode_utf16());
        source.push(0);
    }
    source.push(0);
    let mut operation = SHFILEOPSTRUCTW {
        hwnd: std::ptr::null_mut(),
        wFunc: FO_DELETE,
        pFrom: source.as_ptr(),
        pTo: null(),
        fFlags: (FOF_ALLOWUNDO | FOF_NOCONFIRMATION | FOF_NOERRORUI | FOF_SILENT | FOF_WANTNUKEWARNING) as u16,
        fAnyOperationsAborted: 0,
        hNameMappings: std::ptr::null_mut(),
        lpszProgressTitle: null(),
    };
    let result = unsafe { SHFileOperationW(&mut operation) };
    let moved: Vec<_> = paths.iter().filter(|path| !path.exists()).collect();
    let bytes = moved.iter().map(|path| sizes.get(*path).copied().unwrap_or(0)).sum::<u64>();
    if result != 0 && moved.is_empty() { return Err(format!("无法将文件移入 Windows 回收站（错误 {result}）")); }
    Ok(format!("已将 {} 个文件移入回收站，释放 {:.1} MB{}", moved.len(), bytes as f64 / 1024.0 / 1024.0, if operation.fAnyOperationsAborted != 0 || moved.len() < paths.len() { "；部分文件正在使用或无法回收，已跳过" } else { "" }))
}

#[cfg(not(windows))]
fn recycle_files(_: Vec<PathBuf>) -> Result<String, String> { Err("回收站清理功能仅支持 Windows".into()) }

fn change_startup(source: &str, name: &str, command: &str, enable: bool) -> Result<(), String> {
    let root = RegKey::predef(HKEY_CURRENT_USER);
    let (backup, _) = root.create_subkey(r"Software\DahuangDog\DisabledStartup").map_err(|e| e.to_string())?;
    if enable {
        let stored: String = backup.get_value(name).map_err(|_| "找不到启动项备份")?;
        let mut parts=stored.splitn(2,'\n'); let subkey=parts.next().ok_or("备份损坏")?; let value=parts.next().ok_or("备份损坏")?;
        let (run, _)=root.create_subkey(subkey).map_err(|e|e.to_string())?; run.set_value(name,&value).map_err(|e|e.to_string())?; backup.delete_value(name).map_err(|e|e.to_string())?;
    } else {
        let subkey = source.split("HKEY_CURRENT_USER\\").nth(1).ok_or("启动项来源不可管理")?;
        let run=root.open_subkey_with_flags(subkey, winreg::enums::KEY_READ | winreg::enums::KEY_WRITE).map_err(|e|e.to_string())?;
        let current: String=run.get_value(name).unwrap_or_else(|_| command.into()); backup.set_value(name,&format!("{subkey}\n{current}")).map_err(|e|e.to_string())?; run.delete_value(name).map_err(|e|e.to_string())?;
    }
    Ok(())
}

pub fn disabled_startups() -> Vec<(String,String)> {
    let root=RegKey::predef(HKEY_CURRENT_USER); let Ok(key)=root.open_subkey(r"Software\DahuangDog\DisabledStartup") else{return vec![]};
    key.enum_values().flatten().filter_map(|(name,_)| key.get_value::<String,_>(&name).ok().map(|value|(name,value.splitn(2,'\n').nth(1).unwrap_or("").into()))).collect()
}
