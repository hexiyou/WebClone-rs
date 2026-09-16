//! GUI 选项的持久化模型，存为 exe 同目录的 `webclone-gui.json`。
//!
//! 字段名与落盘格式与 C# 版 `GuiSettings.cs` 完全一致（PascalCase），
//! 两个版本可以互读同一份配置文件。
//!
//! 加载策略（与 C# 版一致）：
//! * 整个文件损坏（不存在 / 非法 JSON / 根不是对象）→ 返回 `None`，调用方用全套默认值；
//! * 单个字段缺失、类型不对或数值越界 → 只跳过该字段，其余正常回显。
//!
//! 保存策略：写失败静默吞掉（例如 exe 放在无写权限目录），不影响克隆功能。

use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use webclone_core::EDGE_USER_AGENT;

/// GUI 选项。`#[serde(default)]` 保证缺字段时按默认值走，落盘为 PascalCase。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct GuiSettings {
    pub start_url: String,
    pub output_directory: String,
    /// `"single"` 或 `"full"`。
    pub mode: String,
    pub max_depth: i32,
    pub max_pages: i32,
    /// `"none"`、`"http"` 或 `"socks5"`。
    pub proxy_kind: String,
    pub proxy_host: String,
    pub proxy_port: i32,
    pub proxy_user_name: String,
    pub proxy_password: String,
    pub include_cross_origin_assets: bool,
    pub convert_to_relative_paths: bool,
    pub enable_incremental: bool,
    pub neutralize_external_links: bool,
    pub ignore_certificate_errors: bool,
    pub concurrency: i32,
    pub timeout_seconds: i32,
    pub retry_count: i32,
    /// 新增：是否启用自定义浏览器 UA（默认不勾选）。
    pub use_custom_user_agent: bool,
    /// 新增：自定义 UA 文本，默认 Edge 浏览器 UA。
    pub custom_user_agent: String,
}

impl Default for GuiSettings {
    fn default() -> Self {
        Self {
            start_url: String::new(),
            output_directory: String::new(),
            mode: "single".to_owned(),
            max_depth: 3,
            max_pages: 0,
            proxy_kind: "none".to_owned(),
            proxy_host: String::new(),
            proxy_port: 8080,
            proxy_user_name: String::new(),
            proxy_password: String::new(),
            include_cross_origin_assets: true,
            convert_to_relative_paths: true,
            enable_incremental: true,
            neutralize_external_links: true,
            ignore_certificate_errors: false,
            concurrency: 8,
            timeout_seconds: 30,
            retry_count: 3,
            use_custom_user_agent: false,
            custom_user_agent: EDGE_USER_AGENT.to_owned(),
        }
    }
}

pub struct GuiSettingsStore;

impl GuiSettingsStore {
    /// 配置文件路径：exe 同目录下的 `webclone-gui.json`。
    ///
    /// 用 `current_exe()` 而不是 `current_dir()`，语义与 C# 版的
    /// `Environment.ProcessPath` 对齐 —— 双击启动时工作目录可能是别处。
    pub fn get_file_path() -> PathBuf {
        std::env::current_exe()
            .ok()
            .and_then(|exe| exe.parent().map(|dir| dir.to_path_buf()))
            .map(|dir| dir.join("webclone-gui.json"))
            .unwrap_or_else(|| PathBuf::from("webclone-gui.json"))
    }

    /// 读取配置。文件不存在、损坏或根不是对象时返回 `None`。
    pub fn load() -> Option<GuiSettings> {
        let text = std::fs::read_to_string(Self::get_file_path()).ok()?;
        let root: Value = serde_json::from_str(&text).ok()?;
        let obj = root.as_object()?;

        let mut settings = GuiSettings::default();

        settings.start_url = get_string(obj, "StartUrl", "");
        settings.output_directory = get_string(obj, "OutputDirectory", "");
        settings.mode = get_string(obj, "Mode", "single");
        settings.max_depth = get_int(obj, "MaxDepth", 3, 0, 100);
        settings.max_pages = get_int(obj, "MaxPages", 0, 0, 100_000);
        settings.proxy_kind = get_string(obj, "ProxyKind", "none");
        settings.proxy_host = get_string(obj, "ProxyHost", "");
        settings.proxy_port = get_int(obj, "ProxyPort", 8080, 1, 65_535);
        settings.proxy_user_name = get_string(obj, "ProxyUserName", "");
        settings.proxy_password = get_string(obj, "ProxyPassword", "");
        settings.include_cross_origin_assets =
            get_bool(obj, "IncludeCrossOriginAssets", true);
        settings.convert_to_relative_paths = get_bool(obj, "ConvertToRelativePaths", true);
        settings.enable_incremental = get_bool(obj, "EnableIncremental", true);
        settings.neutralize_external_links = get_bool(obj, "NeutralizeExternalLinks", true);
        settings.ignore_certificate_errors = get_bool(obj, "IgnoreCertificateErrors", false);
        settings.concurrency = get_int(obj, "Concurrency", 8, 1, 64);
        settings.timeout_seconds = get_int(obj, "TimeoutSeconds", 30, 5, 3600);
        settings.retry_count = get_int(obj, "RetryCount", 3, 0, 10);
        settings.use_custom_user_agent = get_bool(obj, "UseCustomUserAgent", false);
        settings.custom_user_agent =
            get_string(obj, "CustomUserAgent", EDGE_USER_AGENT);

        if !settings.mode.eq_ignore_ascii_case("full")
            && !settings.mode.eq_ignore_ascii_case("single")
        {
            settings.mode = "single".to_owned();
        }

        if !settings.proxy_kind.eq_ignore_ascii_case("http")
            && !settings.proxy_kind.eq_ignore_ascii_case("socks5")
            && !settings.proxy_kind.eq_ignore_ascii_case("none")
        {
            settings.proxy_kind = "none".to_owned();
        }

        Some(settings)
    }

    /// 保存配置。失败静默处理（并尝试落一份诊断文件），不打断克隆。
    pub fn save(settings: &GuiSettings) {
        let json = match serde_json::to_string_pretty(settings) {
            Ok(text) => text,
            Err(err) => return dump_save_error(&err.to_string()),
        };

        if let Err(err) = std::fs::write(Self::get_file_path(), json) {
            dump_save_error(&err.to_string());
        }
    }
}

/// 保存失败时把原因写到 exe 旁的 `gui-save-error.txt`，便于排查（正常情况下该文件不存在）。
fn dump_save_error(message: &str) {
    let path = std::env::current_exe()
        .ok()
        .and_then(|exe| exe.parent().map(|dir| dir.to_path_buf()))
        .unwrap_or_else(|| PathBuf::from("."))
        .join("gui-save-error.txt");

    let stamp = chrono::Local::now().format("%Y-%m-%d %H:%M:%S");
    let _ = std::fs::write(path, format!("{}\n{}\n", stamp, message));
}

// ---------------------------------------------------------------- 字段级容错提取

fn get_string(obj: &Map<String, Value>, name: &str, fallback: &str) -> String {
    match obj.get(name) {
        Some(Value::String(text)) => text.clone(),
        _ => fallback.to_owned(),
    }
}

fn get_int(obj: &Map<String, Value>, name: &str, fallback: i32, min: i32, max: i32) -> i32 {
    let value = match obj.get(name) {
        Some(v) => v,
        None => return fallback,
    };

    let parsed = match value {
        Value::Number(n) => n.as_i64().map(|n| n as i32),
        // 宽松一点：字符串形式的数字也认
        Value::String(text) => text.trim().parse::<i32>().ok(),
        _ => None,
    };

    match parsed {
        Some(n) => n.clamp(min, max),
        None => fallback,
    }
}

fn get_bool(obj: &Map<String, Value>, name: &str, fallback: bool) -> bool {
    let value = match obj.get(name) {
        Some(v) => v,
        None => return fallback,
    };

    match value {
        Value::Bool(b) => *b,
        Value::Number(n) => n.as_i64().map(|n| n != 0).unwrap_or(fallback),
        Value::String(text) => {
            let text = text.trim();
            if text.eq_ignore_ascii_case("true") {
                true
            } else if text.eq_ignore_ascii_case("false") {
                false
            } else {
                text.parse::<i32>().map(|n| n != 0).unwrap_or(fallback)
            }
        }
        _ => fallback,
    }
}
