//! URL 归一化。

use url::Url;

use super::url_helper::is_http_scheme;

/// 把 `/dir/index.html`、`/dir/index`、`/dir/` 归一成同一个地址。
///
/// 不做归一的话，起始页 `http://host/` 和页面里的 `http://host/index.html`
/// 会被当成两个页面下载两份，还会派生出 `index-2.html` 这种多余文件。
/// 带 query 的地址语义可能不同（如 `index.html?lang=en`），保持原样不归一。
pub fn normalize_index_url(uri: &Url) -> Url {
    // C# 里 Uri.Query 含前导 '?'，所以 "?": 单独一个问号不算有查询串
    if uri.query().is_some_and(|q| !q.is_empty()) || !is_http_scheme(uri) {
        return uri.clone();
    }

    let path = uri.path();
    if path.is_empty() || path == "/" {
        return uri.clone();
    }

    let last_slash = path.rfind('/').unwrap_or(0);
    let last = &path[last_slash + 1..];

    let is_index = matches!(
        last.to_ascii_lowercase().as_str(),
        "index" | "index.html" | "index.htm" | "default" | "default.html" | "default.htm"
    );

    if !is_index {
        return uri.clone();
    }

    let mut normalized = uri.clone();
    normalized.set_path(&path[..last_slash + 1]);
    normalized.set_fragment(None);
    normalized
}
