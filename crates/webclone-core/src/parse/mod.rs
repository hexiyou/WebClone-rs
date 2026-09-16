pub mod css_scanner;
pub mod html_entity;
pub mod html_scanner;
pub mod html_url;
pub mod normalize;
pub mod url_helper;

/// ASCII 空白判定：HTML / CSS 的空白就是这几个，UTF-8 续字节恒 >= 0x80，
/// 不会被误判，因此可以直接在字节层做扫描。
#[inline]
pub fn is_ascii_ws(c: u8) -> bool {
    matches!(c, b' ' | b'\t' | b'\r' | b'\n' | 0x0C)
}

/// 大小写不敏感的子串查找（仅 ASCII 折叠），返回字节下标。
pub fn find_ci(haystack: &[u8], needle: &[u8], from: usize) -> Option<usize> {
    if needle.is_empty() || from >= haystack.len() {
        return None;
    }
    let first = needle[0].to_ascii_lowercase();
    let last_start = haystack.len().checked_sub(needle.len())?;
    let mut i = from;
    while i <= last_start {
        if haystack[i].to_ascii_lowercase() == first
            && haystack[i..i + needle.len()]
                .iter()
                .zip(needle)
                .all(|(a, b)| a.to_ascii_lowercase() == b.to_ascii_lowercase())
        {
            return Some(i);
        }
        i += 1;
    }
    None
}

/// 从 `from` 起找第一个字节。
#[inline]
pub fn find_byte(haystack: &[u8], byte: u8, from: usize) -> Option<usize> {
    if from >= haystack.len() {
        return None;
    }
    haystack[from..].iter().position(|c| *c == byte).map(|p| p + from)
}

/// 大小写敏感的子串查找，返回字节下标。
pub fn find_sub(haystack: &[u8], needle: &[u8], from: usize) -> Option<usize> {
    if needle.is_empty() || from >= haystack.len() {
        return None;
    }
    let last = haystack.len().checked_sub(needle.len())?;
    let mut i = from;
    while i <= last {
        if &haystack[i..i + needle.len()] == needle {
            return Some(i);
        }
        i += 1;
    }
    None
}

/// 大小写不敏感的 ASCII 关键字匹配。
pub fn matches_keyword(text: &[u8], index: usize, keyword: &str) -> bool {
    let kw = keyword.as_bytes();
    if index + kw.len() > text.len() {
        return false;
    }
    text[index..index + kw.len()]
        .iter()
        .zip(kw)
        .all(|(a, b)| a.to_ascii_lowercase() == b.to_ascii_lowercase())
}
