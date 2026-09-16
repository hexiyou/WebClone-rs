//! 轻量 HTML 扫描器。
//!
//! 之所以不用 html5ever / scraper 之类的库：这个工具只需要"定位 URL 属性并精确改写"，
//! 不需要构建完整 DOM 树，自己实现状态机反而更可控——尤其是必须保留原文下标，
//! 才能做原地替换而不破坏格式。
//!
//! 扫描时会正确跳过：HTML 注释、CDATA、DOCTYPE、处理指令、script 正文、textarea 正文。
//!
//! 与 C# 版的唯一实现差异：C# 用 UTF-16 下标，这里用 UTF-8 字节下标。
//! 因为所有定界符都是 ASCII，字节下标永远落在字符边界上，语义完全等价。

use super::html_entity;
use super::html_url::{HtmlUrl, StyleRegion, UrlAttributeKind};
use super::{find_byte, find_ci, find_sub, is_ascii_ws};

#[derive(Debug, Default)]
pub struct HtmlDocument {
    pub urls: Vec<HtmlUrl>,
    pub style_regions: Vec<StyleRegion>,
    /// `<base href>`，会影响页面内所有相对 URL 的解析基准。
    pub base_href: Option<String>,
    pub title: Option<String>,
}

pub struct HtmlScanner;

impl HtmlScanner {
    pub fn scan(html: &str) -> HtmlDocument {
        let mut doc = HtmlDocument::default();
        if html.is_empty() {
            return doc;
        }

        let scan = Scanner {
            html,
            bytes: html.as_bytes(),
            len: html.len(),
            i: 0,
        };
        scan.run(&mut doc);
        doc
    }
}

struct Scanner<'a> {
    html: &'a str,
    bytes: &'a [u8],
    len: usize,
    i: usize,
}

#[derive(Debug, Clone)]
struct Attribute {
    name: String,
    value: String,
    value_start: usize,
    value_length: usize,
}

impl Scanner<'_> {
    fn run(mut self, doc: &mut HtmlDocument) {
        while self.i < self.len {
            let lt = match find_byte(self.bytes, b'<', self.i) {
                Some(p) => p,
                None => break,
            };
            self.i = lt;

            if self.starts_with(b"<!--") {
                match find_sub(self.bytes, b"-->", self.i + 4) {
                    Some(end) => self.i = end + 3,
                    None => self.i = self.len,
                }
                continue;
            }

            if self.starts_with(b"<![CDATA[") {
                match find_sub(self.bytes, b"]]>", self.i + 9) {
                    Some(end) => self.i = end + 3,
                    None => self.i = self.len,
                }
                continue;
            }

            if self.i + 1 < self.len && self.bytes[self.i + 1] == b'!' {
                // DOCTYPE 及其他声明
                match find_byte(self.bytes, b'>', self.i + 2) {
                    Some(end) => self.i = end + 1,
                    None => self.i = self.len,
                }
                continue;
            }

            if self.i + 1 < self.len && self.bytes[self.i + 1] == b'?' {
                match find_sub(self.bytes, b"?>", self.i + 2) {
                    Some(end) => self.i = end + 2,
                    None => self.i = self.len,
                }
                continue;
            }

            if self.i + 1 < self.len && self.bytes[self.i + 1] == b'/' {
                match find_byte(self.bytes, b'>', self.i + 2) {
                    Some(end) => self.i = end + 1,
                    None => self.i = self.len,
                }
                continue;
            }

            self.i += 1;
            let name_start = self.i;
            while self.i < self.len
                && !is_ascii_ws(self.bytes[self.i])
                && self.bytes[self.i] != b'>'
                && self.bytes[self.i] != b'/'
            {
                self.i += 1;
            }

            if name_start == self.i {
                continue;
            }

            let tag_name = self.html[name_start..self.i].to_ascii_lowercase();
            let attributes = self.parse_attributes();

            self.process_attributes(doc, &tag_name, &attributes);

            match tag_name.as_str() {
                "script" => self.skip_raw_text("script"),
                "style" => self.capture_style_content(doc),
                "title" => self.capture_title(doc),
                "textarea" => self.skip_raw_text("textarea"),
                _ => {}
            }
        }
    }

    fn starts_with(&self, pattern: &[u8]) -> bool {
        self.i + pattern.len() <= self.len && &self.bytes[self.i..self.i + pattern.len()] == pattern
    }

    fn parse_attributes(&mut self) -> Vec<Attribute> {
        let mut attributes = Vec::with_capacity(4);

        while self.i < self.len {
            while self.i < self.len && is_ascii_ws(self.bytes[self.i]) {
                self.i += 1;
            }

            if self.i >= self.len || self.bytes[self.i] == b'>' {
                if self.i < self.len {
                    self.i += 1;
                }
                break;
            }

            // 自闭合标签的斜杠
            if self.bytes[self.i] == b'/' {
                self.i += 1;
                if self.i < self.len && self.bytes[self.i] == b'>' {
                    self.i += 1;
                    break;
                }
                continue;
            }

            let name_start = self.i;
            while self.i < self.len
                && !is_ascii_ws(self.bytes[self.i])
                && self.bytes[self.i] != b'='
                && self.bytes[self.i] != b'>'
                && self.bytes[self.i] != b'/'
            {
                self.i += 1;
            }

            if name_start == self.i {
                self.i += 1;
                continue;
            }

            let name = self.html[name_start..self.i].to_ascii_lowercase();

            // 跳过空白，寻找 = 号
            let save = self.i;
            while self.i < self.len && is_ascii_ws(self.bytes[self.i]) {
                self.i += 1;
            }

            if self.i >= self.len || self.bytes[self.i] != b'=' {
                // 无值属性
                attributes.push(Attribute {
                    name,
                    value: String::new(),
                    value_start: save,
                    value_length: 0,
                });
                self.i = save;
                continue;
            }

            self.i += 1;
            while self.i < self.len && is_ascii_ws(self.bytes[self.i]) {
                self.i += 1;
            }
            if self.i >= self.len {
                break;
            }

            let quote = self.bytes[self.i];
            if quote == b'"' || quote == b'\'' {
                self.i += 1;
                let value_start = self.i;
                let end = match find_byte(self.bytes, quote, self.i) {
                    Some(e) => {
                        self.i = e + 1;
                        e
                    }
                    None => {
                        self.i = self.len;
                        self.len
                    }
                };
                attributes.push(Attribute {
                    name,
                    value: self.html[value_start..end].to_owned(),
                    value_start,
                    value_length: end - value_start,
                });
            } else {
                let value_start = self.i;
                while self.i < self.len
                    && !is_ascii_ws(self.bytes[self.i])
                    && self.bytes[self.i] != b'>'
                {
                    self.i += 1;
                }
                attributes.push(Attribute {
                    name,
                    value: self.html[value_start..self.i].to_owned(),
                    value_start,
                    value_length: self.i - value_start,
                });
            }
        }

        attributes
    }

    fn process_attributes(
        &mut self,
        doc: &mut HtmlDocument,
        tag_name: &str,
        attributes: &[Attribute],
    ) {
        let mut rel: Option<String> = None;
        let mut http_equiv: Option<String> = None;
        let mut refresh_content: Option<&Attribute> = None;

        for attribute in attributes {
            if attribute.name == "rel" {
                rel = Some(attribute.value.to_ascii_lowercase());
            } else if attribute.name == "http-equiv" {
                http_equiv = Some(attribute.value.to_ascii_lowercase());
            } else if attribute.name == "content" && tag_name == "meta" {
                refresh_content = Some(attribute);
            }
        }

        for attribute in attributes {
            let kind = match resolve_kind(tag_name, &attribute.name, &attribute.value) {
                Some(k) => k,
                None => continue,
            };

            let raw = attribute.value.clone();
            let decoded = html_entity::decode(&raw);

            if tag_name == "base" && attribute.name == "href" {
                // base 会影响页面内所有相对 URL 的解析基准，记录下来。
                // 改写时把它的 href 替换成 "."，重写后的相对路径才不会被带偏。
                doc.base_href = Some(decoded.trim().to_owned());
            }

            if attribute.name == "content" && tag_name == "meta" {
                continue; // meta 的 content 只在 refresh 场景下处理
            }

            if attribute.name == "style" {
                doc.style_regions.push(StyleRegion {
                    start: attribute.value_start,
                    length: attribute.value_length,
                    is_attribute: true,
                });
                doc.urls.push(HtmlUrl {
                    tag_name: tag_name.to_owned(),
                    attribute_name: attribute.name.clone(),
                    raw_value: raw,
                    decoded_value: decoded,
                    value_start: attribute.value_start,
                    value_length: attribute.value_length,
                    kind: UrlAttributeKind::InlineStyle,
                    rel: rel.clone(),
                });
                continue;
            }

            if attribute.name == "srcset"
                && kind == UrlAttributeKind::SrcSet
                && decoded.trim().is_empty()
            {
                continue;
            }

            doc.urls.push(HtmlUrl {
                tag_name: tag_name.to_owned(),
                attribute_name: attribute.name.clone(),
                raw_value: raw,
                decoded_value: decoded,
                value_start: attribute.value_start,
                value_length: attribute.value_length,
                kind,
                rel: rel.clone(),
            });
        }

        // <meta http-equiv="refresh" content="0;url=http://...">
        if http_equiv.as_deref() == Some("refresh") {
            if let Some(content) = refresh_content {
                Self::add_refresh_url(doc, tag_name, content);
            }
        }
    }

    fn add_refresh_url(doc: &mut HtmlDocument, tag_name: &str, content: &Attribute) {
        let value = html_entity::decode(&content.value);
        let marker_index = match value.to_ascii_lowercase().find("url=") {
            Some(p) => p,
            None => return,
        };

        let mut url_start = marker_index + 4;
        let value_bytes = value.as_bytes();
        while url_start < value_bytes.len() && is_ascii_ws(value_bytes[url_start]) {
            url_start += 1;
        }

        let url = value[url_start..].trim().trim_matches('\'');
        if url.is_empty() {
            return;
        }
        let url_owned = url.to_owned();
        let url_length = url.len();

        doc.urls.push(HtmlUrl {
            tag_name: tag_name.to_owned(),
            attribute_name: "content".to_owned(),
            raw_value: value,
            decoded_value: url_owned,
            value_start: content.value_start + url_start,
            value_length: url_length,
            kind: UrlAttributeKind::RefreshUrl,
            rel: None,
        });
    }

    fn skip_raw_text(&mut self, tag_name: &str) {
        let needle = format!("</{}", tag_name);
        match find_ci(self.bytes, needle.as_bytes(), self.i) {
            Some(close) => match find_byte(self.bytes, b'>', close) {
                Some(end) => self.i = end + 1,
                None => self.i = self.len,
            },
            None => self.i = self.len,
        }
    }

    fn capture_style_content(&mut self, doc: &mut HtmlDocument) {
        let start = self.i;
        let close = find_ci(self.bytes, b"</style", self.i);
        let end = close.unwrap_or(self.len);

        if end > start {
            doc.style_regions.push(StyleRegion {
                start,
                length: end - start,
                is_attribute: false,
            });
            doc.urls.push(HtmlUrl {
                tag_name: "style".to_owned(),
                attribute_name: "#text".to_owned(),
                raw_value: self.html[start..end].to_owned(),
                decoded_value: self.html[start..end].to_owned(),
                value_start: start,
                value_length: end - start,
                kind: UrlAttributeKind::StyleContent,
                rel: None,
            });
        }

        self.i = match close.and_then(|c| find_byte(self.bytes, b'>', c)) {
            Some(tag_end) => tag_end + 1,
            None => self.len,
        };
    }

    fn capture_title(&mut self, doc: &mut HtmlDocument) {
        let close = find_ci(self.bytes, b"</title", self.i);
        let end = close.unwrap_or(self.len);
        if end > self.i {
            let title = self.html[self.i..end].trim();
            if !title.is_empty() {
                doc.title = Some(html_entity::decode(title));
            }
        }

        self.i = match close.and_then(|c| find_byte(self.bytes, b'>', c)) {
            Some(tag_end) => tag_end + 1,
            None => self.len,
        };
    }
}

/// 判断某个属性是否承载 URL，以及它的形态。
fn resolve_kind(tag_name: &str, attribute_name: &str, value: &str) -> Option<UrlAttributeKind> {
    if attribute_name == "style" {
        return Some(UrlAttributeKind::InlineStyle);
    }

    if attribute_name == "srcset" && matches!(tag_name, "img" | "source") {
        return Some(UrlAttributeKind::SrcSet);
    }

    if let Some(kind) = known_attribute_kind(tag_name, attribute_name) {
        return Some(kind);
    }

    // 懒加载站点大量使用 data-src / data-original / data-bg 之类自定义属性，
    // 不处理的话图片根本下不下来。
    if let Some(suffix) = attribute_name.strip_prefix("data-") {
        let decoded = html_entity::decode(value);
        if !looks_like_url_like(&decoded) {
            return None;
        }

        if suffix.contains("srcset") {
            return Some(UrlAttributeKind::SrcSet);
        }

        if suffix.contains("src")
            || suffix.contains("href")
            || suffix.contains("url")
            || suffix.contains("bg")
            || suffix.contains("image")
            || suffix.contains("img")
            || suffix.contains("original")
            || suffix.contains("lazy")
        {
            return Some(UrlAttributeKind::SingleUrl);
        }
    }

    None
}

/// 已知的 (标签, 属性) → URL 形态映射，与 C# 版的字典逐条一一对应。
///
/// 注意这里是精确配对，不能按"标签集合 × 属性集合"放量匹配：
/// 例如 `a:src`、`img:href`、`link:src` 都不在表内，多匹配会多抓一堆无关属性。
const KNOWN_ATTRIBUTES: &[(&str, &str, UrlAttributeKind)] = &[
    ("a", "href", UrlAttributeKind::SingleUrl),
    ("area", "href", UrlAttributeKind::SingleUrl),
    ("frame", "src", UrlAttributeKind::SingleUrl),
    ("iframe", "src", UrlAttributeKind::SingleUrl),
    ("link", "href", UrlAttributeKind::SingleUrl),
    ("base", "href", UrlAttributeKind::SingleUrl),
    ("script", "src", UrlAttributeKind::SingleUrl),
    ("img", "src", UrlAttributeKind::SingleUrl),
    ("img", "srcset", UrlAttributeKind::SrcSet),
    ("source", "src", UrlAttributeKind::SingleUrl),
    ("source", "srcset", UrlAttributeKind::SrcSet),
    ("video", "src", UrlAttributeKind::SingleUrl),
    ("video", "poster", UrlAttributeKind::SingleUrl),
    ("audio", "src", UrlAttributeKind::SingleUrl),
    ("track", "src", UrlAttributeKind::SingleUrl),
    ("embed", "src", UrlAttributeKind::SingleUrl),
    ("object", "data", UrlAttributeKind::SingleUrl),
    ("input", "src", UrlAttributeKind::SingleUrl),
    ("body", "background", UrlAttributeKind::SingleUrl),
    ("table", "background", UrlAttributeKind::SingleUrl),
    ("tr", "background", UrlAttributeKind::SingleUrl),
    ("td", "background", UrlAttributeKind::SingleUrl),
];

fn known_attribute_kind(tag_name: &str, attribute_name: &str) -> Option<UrlAttributeKind> {
    KNOWN_ATTRIBUTES
        .iter()
        .find(|(tag, attr, _)| *tag == tag_name && *attr == attribute_name)
        .map(|(_, _, kind)| *kind)
}

/// 粗略判断这个值长得像不像一个 URL，用于过滤 data-* 里的杂项属性。
pub fn looks_like_url_like(value: &str) -> bool {
    let trimmed = value.trim();
    if trimmed.is_empty() || trimmed.len() > 2048 {
        return false;
    }

    let lower = trimmed.to_ascii_lowercase();
    if lower.starts_with("http://")
        || lower.starts_with("https://")
        || trimmed.starts_with("//")
        || trimmed.starts_with('/')
        || trimmed.starts_with("./")
        || trimmed.starts_with("../")
    {
        return true;
    }

    // 相对路径且像文件名（带扩展名、不含空格）
    if trimmed.contains('.') && !trimmed.contains(' ') && !trimmed.contains('\n') {
        let last_segment = match trimmed.rfind('/') {
            Some(slash) => &trimmed[slash + 1..],
            None => trimmed,
        };

        if let Some(dot) = last_segment.rfind('.') {
            let ext_len = last_segment.len() - dot - 1;
            if dot > 0 && (1..=10).contains(&ext_len) {
                return true;
            }
        }
    }

    false
}
