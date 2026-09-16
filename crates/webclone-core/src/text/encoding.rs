//! 网页编码识别。
//!
//! 判定优先级与浏览器一致：
//!   1. BOM（UTF-8 / UTF-16LE / UTF-16BE / UTF-32）——最高优先级
//!   2. HTTP 响应头 Content-Type 里的 charset
//!   3. HTML 的 `<meta charset>` 或 `<meta http-equiv="Content-Type">`、
//!      CSS 文件开头的 `@charset`
//!   4. 二进制嗅探：整段按严格 UTF-8 解码，失败说明不是 UTF-8
//!   5. 兜底按 GB18030 解码（GB18030 是 GBK / GB2312 的超集，国内站点基本都是它）
//!
//! 之所以要自己嗅探：大量国内站点要么不声明 charset，要么声明与实际不符，
//! 只认声明会导致中文乱码。

use std::sync::OnceLock;

use encoding_rs::{Encoding, UTF_8};
use regex::Regex;

/// 嗅探时只读取文件开头这么多字节，足够覆盖 head 里的 charset 声明。
const SNIFF_LEN: usize = 8192;

/// 检测出来的文本编码。UTF-32 不在 Encoding Standard 里，单独处理。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TextEncoding {
    Rs(&'static Encoding),
    Utf32Le,
    Utf32Be,
}

impl TextEncoding {
    pub fn encode(&self, text: &str) -> Vec<u8> {
        match self {
            TextEncoding::Rs(enc) => enc.encode(text).0.into_owned(),
            TextEncoding::Utf32Le => text
                .chars()
                .flat_map(|c| (c as u32).to_le_bytes())
                .collect(),
            TextEncoding::Utf32Be => text
                .chars()
                .flat_map(|c| (c as u32).to_be_bytes())
                .collect(),
        }
    }

    pub fn name(&self) -> &'static str {
        match self {
            TextEncoding::Rs(enc) => enc.name(),
            TextEncoding::Utf32Le => "UTF-32LE",
            TextEncoding::Utf32Be => "UTF-32BE",
        }
    }
}

#[derive(Debug, Clone)]
pub struct DecodedText {
    pub text: String,
    pub encoding: TextEncoding,
    pub had_bom: bool,
}

/// 按名称获取编码，无法识别时返回 None 而不是报错。
pub fn try_get_encoding(name: &str) -> Option<&'static Encoding> {
    let normalized = name.trim().trim_matches(['"', '\'']).trim();
    if normalized.is_empty() {
        return None;
    }
    Encoding::for_label(normalized.as_bytes())
}

/// 解码字节序列，自动识别编码。
pub fn decode(bytes: &[u8], http_charset: Option<&str>, assume_html: bool) -> DecodedText {
    // 1) BOM
    if let Some((enc, bom_len)) = detect_bom(bytes) {
        let body = &bytes[bom_len..];
        let text = match enc {
            TextEncoding::Rs(e) => e.decode(body).0.into_owned(),
            TextEncoding::Utf32Le => decode_utf32(body, true),
            TextEncoding::Utf32Be => decode_utf32(body, false),
        };
        return DecodedText {
            text,
            encoding: enc,
            had_bom: bom_len > 0,
        };
    }

    // 2) 用 Latin1 无损地把开头一段读成文本，便于从中找 charset 声明
    let head = latin1_to_string(&bytes[..SNIFF_LEN.min(bytes.len())]);

    // 3) HTML meta / CSS @charset 声明
    let mut declared = if assume_html {
        extract_html_charset(&head)
    } else {
        extract_css_charset(&head)
    };
    if declared.is_none() && !assume_html {
        // CSS 被当成 HTML 处理时可能漏掉，回退一次
        declared = extract_html_charset(&head);
    }

    // 4) 页面内声明优先试，再试 HTTP 头声明——两者都试，能解出合法文本就用
    let mut candidates: Vec<&str> = Vec::new();
    if let Some(name) = &declared {
        candidates.push(name.as_str());
    }
    if let Some(http) = http_charset {
        candidates.push(http);
    }

    for label in candidates {
        if let Some(text) = try_candidate(bytes, label) {
            let enc = try_get_encoding(label).unwrap_or(UTF_8);
            return DecodedText {
                text,
                encoding: TextEncoding::Rs(enc),
                had_bom: false,
            };
        }
    }

    // 5) 严格 UTF-8 嗅探
    if let Ok(text) = std::str::from_utf8(bytes) {
        return DecodedText {
            text: text.to_owned(),
            encoding: TextEncoding::Rs(UTF_8),
            had_bom: false,
        };
    }

    // 6) 兜底 GB18030（超集，覆盖 GBK / GB2312）
    let gb = Encoding::for_label(b"gb18030").unwrap_or(encoding_rs::WINDOWS_1252);
    DecodedText {
        text: gb.decode(bytes).0.into_owned(),
        encoding: TextEncoding::Rs(gb),
        had_bom: false,
    }
}

fn detect_bom(bytes: &[u8]) -> Option<(TextEncoding, usize)> {
    if bytes.len() >= 4 && bytes[0] == 0x00 && bytes[1] == 0x00 && bytes[2] == 0xFE && bytes[3] == 0xFF
    {
        return Some((TextEncoding::Utf32Be, 4));
    }
    if bytes.len() >= 4 && bytes[0] == 0xFF && bytes[1] == 0xFE && bytes[2] == 0x00 && bytes[3] == 0x00
    {
        return Some((TextEncoding::Utf32Le, 4));
    }
    if let Some((enc, len)) = Encoding::for_bom(bytes) {
        return Some((TextEncoding::Rs(enc), len));
    }
    None
}

fn decode_utf32(bytes: &[u8], little_endian: bool) -> String {
    let mut out = String::with_capacity(bytes.len() / 4);
    let mut i = 0;
    while i + 3 < bytes.len() {
        let code = if little_endian {
            u32::from_le_bytes([bytes[i], bytes[i + 1], bytes[i + 2], bytes[i + 3]])
        } else {
            u32::from_be_bytes([bytes[i], bytes[i + 1], bytes[i + 2], bytes[i + 3]])
        };
        out.push(char::from_u32(code).unwrap_or('\u{FFFD}'));
        i += 4;
    }
    out
}

/// 严格解码：只要出现非法字节就判失败，用于"声明的编码是不是真能解出合法文本"的试探。
fn try_decode_strict(bytes: &[u8], encoding: &'static Encoding) -> Option<String> {
    if encoding == UTF_8 {
        return std::str::from_utf8(bytes).ok().map(|s| s.to_owned());
    }

    let mut decoder = encoding.new_decoder_without_bom_handling();
    let mut out = String::with_capacity(bytes.len());
    let (result, _read) =
        decoder.decode_to_string_without_replacement(bytes, &mut out, true);

    match result {
        encoding_rs::DecoderResult::Malformed(_, _) => None,
        _ => Some(out),
    }
}

/// 试探性解码。
///
/// C# 版对 Latin1 候选直接跳过——它任何字节都能解开，必然产出乱码；
/// ASCII 候选则要求全部字节 < 0x80（.NET 的 ASCII 用异常回退，遇到高位字节就失败）。
fn try_candidate(bytes: &[u8], label: &str) -> Option<String> {
    let normalized = label
        .trim()
        .trim_matches(['"', '\''])
        .trim()
        .to_ascii_lowercase();

    match normalized.as_str() {
        "iso-8859-1" | "latin1" | "latin-1" | "latin_1" | "iso8859-1" => return None,
        "ascii" | "us-ascii" => {
            return if bytes.iter().all(|b| *b < 0x80) {
                Some(bytes.iter().map(|b| *b as char).collect())
            } else {
                None
            };
        }
        _ => {}
    }

    try_decode_strict(bytes, try_get_encoding(&normalized)?)
}

/// 把字节按 ISO-8859-1 无损映射成字符串（每个字节 → 同码点字符）。
fn latin1_to_string(bytes: &[u8]) -> String {
    bytes.iter().map(|b| *b as char).collect()
}

fn regex_cell(slot: &'static OnceLock<Regex>, pattern: &str) -> &'static Regex {
    slot.get_or_init(|| Regex::new(pattern).expect("内置正则必然可编译"))
}

fn meta_tag_regex() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    regex_cell(&RE, r"(?is)<meta\s[^>]*>")
}

fn charset_attribute_regex() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    regex_cell(
        &RE,
        r#"(?i)charset\s*=\s*["']?\s*(?P<name>[A-Za-z0-9_\-:.]+)"#,
    )
}

fn content_attribute_regex() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    regex_cell(&RE, r#"(?i)content\s*=\s*["'](?P<value>[^"']*)["']"#)
}

fn css_charset_regex() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    regex_cell(&RE, r#"(?i)^\s*@charset\s+["'](?P<name>[^"']+)["']\s*;"#)
}

/// 从 HTML head 里找 charset 声明。
/// 优先取独立的 charset 属性，其次从 http-equiv=Content-Type 的 content 里解析。
pub fn extract_html_charset(head: &str) -> Option<String> {
    let charset_re = charset_attribute_regex();

    for tag_match in meta_tag_regex().find_iter(head) {
        let tag = tag_match.as_str();
        let lower = tag.to_ascii_lowercase();
        let is_content_type = lower.contains("http-equiv") && lower.contains("content-type");

        let mut has_standalone_charset = false;
        for m in charset_re.captures_iter(tag) {
            let name = &m["name"];
            if is_content_type && !name.starts_with("text/") && !name.is_empty() {
                return Some(name.to_owned());
            }
            if !is_content_type {
                has_standalone_charset = true;
            }
        }

        if !is_content_type && has_standalone_charset {
            if let Some(m) = charset_re.captures(tag) {
                return Some(m["name"].to_owned());
            }
        }

        if is_content_type {
            if let Some(cm) = content_attribute_regex().captures(tag) {
                if let Some(inner) = charset_re.captures(&cm["value"]) {
                    return Some(inner["name"].to_owned());
                }
            }
        }
    }

    None
}

/// CSS 的 @charset 必须出现在文件最开头，否则无效。
pub fn extract_css_charset(head: &str) -> Option<String> {
    css_charset_regex()
        .captures(head)
        .map(|m| m["name"].to_owned())
}
