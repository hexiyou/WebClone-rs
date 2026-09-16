//! 克隆任务配置。

use crate::models::{CloneMode, ProxyKind};

/// 默认 User-Agent：与 C# 版一致（Chrome 126 on Windows）。
pub const DEFAULT_USER_AGENT: &str =
    "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/126.0.0.0 Safari/537.36";

/// Edge 浏览器 UA，作为 GUI"自定义浏览器 UA"输入框的默认值。
pub const EDGE_USER_AGENT: &str =
    "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/126.0.0.0 Safari/537.36 Edg/126.0.0.0";

/// 代理配置。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProxySettings {
    pub kind: ProxyKind,
    /// 代理主机，如 127.0.0.1。
    pub host: String,
    pub port: u16,
    pub username: Option<String>,
    pub password: Option<String>,
}

impl Default for ProxySettings {
    fn default() -> Self {
        Self {
            kind: ProxyKind::None,
            host: String::new(),
            port: 0,
            username: None,
            password: None,
        }
    }
}

impl ProxySettings {
    pub fn none() -> Self {
        Self::default()
    }

    pub fn is_enabled(&self) -> bool {
        self.kind != ProxyKind::None && !self.host.trim().is_empty() && self.port > 0
    }

    /// 拼成 `socks5h://user:pass@host:port` 这类代理 URL。
    ///
    /// SOCKS5 用 `socks5h`（h = host）：域名交给代理去解析，
    /// 与 C# 版手写实现里按 ATYP=域名 发给代理的行为一致。
    pub fn to_proxy_url(&self) -> Option<String> {
        if !self.is_enabled() {
            return None;
        }

        let scheme = match self.kind {
            ProxyKind::Http => "http",
            ProxyKind::Socks5 => "socks5h",
            ProxyKind::None => return None,
        };

        let auth = match (&self.username, &self.password) {
            (Some(user), Some(pass)) if !user.is_empty() => {
                format!("{}:{}@", urlencode(user), urlencode(pass))
            }
            (Some(user), None) if !user.is_empty() => format!("{}@", urlencode(user)),
            _ => String::new(),
        };

        Some(format!("{}://{}{}:{}", scheme, auth, self.host, self.port))
    }
}

impl std::fmt::Display for ProxySettings {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if self.is_enabled() {
            match &self.username {
                Some(user) if !user.is_empty() => {
                    write!(f, "{}://{}@{}:{}", self.kind, user, self.host, self.port)
                }
                _ => write!(f, "{}://{}:{}", self.kind, self.host, self.port),
            }
        } else {
            write!(f, "直连")
        }
    }
}

fn urlencode(value: &str) -> String {
    const SET: &percent_encoding::AsciiSet = &percent_encoding::NON_ALPHANUMERIC
        .remove(b'-')
        .remove(b'.')
        .remove(b'_')
        .remove(b'~');
    percent_encoding::utf8_percent_encode(value, SET).to_string()
}

/// 克隆任务配置。字段与 C# 的 CloneOptions 一一对应。
#[derive(Debug, Clone)]
pub struct CloneOptions {
    /// 起始网址。
    pub start_uri: url::Url,
    /// 保存根目录。
    pub output_directory: String,
    /// 单页 / 整站。
    pub mode: CloneMode,
    /// 最大递归深度，1 表示只抓取起始页本身。
    pub max_depth: i32,
    /// 并发下载数。
    pub concurrency: usize,
    /// 单请求超时（秒）。
    pub timeout_seconds: u64,
    /// 失败重试次数。
    pub retry_count: u32,
    /// User-Agent。
    pub user_agent: String,
    /// 代理设置。
    pub proxy: ProxySettings,
    /// 是否忽略 HTTPS 证书错误。
    pub ignore_certificate_errors: bool,
    /// 是否下载跨域（CDN）上的资源。
    pub include_cross_origin_assets: bool,
    /// 是否启用增量：本地已存在且服务端未变动的资源跳过下载。
    pub enable_incremental: bool,
    /// 是否改写 HTML/CSS 中的资源路径为相对路径。
    pub convert_to_relative_paths: bool,
    /// 单页模式下，把指向外站的 a 链接替换成 #。
    pub neutralize_external_links: bool,
    /// 输出的文本文件统一使用 UTF-8（无 BOM）保存。
    pub normalize_to_utf8: bool,
    /// 整站模式下同一域名最多抓取的页面数，0 表示不限制。
    pub max_pages: usize,
    /// 整站模式下单页面最多追踪的外链数，0 表示不限制。
    pub max_links_per_page: usize,
}

impl Default for CloneOptions {
    fn default() -> Self {
        Self {
            start_uri: url::Url::parse("https://example.com/").expect("固定 URL 必然可解析"),
            output_directory: String::new(),
            mode: CloneMode::SinglePage,
            max_depth: 5,
            concurrency: 8,
            timeout_seconds: 30,
            retry_count: 3,
            user_agent: DEFAULT_USER_AGENT.to_owned(),
            proxy: ProxySettings::none(),
            ignore_certificate_errors: false,
            include_cross_origin_assets: true,
            enable_incremental: true,
            convert_to_relative_paths: true,
            neutralize_external_links: true,
            normalize_to_utf8: true,
            max_pages: 0,
            max_links_per_page: 0,
        }
    }
}
