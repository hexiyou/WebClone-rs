//! 文件名清洗。URL 里带 ? & = 之类的字符在 Windows 上是非法的，必须转换成合法文件名；
//! 同时处理保留设备名和控制字符。

/// Windows 非法字符。
const INVALID_CHARS: &[u8] = b"<>:\"/\\|?*";

const RESERVED_NAMES: &[&str] = &[
    "CON", "PRN", "AUX", "NUL", "COM1", "COM2", "COM3", "COM4", "COM5", "COM6", "COM7", "COM8",
    "COM9", "LPT1", "LPT2", "LPT3", "LPT4", "LPT5", "LPT6", "LPT7", "LPT8", "LPT9",
];

/// 清洗成合法文件名（不含目录分隔符）。
pub fn sanitize(raw_name: &str, fallback: &str) -> String {
    if raw_name.trim().is_empty() {
        return fallback.to_owned();
    }

    let mut out = String::with_capacity(raw_name.len());
    for ch in raw_name.chars() {
        if ch == '\0' {
            continue;
        }
        let code = ch as u32;
        if code < 0x20 || (ch.is_ascii() && INVALID_CHARS.contains(&(ch as u8))) {
            out.push('_');
        } else {
            out.push(ch);
        }
    }

    // 去掉结尾的点和空格——Windows 会自动忽略它们，可能导致两个文件名撞车
    let mut name = out.trim_end_matches(['.', ' ', '\t']).to_owned();

    if name.is_empty() {
        name = fallback.to_owned();
    }

    if RESERVED_NAMES
        .iter()
        .any(|r| r.eq_ignore_ascii_case(&name))
    {
        name.insert(0, '_');
    }

    // 单个文件名长度上限控制在 120，避开 MAX_PATH 的累计问题
    if name.chars().count() > 120 {
        name = name.chars().take(120).collect();
        name = name.trim_end_matches(['.', ' ']).to_owned();
    }

    name
}

/// 清洗路径的各个分段（用于 URL 路径转本地目录）。
pub fn sanitize_path_segment(segment: &str, fallback: &str) -> String {
    let cleaned = sanitize(segment, fallback);
    if cleaned.is_empty() {
        fallback.to_owned()
    } else {
        cleaned
    }
}

/// 判断扩展名是否"看起来正常"：1-10 个字母数字，避免把一长串随机串
/// 或整句查询参数当成扩展名。`ext` 形如 `.css`。
pub fn is_plausible_extension(ext: &str) -> bool {
    let bytes = ext.as_bytes();
    if bytes.len() < 2 || bytes.len() > 11 {
        return false;
    }

    bytes[1..].iter().all(|c| {
        c.is_ascii_lowercase() || c.is_ascii_uppercase() || c.is_ascii_digit() || *c == b'_' || *c == b'-'
    })
}
