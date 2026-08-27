use crate::types::{ProgramRisk, SecurityReport, StartupEntry};
use std::{
    collections::HashSet,
    ffi::OsStr,
    mem::size_of,
    os::windows::ffi::OsStrExt,
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};
use sysinfo::System;
use windows_sys::{
    core::GUID,
    Win32::Security::WinTrust::{
        WinVerifyTrust, WINTRUST_ACTION_GENERIC_VERIFY_V2, WINTRUST_DATA, WINTRUST_DATA_0,
        WINTRUST_FILE_INFO, WTD_CACHE_ONLY_URL_RETRIEVAL, WTD_CHOICE_FILE,
        WTD_REVOCATION_CHECK_NONE, WTD_REVOKE_NONE, WTD_STATEACTION_IGNORE, WTD_UI_NONE,
    },
};
use winreg::{
    enums::{HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE},
    RegKey,
};

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

fn wide(value: &OsStr) -> Vec<u16> {
    value.encode_wide().chain(Some(0)).collect()
}

fn has_valid_signature(path: &Path) -> bool {
    let path_wide = wide(path.as_os_str());
    let mut file_info = WINTRUST_FILE_INFO {
        cbStruct: size_of::<WINTRUST_FILE_INFO>() as u32,
        pcwszFilePath: path_wide.as_ptr(),
        hFile: std::ptr::null_mut(),
        pgKnownSubject: std::ptr::null_mut(),
    };
    let mut data = WINTRUST_DATA {
        cbStruct: size_of::<WINTRUST_DATA>() as u32,
        pPolicyCallbackData: std::ptr::null_mut(),
        pSIPClientData: std::ptr::null_mut(),
        dwUIChoice: WTD_UI_NONE,
        fdwRevocationChecks: WTD_REVOKE_NONE,
        dwUnionChoice: WTD_CHOICE_FILE,
        Anonymous: WINTRUST_DATA_0 {
            pFile: &mut file_info,
        },
        dwStateAction: WTD_STATEACTION_IGNORE,
        hWVTStateData: std::ptr::null_mut(),
        pwszURLReference: std::ptr::null_mut(),
        dwProvFlags: WTD_REVOCATION_CHECK_NONE | WTD_CACHE_ONLY_URL_RETRIEVAL,
        dwUIContext: 0,
        pSignatureSettings: std::ptr::null_mut(),
    };
    let mut action: GUID = WINTRUST_ACTION_GENERIC_VERIFY_V2;
    // WinVerifyTrust is read-only here: no UI, no network retrieval, no state handle retained.
    unsafe {
        WinVerifyTrust(
            std::ptr::null_mut(),
            &mut action,
            (&mut data as *mut WINTRUST_DATA).cast(),
        ) == 0
    }
}

fn user_writable_path(path: &Path) -> bool {
    let normalized = path.to_string_lossy().to_ascii_lowercase();
    ["\\appdata\\", "\\temp\\", "\\downloads\\", "\\desktop\\"]
        .iter()
        .any(|fragment| normalized.contains(fragment))
}

fn assess_program(pid: u32, name: String, path: PathBuf) -> ProgramRisk {
    let signed = has_valid_signature(&path);
    let writable = user_writable_path(&path);
    let mut reasons = Vec::new();
    if !signed {
        reasons.push("没有可验证的数字签名".into());
    }
    if writable {
        reasons.push("程序位于用户可写目录".into());
    }
    let risk_level = match (signed, writable) {
        (false, true) => "medium",
        (false, false) | (true, true) => "low",
        (true, false) => "normal",
    };
    ProgramRisk {
        pid,
        name,
        path: path.to_string_lossy().into_owned(),
        signature_status: if signed { "valid" } else { "unverified" }.into(),
        risk_level: risk_level.into(),
        reasons,
    }
}

fn registry_startups(root: winreg::HKEY, root_name: &str) -> Vec<StartupEntry> {
    let key = RegKey::predef(root);
    let mut entries = Vec::new();
    for subkey in [
        r"Software\Microsoft\Windows\CurrentVersion\Run",
        r"Software\Microsoft\Windows\CurrentVersion\RunOnce",
    ] {
        let source = format!("{root_name}\\{subkey}");
        let Ok(run) = key.open_subkey(subkey) else {
            continue;
        };
        for value in run.enum_values().flatten() {
            let name = value.0;
            let Ok(command) = run.get_value::<String, _>(&name) else {
                continue;
            };
            let risky = command.to_ascii_lowercase().contains("\\temp\\");
            entries.push(StartupEntry {
                name,
                command,
                source: source.clone(),
                risk_level: if risky { "medium" } else { "normal" }.into(),
                reasons: if risky {
                    vec!["启动命令指向临时目录".into()]
                } else {
                    vec![]
                },
            });
        }
    }
    entries
}

fn folder_startups() -> Vec<StartupEntry> {
    let mut folders = Vec::new();
    if let Some(appdata) = std::env::var_os("APPDATA") {
        folders.push(PathBuf::from(appdata).join(r"Microsoft\Windows\Start Menu\Programs\Startup"));
    }
    if let Some(program_data) = std::env::var_os("PROGRAMDATA") {
        folders.push(
            PathBuf::from(program_data).join(r"Microsoft\Windows\Start Menu\Programs\Startup"),
        );
    }
    folders
        .into_iter()
        .flat_map(|folder| {
            std::fs::read_dir(&folder)
                .ok()
                .into_iter()
                .flatten()
                .filter_map(move |entry| {
                    let path = entry.ok()?.path();
                    Some(StartupEntry {
                        name: path.file_name()?.to_string_lossy().into_owned(),
                        command: path.to_string_lossy().into_owned(),
                        source: "Startup 文件夹".into(),
                        risk_level: "normal".into(),
                        reasons: vec![],
                    })
                })
        })
        .collect()
}

pub fn scan_security() -> Result<SecurityReport, String> {
    let system = System::new_all();
    let mut seen = HashSet::new();
    let mut programs = Vec::new();
    let mut signed_programs = 0;
    for (pid, process) in system.processes() {
        let Some(path) = process.exe() else { continue };
        let canonical = path.to_string_lossy().to_ascii_lowercase();
        if !seen.insert(canonical) || programs.len() >= 120 {
            continue;
        }
        let assessed = assess_program(
            pid.as_u32(),
            process.name().to_string_lossy().into_owned(),
            path.to_path_buf(),
        );
        if assessed.signature_status == "valid" {
            signed_programs += 1;
        }
        programs.push(assessed);
    }
    programs.sort_by_key(|program| match program.risk_level.as_str() {
        "medium" => 0,
        "low" => 1,
        _ => 2,
    });
    let mut startup_entries = registry_startups(HKEY_CURRENT_USER, "当前用户");
    startup_entries.extend(registry_startups(HKEY_LOCAL_MACHINE, "本机"));
    startup_entries.extend(folder_startups());
    let notable = programs
        .iter()
        .filter(|program| program.risk_level == "medium")
        .count()
        + startup_entries
            .iter()
            .filter(|entry| entry.risk_level == "medium")
            .count();
    Ok(SecurityReport {
        scanned_at: now_ms(),
        scanned_programs: programs.len(),
        signed_programs,
        programs: programs
            .into_iter()
            .filter(|program| program.risk_level != "normal")
            .take(30)
            .collect(),
        startup_entries,
        summary: if notable == 0 {
            "没有发现明显需要注意的项目。".into()
        } else {
            format!("发现 {notable} 个值得进一步确认的项目。")
        },
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identifies_user_writable_locations() {
        assert!(user_writable_path(Path::new(
            r"C:\Users\me\AppData\Local\Temp\a.exe"
        )));
        assert!(!user_writable_path(Path::new(
            r"C:\Program Files\Example\a.exe"
        )));
    }
}
