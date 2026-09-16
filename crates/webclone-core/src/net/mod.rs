pub mod downloader;
pub mod http;

use chrono::{DateTime, FixedOffset, TimeZone, Utc};

/// 把时间格式化成 HTTP 日期（IMF-fixdate / RFC 1123），
/// 与 .NET 的 DateTimeOffset.ToString("R") 输出一致。
pub fn format_http_date(value: &DateTime<FixedOffset>) -> String {
    value
        .with_timezone(&Utc)
        .format("%a, %d %b %Y %H:%M:%S GMT")
        .to_string()
}

/// 解析 HTTP 日期。兼容 "Wed, 16 Sep 2026 10:57:42 GMT" 与 "+0800" 两种写法。
pub fn parse_http_date(value: &str) -> Option<DateTime<FixedOffset>> {
    let trimmed = value.trim();

    if let Ok(parsed) = DateTime::parse_from_rfc2822(trimmed) {
        return Some(parsed);
    }

    let normalized = if let Some(stripped) = trimmed.strip_suffix(" GMT") {
        format!("{} +0000", stripped)
    } else if let Some(stripped) = trimmed.strip_suffix(" UTC") {
        format!("{} +0000", stripped)
    } else {
        trimmed.to_owned()
    };

    if let Ok(parsed) = DateTime::parse_from_rfc2822(&normalized) {
        return Some(parsed);
    }

    if let Ok(naive) = chrono::NaiveDateTime::parse_from_str(trimmed, "%a, %d %b %Y %H:%M:%S") {
        return Some(Utc.from_utc_datetime(&naive).fixed_offset());
    }

    None
}
