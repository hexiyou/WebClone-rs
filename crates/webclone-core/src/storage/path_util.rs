//! 路径规范化。

use std::path::PathBuf;

/// 把 Git Bash / Cygwin 风格的路径转成 Windows 路径。
///
/// 例如 `/c/Users/Administrator` 在 Windows 上会被当成根相对路径解析成
/// `C:\c\Users\Administrator`（拼上当前盘符），结果完全不对。这里先做转换。
pub fn normalize_path(path: &str) -> String {
    let trimmed = path.trim();
    if trimmed.is_empty() {
        return trimmed.to_owned();
    }

    let bytes = trimmed.as_bytes();
    // /c/Users/... 或 /C/Users/...
    if bytes.len() >= 3 && bytes[0] == b'/' && bytes[1].is_ascii_alphabetic() && bytes[2] == b'/' {
        let drive = (bytes[1] as char).to_ascii_uppercase();
        let rest = trimmed[2..].replace('/', "\\");
        return format!("{}:{}", drive, rest);
    }

    trimmed.to_owned()
}

/// 规范化并转成绝对路径。
pub fn get_full_path(path: &str) -> PathBuf {
    let normalized = normalize_path(path);
    if normalized.is_empty() {
        return std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    }

    match std::path::absolute(&normalized) {
        Ok(p) => p,
        Err(_) => PathBuf::from(normalized),
    }
}
