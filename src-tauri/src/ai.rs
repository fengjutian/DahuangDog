use crate::types::{AiStatus, LocalDiagnosis};
use serde_json::{json, Value};
use std::time::Duration;

const CREDENTIAL_TARGET: &str = "DahuangDog/MiniMaxApiKey";
const ENDPOINT: &str = "https://api.minimaxi.com/v1/chat/completions";
const ALLOWED_MODELS: [&str; 4] = ["MiniMax-M2.7", "MiniMax-M2.7-highspeed", "MiniMax-M2.5", "MiniMax-M2.5-highspeed"];

fn validate_model(model: &str) -> Result<(), String> {
    if ALLOWED_MODELS.contains(&model) { Ok(()) } else { Err("不支持的 MiniMax 模型".into()) }
}

fn validate_api_key(key: &str) -> Result<(), String> {
    if key.len() < 16 || key.len() > 2_500 { Err("MiniMax API Key 格式不正确".into()) } else { Ok(()) }
}

fn normalize_api_key(value: &str) -> Result<String, String> {
    let mut key = value.trim().trim_matches(|character| character == '"' || character == '\'').trim();
    if key.get(..7).is_some_and(|prefix| prefix.eq_ignore_ascii_case("bearer ")) { key = key.get(7..).unwrap_or_default().trim(); }
    validate_api_key(key)?;
    if key.chars().any(char::is_whitespace) { return Err("API Key 中包含空格或换行，请只粘贴密钥本身".into()); }
    Ok(key.to_string())
}

#[cfg(windows)]
fn wide(value: &str) -> Vec<u16> { value.encode_utf16().chain(Some(0)).collect() }

#[cfg(windows)]
pub fn save_api_key(api_key: &str) -> Result<AiStatus, String> {
    use windows_sys::Win32::Security::Credentials::{CredWriteW, CREDENTIALW, CRED_PERSIST_LOCAL_MACHINE, CRED_TYPE_GENERIC};
    let key = normalize_api_key(api_key)?;
    if key.len() < 16 || key.len() > 2_500 { return Err("MiniMax API Key 格式不正确".into()); }
    let mut target = wide(CREDENTIAL_TARGET);
    let mut username = wide("MiniMax API");
    let mut blob = key.as_bytes().to_vec();
    let credential = CREDENTIALW {
        Type: CRED_TYPE_GENERIC,
        TargetName: target.as_mut_ptr(),
        CredentialBlobSize: blob.len() as u32,
        CredentialBlob: blob.as_mut_ptr(),
        Persist: CRED_PERSIST_LOCAL_MACHINE,
        UserName: username.as_mut_ptr(),
        ..Default::default()
    };
    if unsafe { CredWriteW(&credential, 0) } == 0 { return Err("无法将 API Key 保存到 Windows 凭据管理器".into()); }
    blob.fill(0);
    Ok(AiStatus { configured: true })
}

#[cfg(windows)]
fn read_api_key() -> Result<String, String> {
    use std::{ptr::null_mut, slice};
    use windows_sys::Win32::Security::Credentials::{CredFree, CredReadW, CREDENTIALW, CRED_TYPE_GENERIC};
    let target = wide(CREDENTIAL_TARGET);
    let mut credential: *mut CREDENTIALW = null_mut();
    if unsafe { CredReadW(target.as_ptr(), CRED_TYPE_GENERIC, 0, &mut credential) } == 0 || credential.is_null() {
        return Err("尚未配置 MiniMax API Key".into());
    }
    let value = unsafe {
        let item = &*credential;
        let bytes = slice::from_raw_parts(item.CredentialBlob, item.CredentialBlobSize as usize);
        String::from_utf8(bytes.to_vec()).map_err(|_| "Windows 凭据中的 API Key 已损坏".to_string())
    };
    unsafe { CredFree(credential.cast()) };
    value
}

#[cfg(windows)]
pub fn clear_api_key() -> Result<AiStatus, String> {
    use windows_sys::Win32::Security::Credentials::{CredDeleteW, CRED_TYPE_GENERIC};
    let target = wide(CREDENTIAL_TARGET);
    if unsafe { CredDeleteW(target.as_ptr(), CRED_TYPE_GENERIC, 0) } == 0 && has_api_key() {
        return Err("无法从 Windows 凭据管理器删除 API Key".into());
    }
    Ok(AiStatus { configured: false })
}

#[cfg(windows)]
pub fn has_api_key() -> bool { read_api_key().is_ok() }

#[cfg(not(windows))]
pub fn save_api_key(_: &str) -> Result<AiStatus, String> { Err("MiniMax 凭据管理仅支持 Windows".into()) }
#[cfg(not(windows))]
fn read_api_key() -> Result<String, String> { Err("MiniMax 凭据管理仅支持 Windows".into()) }
#[cfg(not(windows))]
pub fn clear_api_key() -> Result<AiStatus, String> { Err("MiniMax 凭据管理仅支持 Windows".into()) }
#[cfg(not(windows))]
pub fn has_api_key() -> bool { false }

pub fn status() -> AiStatus { AiStatus { configured: has_api_key() } }

pub fn test_connection(model: &str, api_key: Option<String>) -> Result<String, String> {
    validate_model(model)?;
    let key = match api_key.map(|value| value.trim().to_string()).filter(|value| !value.is_empty()) {
        Some(value) => value,
        None => read_api_key()?,
    };
    let key = normalize_api_key(&key)?;
    let response = reqwest::blocking::Client::builder().timeout(Duration::from_secs(20)).build()
        .map_err(|_| "无法初始化 MiniMax 网络客户端".to_string())?
        .post(ENDPOINT).bearer_auth(key).json(&json!({
            "model": model, "stream": false, "max_completion_tokens": 24, "temperature": 0,
            "messages": [{ "role": "user", "content": "请只回复：连接成功" }]
        })).send().map_err(|error| format!("无法连接 MiniMax：{error}"))?;
    let status = response.status();
    let payload: Value = response.json().map_err(|_| format!("MiniMax 返回了无法解析的响应（HTTP {status}）"))?;
    if !status.is_success() {
        let message = payload.pointer("/base_resp/status_msg").and_then(Value::as_str)
            .or_else(|| payload.pointer("/error/message").and_then(Value::as_str)).unwrap_or("请求失败");
        if status.as_u16() == 401 && (message.contains("Authorization") || message.contains("1004")) {
            return Err("MiniMax 未识别 API Key。请只粘贴密钥本身（不要包含 Bearer 或引号），并确认它来自国内开放平台 platform.minimaxi.com 的接口密钥页面。".into());
        }
        return Err(format!("MiniMax 请求失败（HTTP {status}）：{message}"));
    }
    let reply = payload.pointer("/choices/0/message/content").and_then(Value::as_str).unwrap_or("连接成功");
    Ok(format!("连接成功 · {model} · {}", reply.trim().chars().take(40).collect::<String>()))
}

fn response_json(content: &str) -> Result<Value, String> {
    let without_thinking = if let Some(end) = content.rfind("</think>") { &content[end + 8..] } else { content };
    let start = without_thinking.find('{').ok_or("MiniMax 返回内容缺少 JSON")?;
    let end = without_thinking.rfind('}').ok_or("MiniMax 返回内容缺少 JSON")?;
    serde_json::from_str(&without_thinking[start..=end]).map_err(|_| "MiniMax 返回内容格式不正确".into())
}

pub fn diagnose(model: &str, context: Value) -> Result<LocalDiagnosis, String> {
    validate_model(model)?;
    let api_key = normalize_api_key(&read_api_key()?)?;
    let body = json!({
        "model": model,
        "stream": false,
        "max_completion_tokens": 1400,
        "temperature": 0.2,
        "reasoning_split": true,
        "messages": [
            {
                "role": "system",
                "content": "你是大黄狗，一名谨慎的 Windows 性能诊断助手。只根据给出的客观指标判断，不虚构进程行为，不声称程序恶意，不提供未经用户确认的破坏性操作。只输出 JSON，结构必须是：{\"summary\":\"一句中文结论\",\"details\":[\"证据\"],\"suggestions\":[\"安全建议\"],\"confidence\":\"low|medium|high\"}。details 和 suggestions 各不超过 5 条。"
            },
            {
                "role": "user",
                "content": format!("请分析这份本机资源摘要：{}", context)
            }
        ]
    });
    let response = reqwest::blocking::Client::builder().timeout(Duration::from_secs(45)).build()
        .map_err(|_| "无法初始化 MiniMax 网络客户端".to_string())?
        .post(ENDPOINT).bearer_auth(api_key).json(&body).send()
        .map_err(|error| format!("无法连接 MiniMax：{error}"))?;
    let status = response.status();
    let payload: Value = response.json().map_err(|_| format!("MiniMax 返回了无法解析的响应（HTTP {status}）"))?;
    if !status.is_success() {
        let message = payload.pointer("/base_resp/status_msg").and_then(Value::as_str).or_else(|| payload.pointer("/error/message").and_then(Value::as_str)).unwrap_or("请求失败");
        return Err(format!("MiniMax 请求失败（HTTP {status}）：{message}"));
    }
    let content = payload.pointer("/choices/0/message/content").and_then(Value::as_str).ok_or("MiniMax 响应中没有诊断内容")?;
    let parsed = response_json(content)?;
    let summary = parsed.get("summary").and_then(Value::as_str).unwrap_or("MiniMax 没有给出明确结论").chars().take(240).collect();
    let list = |name: &str| parsed.get(name).and_then(Value::as_array).into_iter().flatten().filter_map(Value::as_str).take(5).map(|value| value.chars().take(300).collect()).collect::<Vec<String>>();
    let confidence = match parsed.get("confidence").and_then(Value::as_str) { Some("high") => "high", Some("low") => "low", _ => "medium" };
    Ok(LocalDiagnosis { summary, details: list("details"), suggestions: list("suggestions"), confidence: confidence.into(), source: "minimax".into(), model: Some(model.into()) })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_json_after_reasoning_block() {
        let parsed = response_json("<think>private reasoning</think>\n{\"summary\":\"正常\",\"details\":[],\"suggestions\":[],\"confidence\":\"high\"}").unwrap();
        assert_eq!(parsed["summary"], "正常");
    }

    #[test]
    fn model_allowlist_rejects_unexpected_values() {
        assert!(validate_model("MiniMax-M2.7").is_ok());
        assert!(validate_model("custom-model").is_err());
    }

    #[test]
    fn normalizes_pasted_bearer_prefix_and_quotes() {
        assert_eq!(normalize_api_key("  \"Bearer sk-test-1234567890\"  ").unwrap(), "sk-test-1234567890");
        assert!(normalize_api_key("sk-test 1234567890").is_err());
    }
}
