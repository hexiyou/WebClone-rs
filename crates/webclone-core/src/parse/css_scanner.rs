//! CSS 扫描器。提取 url() 与 @import 引用的资源地址，并记录精确位置以便原地改写。
//!
//! 覆盖场景：背景图、@font-face 的 src、cursor 图、@import 嵌套样式表、
//! image-set()、border-image 等——它们本质上都是 url() 或 @import，统一处理即可。

use super::{find_sub, is_ascii_ws, matches_keyword};

/// CSS 中的一处 URL 引用。
#[derive(Debug, Clone)]
pub struct CssUrl {
    /// 原始 URL 文本。
    pub value: String,
    /// 在 CSS 文本中的起始字节下标。
    pub start: usize,
    /// 长度，含两侧引号（若原本带引号）。
    pub length: usize,
    /// 是否来自 @import。@import 进来的是样式表，不是普通资源。
    pub is_import: bool,
    /// URL 本身是否带引号，替换时需保持一致的写法。
    pub was_quoted: bool,
}

pub struct CssScanner;

impl CssScanner {
    pub fn scan(css: &str) -> Vec<CssUrl> {
        let mut results = Vec::new();
        if css.is_empty() {
            return results;
        }

        let bytes = css.as_bytes();
        let length = bytes.len();
        let mut i = 0usize;

        while i < length {
            let c = bytes[i];

            // 注释
            if c == b'/' && i + 1 < length && bytes[i + 1] == b'*' {
                match find_sub(bytes, b"*/", i + 2) {
                    Some(end) => i = end + 2,
                    None => i = length,
                }
                continue;
            }

            // url(
            if (c == b'u' || c == b'U') && matches_keyword(bytes, i, "url") {
                let mut paren_index = i + 3;
                while paren_index < length && is_ascii_ws(bytes[paren_index]) {
                    paren_index += 1;
                }

                if paren_index < length && bytes[paren_index] == b'(' {
                    if let Some((url, next)) = try_parse_url_function(css, paren_index) {
                        if !Self::is_skippable(&url.value) {
                            results.push(url);
                        }
                        i = next;
                        continue;
                    }

                    i = paren_index + 1;
                    continue;
                }
            }

            // @import
            if c == b'@' && matches_keyword(bytes, i, "@import") {
                let mut cursor = i + "@import".len();
                while cursor < length && is_ascii_ws(bytes[cursor]) {
                    cursor += 1;
                }

                if cursor < length {
                    if bytes[cursor] == b'"' || bytes[cursor] == b'\'' {
                        if let Some((value, start, len, after)) = try_read_quoted(css, cursor) {
                            if !Self::is_skippable(&value) {
                                results.push(CssUrl {
                                    value,
                                    start,
                                    length: len,
                                    is_import: true,
                                    was_quoted: true,
                                });
                            }
                            i = after;
                            continue;
                        }
                    } else if matches_keyword(bytes, cursor, "url") {
                        let mut paren_index = cursor + 3;
                        while paren_index < length && is_ascii_ws(bytes[paren_index]) {
                            paren_index += 1;
                        }

                        if paren_index < length && bytes[paren_index] == b'(' {
                            if let Some((url, next)) = try_parse_url_function(css, paren_index) {
                                if !Self::is_skippable(&url.value) {
                                    results.push(CssUrl {
                                        value: url.value,
                                        start: url.start,
                                        length: url.length,
                                        is_import: true,
                                        was_quoted: url.was_quoted,
                                    });
                                }
                                i = next;
                                continue;
                            }
                        }
                    }
                }
            }

            i += 1;
        }

        results
    }

    /// data: URI 之类的内联内容不需要下载。
    pub fn is_skippable(url: &str) -> bool {
        let trimmed = url.trim();
        if trimmed.is_empty() || trimmed == "#" {
            return true;
        }
        let b = trimmed.as_bytes();
        (b.len() >= 5 && b[..5].eq_ignore_ascii_case(b"data:"))
            || (b.len() >= 6 && b[..6].eq_ignore_ascii_case(b"about:"))
    }
}

/// 解析 `url(` 之后的内容，返回引用与下一个扫描位置。
fn try_parse_url_function(css: &str, open_paren: usize) -> Option<(CssUrl, usize)> {
    let bytes = css.as_bytes();
    let length = bytes.len();
    let mut i = open_paren + 1;
    while i < length && is_ascii_ws(bytes[i]) {
        i += 1;
    }
    if i >= length {
        return None;
    }

    if bytes[i] == b'"' || bytes[i] == b'\'' {
        let (value, start, len, after) = try_read_quoted(css, i)?;
        return Some((
            CssUrl {
                value,
                start,
                length: len,
                is_import: false,
                was_quoted: true,
            },
            after,
        ));
    }

    let value_start = i;
    while i < length && bytes[i] != b')' {
        i += 1;
    }
    if i >= length {
        return None;
    }

    let raw = css[value_start..i].trim();
    Some((
        CssUrl {
            value: raw.to_owned(),
            start: value_start,
            length: raw.len(),
            is_import: false,
            was_quoted: false,
        },
        i + 1,
    ))
}

/// 读取一段带引号的文本，返回 (内容, 起始下标含引号, 总长度含引号, 结束后下标)。
fn try_read_quoted(css: &str, quote_index: usize) -> Option<(String, usize, usize, usize)> {
    let bytes = css.as_bytes();
    let quote = bytes[quote_index];
    let mut i = quote_index + 1;
    let content_start = i;

    while i < bytes.len() {
        let c = bytes[i];
        if c == b'\\' {
            i += 2;
            continue;
        }
        if c == quote {
            let value = css[content_start..i].to_owned();
            return Some((value, quote_index, i - quote_index + 1, i + 1));
        }
        i += 1;
    }

    None
}
