//! 把 URL 映射成本地文件路径。
//!
//! 映射规则（wget -E 风格，但针对 Windows 文件名合法性做了适配）：
//!   `example.com/`                  -> `example.com/index.html`
//!   `example.com/a/b`               -> `example.com/a/b.html`        （无扩展名时按类型补）
//!   `example.com/a/b/`              -> `example.com/a/b/index.html`
//!   `example.com/a/b.css`           -> `example.com/a/b.css`
//!   `example.com/a/b.css?v=1&x=2`   -> `example.com/a/b_3f7a9c21.css`（查询参数哈希化）
//!
//! 非默认端口会体现在目录名上（`example.com_8080`），避免同一 host 不同端口撞车。
//! 跨域资源按各自的 host 单独归档，因此 CDN 上的文件也会落到 `cdn.xxx.com/...` 下。

use std::collections::{HashMap, HashSet};
use std::path::{Component, Path, PathBuf};
use std::sync::Mutex;

use sha2::{Digest, Sha256};
use url::Url;

use super::path_util;
use super::safe_name;
use crate::models::ResourceKind;

/// URL 映射后的本地落盘目标。
#[derive(Debug, Clone)]
pub struct LocalTarget {
    pub absolute_path: PathBuf,
    /// 相对保存根目录的路径，用 / 分隔。
    pub relative_path: String,
    /// 文件所在目录（绝对）。
    pub directory: PathBuf,
    pub file_name: String,
}

impl std::fmt::Display for LocalTarget {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.relative_path)
    }
}

pub struct PathMapper {
    root: PathBuf,
    claimed: Mutex<HashSet<String>>,
    by_url: Mutex<HashMap<String, LocalTarget>>,
}

impl PathMapper {
    pub fn new(output_root: &str) -> Self {
        Self {
            root: path_util::get_full_path(output_root),
            claimed: Mutex::new(HashSet::new()),
            by_url: Mutex::new(HashMap::new()),
        }
    }

    /// 保存根目录的绝对路径。
    pub fn root(&self) -> &Path {
        &self.root
    }

    fn host_directory(url: &Url) -> String {
        let host = url.host_str().unwrap_or("unknown");
        // url::Url::port() 仅在显式指定且非默认端口时返回 Some，
        // 与 .NET 的 !IsDefaultPort 语义一致。
        let name = match url.port() {
            Some(port) => format!("{}_{}", host, port),
            None => host.to_owned(),
        };
        safe_name::sanitize_path_segment(&name, "_host")
    }

    /// 计算 URL 对应的本地目标。同一 URL 多次调用返回同一结果。
    pub fn map(&self, url: &Url, kind: ResourceKind) -> LocalTarget {
        let key = canonical_url_key(url);
        {
            let cache = self.by_url.lock().expect("映射表加锁失败");
            if let Some(hit) = cache.get(&key) {
                return hit.clone();
            }
        }

        let target = self.build(url, kind);
        self.by_url
            .lock()
            .expect("映射表加锁失败")
            .insert(key, target.clone());
        target
    }

    /// 下载后按真实 Content-Type 修正资源类型并重新映射。
    ///
    /// 典型场景：Google Fonts 的动态样式表（`/css?family=...`）URL 无扩展名，
    /// 初次映射按 HTML 存成 `.html`，浏览器会因 MIME 类型不是 text/css 拒绝应用。
    pub fn retarget(&self, url: &Url, kind: ResourceKind) -> LocalTarget {
        let key = canonical_url_key(url);
        {
            let cache = self.by_url.lock().expect("映射表加锁失败");
            if let Some(existing) = cache.get(&key) {
                let existing_ext = Path::new(&existing.relative_path)
                    .extension()
                    .map(|e| format!(".{}", e.to_string_lossy()))
                    .unwrap_or_default();
                if existing_ext.eq_ignore_ascii_case(default_extension(kind)) {
                    return existing.clone();
                }
            }
        }

        let target = self.build(url, kind);
        self.by_url
            .lock()
            .expect("映射表加锁失败")
            .insert(key, target.clone());
        target
    }

    fn build(&self, url: &Url, kind: ResourceKind) -> LocalTarget {
        let mut segments: Vec<String> = vec![Self::host_directory(url)];

        // 逐段解码可以避免 %2F 被还原成路径分隔符造成穿越
        let raw_segments: Vec<String> = url
            .path_segments()
            .map(|it| it.map(|s| s.to_owned()).collect())
            .unwrap_or_default();

        for seg in &raw_segments {
            if seg.is_empty() {
                continue;
            }
            let decoded = percent_encoding::percent_decode_str(seg)
                .decode_utf8_lossy()
                .into_owned();
            segments.push(safe_name::sanitize_path_segment(&decoded, "_"));
        }

        // 最后一段作为文件名；URL 以 / 结尾则视为目录
        let ends_with_slash = url.path().ends_with('/');
        let file_name_candidate = if ends_with_slash || segments.len() == 1 {
            "index.html".to_owned()
        } else {
            segments[segments.len() - 1].clone()
        };

        if !ends_with_slash && segments.len() > 1 {
            segments.pop();
        }

        let query = url.query().filter(|q| !q.is_empty()).unwrap_or("");
        let file_name = build_file_name(&file_name_candidate, query, kind);

        let dir_relative = segments.join("/");
        let mut dir_absolute = self.root.clone();
        for seg in &segments {
            dir_absolute.push(seg);
        }
        dir_absolute = path_util::get_full_path(&dir_absolute.to_string_lossy());

        // 目录穿越兜底：映射结果必须落在根目录内
        let root_text = self.root.to_string_lossy().to_string();
        let dir_text = dir_absolute.to_string_lossy().to_string();
        let mut dir_relative = dir_relative;
        if !dir_text
            .to_lowercase()
            .starts_with(&format!("{}\\", root_text.trim_end_matches(['\\', '/'])).to_lowercase())
            && dir_text.to_lowercase() != root_text.to_lowercase()
        {
            dir_absolute = self.root.join("_unsafe");
            dir_relative = "_unsafe".to_owned();
        }

        let unique_name = self.ensure_unique(&dir_relative, &file_name);
        let absolute = dir_absolute.join(&unique_name);
        let relative = format!("{}/{}", dir_relative, unique_name);

        LocalTarget {
            absolute_path: absolute,
            relative_path: relative,
            directory: dir_absolute,
            file_name: unique_name,
        }
    }

    fn ensure_unique(&self, dir_relative: &str, file_name: &str) -> String {
        let mut claimed = self.claimed.lock().expect("文件名占用表加锁失败");

        let mut candidate = file_name.to_owned();
        let mut key = format!("{}/{}", dir_relative, candidate).to_lowercase();
        if claimed.insert(key) {
            return candidate;
        }

        let ext = Path::new(file_name)
            .extension()
            .map(|e| format!(".{}", e.to_string_lossy()))
            .unwrap_or_default();
        let stem = &file_name[..file_name.len() - ext.len()];

        for i in 2..1000 {
            candidate = format!("{}-{}{}", stem, i, ext);
            key = format!("{}/{}", dir_relative, candidate).to_lowercase();
            if claimed.insert(key) {
                return candidate;
            }
        }

        candidate = format!("{}_{}{}", stem, short_hash(file_name), ext);
        claimed.insert(format!("{}/{}", dir_relative, candidate).to_lowercase());
        candidate
    }
}

fn build_file_name(candidate: &str, query: &str, kind: ResourceKind) -> String {
    let ext = Path::new(candidate)
        .extension()
        .map(|e| format!(".{}", e.to_string_lossy()))
        .unwrap_or_default();

    let (mut stem, ext) = if safe_name::is_plausible_extension(&ext) {
        let stem = candidate[..candidate.len() - ext.len()].to_owned();
        (stem, ext)
    } else {
        (candidate.to_owned(), default_extension(kind).to_owned())
    };

    stem = safe_name::sanitize(&stem, "index");

    // 带查询参数：把参数哈希后追加到主文件名，既保留可读性又保证唯一且合法
    if !query.is_empty() {
        stem = format!("{}_{}", stem, short_hash(query));
    }

    stem + &ext
}

pub fn default_extension(kind: ResourceKind) -> &'static str {
    match kind {
        ResourceKind::Html => ".html",
        ResourceKind::Css => ".css",
        ResourceKind::JavaScript => ".js",
        _ => ".bin",
    }
}

/// 查询参数的短哈希（8 位十六进制，取 SHA-256 前 4 字节）。
pub fn short_hash(input: &str) -> String {
    let digest = Sha256::digest(input.as_bytes());
    let mut out = String::with_capacity(8);
    for byte in &digest[..4] {
        out.push_str(&format!("{:02x}", byte));
    }
    out
}

fn canonical_url_key(url: &Url) -> String {
    let mut key = format!(
        "{}://{}",
        url.scheme().to_ascii_lowercase(),
        url.host_str().unwrap_or("").to_ascii_lowercase()
    );
    if let Some(port) = url.port() {
        key.push_str(&format!(":{}", port));
    }
    key.push_str(url.path());
    if let Some(q) = url.query().filter(|q| !q.is_empty()) {
        key.push('?');
        key.push_str(q);
    }
    key
}

/// 计算 from 所在目录到 to 的相对路径，用作 HTML/CSS 中的引用路径。
/// 分隔符统一为 /，URL 里必须用正斜杠。
pub fn relative_link(from_file_absolute: &Path, to_file_absolute: &Path) -> String {
    let from_dir = match from_file_absolute.parent() {
        Some(d) if !d.as_os_str().is_empty() => d,
        _ => {
            return to_file_absolute
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default()
        }
    };

    relative_from(from_dir, to_file_absolute)
}

/// 计算 from 目录到 to 路径的相对路径，分隔符统一为 /。
/// 等价于 .NET 的 `Path.GetRelativePath(from, to)`。
pub fn relative_from(from: &Path, to: &Path) -> String {
    let from_components: Vec<Component> = from.components().collect();
    let to_components: Vec<Component> = to.components().collect();

    let common = from_components
        .iter()
        .zip(to_components.iter())
        .take_while(|(a, b)| a.as_os_str().eq_ignore_ascii_case(b.as_os_str()))
        .count();

    let mut parts: Vec<String> = Vec::new();
    for _ in common..from_components.len() {
        parts.push("..".to_owned());
    }
    for c in &to_components[common..] {
        parts.push(c.as_os_str().to_string_lossy().into_owned());
    }

    if parts.is_empty() {
        return to
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
    }

    parts.join("/")
}
