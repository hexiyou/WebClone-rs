//! HTML 中一处 URL 引用的相关类型。

/// URL 属性的形态，决定后续如何解析这个值。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UrlAttributeKind {
    /// 单个 URL，如 `<img src="a.png">`。
    SingleUrl,
    /// srcset，形如 `"a.png 1x, b.png 2x"`。
    SrcSet,
    /// 内联 style 属性，内容是 CSS，需要二次扫描 url()。
    InlineStyle,
    /// `<style>` 标签的正文，内容是 CSS。
    StyleContent,
    /// `<meta http-equiv="refresh">` 的 content 里的 url=。
    RefreshUrl,
}

/// HTML 中一处 URL 引用。
#[derive(Debug, Clone)]
pub struct HtmlUrl {
    pub tag_name: String,
    pub attribute_name: String,
    /// 原文中的属性值（未解码实体）。
    pub raw_value: String,
    /// 解码实体后的值，用于构造绝对 URL。
    pub decoded_value: String,
    /// 属性值在原文中的起始字节下标（不含引号）。
    pub value_start: usize,
    /// 属性值在原文中的长度（不含引号）。
    pub value_length: usize,
    pub kind: UrlAttributeKind,
    /// `<link>` 标签的 rel 属性，小写。
    pub rel: Option<String>,
}

impl HtmlUrl {
    /// 是否是页面导航链接（a / area / frame 的 href），整站模式下才递归。
    pub fn is_navigation_link(&self) -> bool {
        matches!(self.tag_name.as_str(), "a" | "area" | "frame")
    }

    /// 这段值本身是 CSS 文本，需要交给 CSS 扫描器二次处理。
    pub fn is_style_text(&self) -> bool {
        matches!(
            self.kind,
            UrlAttributeKind::InlineStyle | UrlAttributeKind::StyleContent
        )
    }

    pub fn describe(&self) -> String {
        format!("<{} {}=\"{}\">", self.tag_name, self.attribute_name, self.decoded_value)
    }
}

/// 一段 CSS 文本在 HTML 原文中的位置。
#[derive(Debug, Clone, Copy)]
pub struct StyleRegion {
    pub start: usize,
    pub length: usize,
    /// true 表示来自 `style="..."` 属性，false 表示来自 `<style>` 标签正文。
    pub is_attribute: bool,
}
