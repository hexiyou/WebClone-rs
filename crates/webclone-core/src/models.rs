//! 数据模型。字段名与 C# 版保持一致，清单 JSON 可以直接互相读写。

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// 克隆模式。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum CloneMode {
    /// 单页模板：只抓当前页与它引用的资源，不递归 a 链接。
    /// 等价于 wget -r -E -np -c -k --ignore-tags=a
    SinglePage,
    /// 整站克隆：递归追踪 a 标签（不向上级目录）。等价于 wget -r -np -c -k
    FullSite,
}

impl CloneMode {
    /// 清单里记录的字符串形式（与 C# 的 enum.ToString() 一致）。
    pub fn as_str(self) -> &'static str {
        match self {
            CloneMode::SinglePage => "SinglePage",
            CloneMode::FullSite => "FullSite",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            CloneMode::SinglePage => "单页模板",
            CloneMode::FullSite => "整站克隆",
        }
    }
}

/// 资源类型：决定扩展名补全规则、文件名策略以及是否跟踪其中的次级链接。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResourceKind {
    Html,
    Css,
    JavaScript,
    Image,
    Font,
    Media,
    Other,
}

/// 代理类型。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ProxyKind {
    None,
    /// HTTP / HTTPS 代理（CONNECT 隧道）。
    Http,
    /// SOCKS5 代理，支持无认证与用户名密码认证。
    Socks5,
}

impl std::fmt::Display for ProxyKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ProxyKind::None => write!(f, "none"),
            ProxyKind::Http => write!(f, "http"),
            ProxyKind::Socks5 => write!(f, "socks5"),
        }
    }
}

/// 下载状态。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DownloadStatus {
    /// 已下载或续传完成。
    Downloaded,
    /// 本地已存在且服务端未变动（HTTP 304），跳过。
    Skipped,
    /// 下载失败。
    Failed,
}

/// 单个资源的下载结果。
#[derive(Debug, Clone)]
pub struct DownloadResult {
    pub status: DownloadStatus,
    pub url: url::Url,
    pub local_path: String,
    pub status_code: u16,
    pub bytes_written: u64,
    pub total_bytes: u64,
    /// MIME 类型，如 text/css。
    pub content_type: Option<String>,
    /// 完整的 Content-Type 头，含 charset 参数。编码识别要用它。
    pub content_type_header: Option<String>,
    pub etag: Option<String>,
    pub last_modified: Option<chrono::DateTime<chrono::FixedOffset>>,
    /// 重定向之后的最终地址。
    pub final_uri: Option<url::Url>,
    pub error_message: Option<String>,
    pub was_resumed: bool,
}

impl DownloadResult {
    pub fn success(&self) -> bool {
        self.status != DownloadStatus::Failed
    }

    pub fn failed(url: url::Url, local_path: String, message: String, status_code: u16) -> Self {
        Self {
            status: DownloadStatus::Failed,
            url,
            local_path,
            status_code,
            bytes_written: 0,
            total_bytes: 0,
            content_type: None,
            content_type_header: None,
            etag: None,
            last_modified: None,
            final_uri: None,
            error_message: Some(message),
            was_resumed: false,
        }
    }
}

/// 单个资源的下载记录，用于增量判断。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ManifestEntry {
    #[serde(rename = "Url", default)]
    pub url: String,
    /// 相对保存根目录的路径。
    #[serde(rename = "LocalPath", default)]
    pub local_path: String,
    #[serde(rename = "ETag", default, skip_serializing_if = "Option::is_none")]
    pub etag: Option<String>,
    #[serde(rename = "LastModified", default, skip_serializing_if = "Option::is_none")]
    pub last_modified: Option<String>,
    #[serde(rename = "Length", default)]
    pub length: u64,
    #[serde(rename = "ContentType", default, skip_serializing_if = "Option::is_none")]
    pub content_type: Option<String>,
    #[serde(rename = "DownloadedAt", default = "default_downloaded_at")]
    pub downloaded_at: chrono::DateTime<chrono::Local>,
}

fn default_downloaded_at() -> chrono::DateTime<chrono::Local> {
    chrono::Local::now()
}

/// 整次克隆任务的清单，落盘为 .webclone-manifest.json。
/// 下次对同一目录克隆时读取它，配合条件请求实现"没变动就不重新下载"。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CloneManifest {
    #[serde(rename = "Version", default = "default_version")]
    pub version: String,
    #[serde(rename = "StartUrl", default)]
    pub start_url: String,
    #[serde(rename = "Mode", default)]
    pub mode: String,
    #[serde(rename = "UpdatedAt", default = "default_updated_at")]
    pub updated_at: chrono::DateTime<chrono::Local>,
    #[serde(rename = "Entries", default)]
    pub entries: HashMap<String, ManifestEntry>,
}

fn default_version() -> String {
    "1.0".to_owned()
}

fn default_updated_at() -> chrono::DateTime<chrono::Local> {
    chrono::Local::now()
}

impl CloneManifest {
    pub const FILE_NAME: &'static str = ".webclone-manifest.json";

    pub fn new(start_url: String, mode: CloneMode) -> Self {
        Self {
            version: default_version(),
            start_url,
            mode: mode.as_str().to_owned(),
            updated_at: chrono::Local::now(),
            entries: HashMap::new(),
        }
    }

    pub fn get(&self, url: &str) -> Option<&ManifestEntry> {
        self.entries.get(url)
    }

    pub fn set(&mut self, mut entry: ManifestEntry) {
        entry.downloaded_at = chrono::Local::now();
        self.entries.insert(entry.url.clone(), entry);
    }
}
