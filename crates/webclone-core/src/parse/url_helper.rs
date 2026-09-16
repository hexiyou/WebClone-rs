//! URL 相关工具。

use crate::models::ResourceKind;
use url::Url;

use super::is_ascii_ws;

/// srcset 中的一个候选项。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SrcSetEntry {
    pub url: String,
    pub start: usize,
    pub length: usize,
}

/// 把可能相对的 URL 解析成绝对 URL。无法解析或属于无需下载的协议时返回 None。
pub fn try_make_absolute(value: &str, base: &Url) -> Option<Url> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return None;
    }

    if is_non_network_url(trimmed) {
        return None;
    }

    // 协议相对地址 //host/path 补上当前页面的协议
    let candidate = if let Some(rest) = trimmed.strip_prefix("//") {
        format!("{}://{}", base.scheme(), rest)
    } else {
        trimmed.to_owned()
    };

    let parsed = match Url::parse(&candidate) {
        Ok(u) => u,
        Err(url::ParseError::RelativeUrlWithoutBase) => base.join(trimmed).ok()?,
        Err(_) => return None,
    };

    if !is_http_scheme(&parsed) {
        return None;
    }

    Some(parsed)
}

/// javascript: / mailto: / tel: / data: / 锚点 等，不需要也不应该下载。
pub fn is_non_network_url(value: &str) -> bool {
    let trimmed = value.trim_start();
    if trimmed.is_empty() || trimmed.starts_with('#') {
        return true;
    }

    const PREFIXES: &[&str] = &[
        "javascript:",
        "mailto:",
        "tel:",
        "sms:",
        "data:",
        "about:",
        "blob:",
        "vbscript:",
    ];

    PREFIXES.iter().any(|p| {
        trimmed.len() >= p.len() && trimmed.as_bytes()[..p.len()].eq_ignore_ascii_case(p.as_bytes())
    })
}

pub fn is_http_scheme(uri: &Url) -> bool {
    matches!(uri.scheme(), "http" | "https")
}

/// 解析 srcset：`"a.png 1x, b.png 2x"` 或 `"a.png 480w, b.png 800w"`。
/// 返回每个候选项的 URL 及其在原始字符串中的位置，便于逐个替换。
pub fn parse_srcset(value: &str) -> Vec<SrcSetEntry> {
    let mut entries = Vec::new();
    if value.trim().is_empty() {
        return entries;
    }

    let bytes = value.as_bytes();
    let length = bytes.len();
    let mut i = 0usize;

    while i < length {
        while i < length && (is_ascii_ws(bytes[i]) || bytes[i] == b',') {
            i += 1;
        }
        if i >= length {
            break;
        }

        let url_start = i;
        while i < length && !is_ascii_ws(bytes[i]) && bytes[i] != b',' {
            i += 1;
        }

        let url_length = i - url_start;
        if url_length > 0 {
            entries.push(SrcSetEntry {
                url: value[url_start..url_start + url_length].to_owned(),
                start: url_start,
                length: url_length,
            });
        }

        // 跳过描述符（1x / 480w 等）
        while i < length && bytes[i] != b',' {
            i += 1;
        }
    }

    entries
}

/// 按 URL 路径猜测资源类型。
pub fn guess_from_path(uri: &Url) -> ResourceKind {
    let path = uri.path();
    let dot = match path.rfind('.') {
        Some(d) if d != path.len() - 1 => d,
        _ => return ResourceKind::Html,
    };

    let ext = path[dot + 1..].to_ascii_lowercase();
    match ext.as_str() {
        "css" => ResourceKind::Css,
        "js" | "mjs" | "cjs" => ResourceKind::JavaScript,
        "png" | "jpg" | "jpeg" | "gif" | "webp" | "avif" | "svg" | "ico" | "bmp" | "jfif"
        | "tiff" => ResourceKind::Image,
        "woff" | "woff2" | "ttf" | "otf" | "eot" => ResourceKind::Font,
        "mp4" | "webm" | "ogg" | "ogv" | "mp3" | "wav" | "m4a" | "flac" | "mov" | "avi"
        | "mkv" => ResourceKind::Media,
        "html" | "htm" | "shtml" | "xhtml" | "php" | "asp" | "aspx" | "jsp" => ResourceKind::Html,
        _ => ResourceKind::Other,
    }
}

/// 按 Content-Type 猜测资源类型，优先级高于路径猜测。
pub fn guess_from_content_type(content_type: Option<&str>) -> ResourceKind {
    let content_type = match content_type {
        Some(c) if !c.is_empty() => c,
        _ => return ResourceKind::Other,
    };

    let mime = content_type
        .split(';')
        .next()
        .unwrap_or("")
        .trim()
        .to_ascii_lowercase();

    if mime.starts_with("text/html") {
        return ResourceKind::Html;
    }
    if mime.starts_with("text/css") {
        return ResourceKind::Css;
    }
    if mime.contains("javascript") || mime.contains("ecmascript") {
        return ResourceKind::JavaScript;
    }
    if mime.starts_with("image/") {
        return ResourceKind::Image;
    }
    if mime.starts_with("font/") || mime.starts_with("application/font") || mime.contains("woff") {
        return ResourceKind::Font;
    }
    if mime.starts_with("video/") || mime.starts_with("audio/") {
        return ResourceKind::Media;
    }

    ResourceKind::Other
}

/// 去掉 URL 的 fragment，避免同一资源因锚点不同被重复下载。
pub fn without_fragment(uri: &Url) -> String {
    let mut u = uri.clone();
    u.set_fragment(None);
    u.to_string()
}
