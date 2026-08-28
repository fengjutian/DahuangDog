use crate::types::{ActionResult, CleanupCandidate, CleanupReport, MaintenancePreview};
use std::{collections::HashMap, fs, path::{Path, PathBuf}, time::{SystemTime, UNIX_EPOCH}};
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

fn cleanup_roots() -> Vec<(PathBuf, &'static str)> {
    let mut roots = vec![(std::env::temp_dir(), "临时文件")];
    if let Some(local) = std::env::var_os("LOCALAPPDATA").map(PathBuf::from) {
        roots.push((local.join(r"Microsoft\Windows\INetCache"), "Windows 网络缓存"));
        roots.push((local.join(r"Microsoft\Edge\User Data\Default\Cache"), "Edge 缓存"));
        roots.push((local.join(r"Google\Chrome\User Data\Default\Cache"), "Chrome 缓存"));
    }
    roots
}

pub fn scan_cleanup() -> CleanupReport {
    let mut candidates = Vec::new();
    for (root, category) in cleanup_roots() { visit_files(&root, category, true, &mut candidates); }
    if let Some(profile) = std::env::var_os("USERPROFILE") { visit_files(&PathBuf::from(profile).join("Downloads"), "下载目录大文件（只读）", false, &mut candidates); }
    candidates.sort_by_key(|item| std::cmp::Reverse(item.size_bytes));
    let reclaimable_bytes = candidates.iter().filter(|item| item.cleanable).map(|item| item.size_bytes).sum();
    CleanupReport { scanned_at: now_ms(), reclaimable_bytes, candidates }
}

enum PendingMaintenance { Cleanup(Vec<PathBuf>), Startup { source: String, name: String, command: String, enable: bool } }

pub struct MaintenanceManager { pending: HashMap<String, (u64, PendingMaintenance)> }
impl MaintenanceManager {
    pub fn new() -> Self { Self { pending: HashMap::new() } }
    pub fn prepare_cleanup(&mut self, paths: Vec<String>) -> Result<MaintenancePreview, String> {
        let allowed: Vec<PathBuf> = cleanup_roots().into_iter().filter_map(|(root, _)| fs::canonicalize(root).ok()).collect();
        if allowed.is_empty() { return Err("无法定位可清理目录".into()); }
        let mut valid = Vec::new(); let mut total = 0_u64;
        for value in paths.into_iter().take(1_000) {
            let path = fs::canonicalize(&value).map_err(|_| format!("文件已不存在：{value}"))?;
            if !allowed.iter().any(|root| path.starts_with(root)) || !path.is_file() { return Err("安全策略只允许清理已扫描缓存目录中的普通文件".into()); }
            total = total.saturating_add(path.metadata().map(|m| m.len()).unwrap_or(0)); valid.push(path);
        }
        if valid.is_empty() { return Err("没有选择可清理文件".into()); }
        let id = Uuid::new_v4().to_string(); let expires_at = now_ms() + 30_000;
        self.pending.insert(id.clone(), (expires_at, PendingMaintenance::Cleanup(valid.clone())));
        Ok(MaintenancePreview { preview_id: id, title: "清理临时文件".into(), warning: "文件删除后无法从大黄狗恢复；正在使用的文件会跳过。".into(), item_count: valid.len(), total_bytes: total, expires_at })
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
            PendingMaintenance::Cleanup(paths) => { let mut count=0; let mut bytes=0; for path in paths { let size=path.metadata().map(|m|m.len()).unwrap_or(0); if fs::remove_file(&path).is_ok() { count+=1; bytes+=size; } } format!("已清理 {count} 个临时文件，释放 {:.1} MB", bytes as f64/1024.0/1024.0) }
            PendingMaintenance::Startup { source, name, command, enable } => { change_startup(&source, &name, &command, enable)?; format!("已{}启动项 {name}", if enable {"启用"} else {"禁用"}) }
        };
        Ok(ActionResult { action_id: Uuid::new_v4().to_string(), success: true, message })
    }
}

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
