//! 改写文本里的编码声明。
//!
//! 文件统一以 UTF-8 落盘后，页面里原本写着的 `charset=gbk` / `@charset "GBK"`
//! 就会变成错误声明，浏览器照着它解就会乱码。所以转码之后必须把声明一起改掉。

use std::sync::OnceLock;

use regex::Regex;

/// 只在这些标签内部改写 charset=，避免误伤内联 JS 里的字符串。
fn tag_charset_regex() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(r#"(?i)(<(?:meta|script|link|a|style|form)\b[^>]*?charset\s*=\s*["']?)([A-Za-z0-9_\-:.]+)"#)
            .expect("内置正则必然可编译")
    })
}

fn css_charset_regex() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(r#"(?i)(@charset\s+["'])([^"']+)(["']\s*;)"#).expect("内置正则必然可编译")
    })
}

/// 把 HTML 中所有 meta / script / link 等标签上的 charset 声明改成 utf-8。
pub fn rewrite_html(html: &str) -> String {
    if html.is_empty() {
        return html.to_owned();
    }
    tag_charset_regex()
        .replace_all(html, "${1}utf-8")
        .into_owned()
}

/// 规范化 CSS 的编码声明：已有 @charset 就改成 UTF-8，没有则在开头补一条。
/// 存在非 ASCII 字符时才补，纯 ASCII 的 CSS 没必要多此一举。
pub fn rewrite_css(css: &str) -> String {
    if css.is_empty() {
        return css.to_owned();
    }

    let re = css_charset_regex();
    let had_charset = re.is_match(css);
    let rewritten = re.replacen(css, 1, "${1}UTF-8${3}").into_owned();

    if had_charset {
        return rewritten;
    }

    if !css.bytes().any(|b| b > 0x7F) {
        return css.to_owned();
    }

    format!("@charset \"UTF-8\";\n{}", css)
}
