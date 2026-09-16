//! 克隆引擎。
//!
//! 抓取流程：
//!   1. 起始页入队，BFS 逐层处理；
//!   2. 每下载一个页面，扫描出它引用的全部资源 URL，并发下载；
//!   3. CSS 单独分层处理——CSS 里还有 url() 和 @import，需要下载完 CSS 才知道，
//!      所以按轮次推进（最多 5 层 @import 嵌套），而不是在资源下载任务里递归，
//!      避免 @import 成环时任务互相等待导致死锁；
//!   4. 所有资源就位后，把页面里的引用统一改写成相对路径再落盘。

use std::collections::{HashMap, HashSet, VecDeque};
use std::path::Path;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Instant;

use url::Url;

use crate::cancel::CancelToken;
use crate::error::{Error, Result};
use crate::models::{
    CloneManifest, CloneMode, DownloadResult, DownloadStatus, ManifestEntry, ResourceKind,
};
use crate::net::downloader::ResourceDownloader;
use crate::options::CloneOptions;
use crate::parse::css_scanner::{CssScanner, CssUrl};
use crate::parse::html_entity;
use crate::parse::html_scanner::{HtmlDocument, HtmlScanner};
use crate::parse::html_url::{HtmlUrl, UrlAttributeKind};
use crate::parse::normalize::normalize_index_url;
use crate::parse::url_helper;
use crate::parallel::for_each_concurrent;
use crate::progress::{CloneProgress, CloneProgressKind};
use crate::report::CloneReport;
use crate::storage::path_mapper::{relative_from, relative_link, PathMapper};
use crate::storage::path_util;
use crate::text::charset as charset_rewriter;
use crate::text::text_file as text_file;

const MAX_IMPORT_DEPTH: usize = 5;
const MAX_ERRORS: usize = 500;

type ProgressFn = Arc<dyn Fn(CloneProgress) + Send + Sync>;

pub struct CloneEngine {
    options: CloneOptions,
    /// 归一化之后的起始地址（`/dir/index.html` 会归一成 `/dir/`，避免同一页面存两份）。
    start_uri: Url,
    mapper: PathMapper,
    downloader: ResourceDownloader,
    manifest: Mutex<CloneManifest>,
    /// URL 规范化键 -> 本地文件绝对路径。
    local_path_by_url: Mutex<HashMap<String, String>>,
    enqueued_pages: Mutex<HashSet<String>>,
    /// 已扫描并改写过的 CSS。CSS 落盘前会被重写，二次扫描会把哈希文件名当成服务器地址。
    processed_css: Mutex<HashSet<String>>,
    errors: Mutex<Vec<String>>,

    pages_downloaded: AtomicUsize,
    pages_skipped: AtomicUsize,
    assets_downloaded: AtomicUsize,
    assets_skipped: AtomicUsize,
    failures: AtomicUsize,
    bytes_written: AtomicU64,

    progress: Option<ProgressFn>,
}

impl CloneEngine {
    pub fn new(options: CloneOptions) -> Result<Self> {
        let start_uri = normalize_index_url(&options.start_uri);
        let mapper = PathMapper::new(&options.output_directory);
        let downloader = ResourceDownloader::new(&options)?;
        let manifest = load_manifest(&options);

        Ok(Self {
            options,
            start_uri,
            mapper,
            downloader,
            manifest: Mutex::new(manifest),
            local_path_by_url: Mutex::new(HashMap::new()),
            enqueued_pages: Mutex::new(HashSet::new()),
            processed_css: Mutex::new(HashSet::new()),
            errors: Mutex::new(Vec::new()),
            pages_downloaded: AtomicUsize::new(0),
            pages_skipped: AtomicUsize::new(0),
            assets_downloaded: AtomicUsize::new(0),
            assets_skipped: AtomicUsize::new(0),
            failures: AtomicUsize::new(0),
            bytes_written: AtomicU64::new(0),
            progress: None,
        })
    }

    /// 设置进度回调。可能来自多个下载线程，订阅方需自行调度到 UI 线程。
    pub fn set_progress<F>(&mut self, f: F)
    where
        F: Fn(CloneProgress) + Send + Sync + 'static,
    {
        self.progress = Some(Arc::new(f));
    }

    fn report(&self, kind: CloneProgressKind, message: impl Into<String>) {
        if let Some(handler) = &self.progress {
            handler(CloneProgress::new(kind, message));
        }
    }

    /// 保存根目录（绝对路径）。
    pub fn output_root(&self) -> &Path {
        self.mapper.root()
    }

    pub fn run(&self, cancel: &CancelToken) -> Result<CloneReport> {
        let stopwatch = Instant::now();
        std::fs::create_dir_all(&self.options.output_directory)?;

        self.report(
            CloneProgressKind::Started,
            format!(
                "开始克隆：{}（{}）",
                self.start_uri,
                self.options.mode.label()
            ),
        );

        let mut queue: VecDeque<(Url, i32)> = VecDeque::new();
        queue.push_back((self.start_uri.clone(), 0));
        self.enqueued_pages
            .lock()
            .expect("入队表加锁失败")
            .insert(url_key(&self.start_uri));

        let mut pages_processed = 0usize;

        while let Some((uri, depth)) = queue.pop_front() {
            if cancel.is_cancelled() {
                return Err(Error::Cancelled);
            }

            if self.options.max_pages > 0 && pages_processed >= self.options.max_pages {
                break;
            }

            pages_processed += 1;

            let links = match self.process_page(&uri, depth, cancel) {
                Ok(links) => links,
                Err(Error::Cancelled) => return Err(Error::Cancelled),
                Err(e) => {
                    self.failures.fetch_add(1, Ordering::SeqCst);
                    self.add_error(format!("处理页面 {} 失败：{}", uri, e));
                    continue;
                }
            };

            if self.options.mode != CloneMode::FullSite || depth >= self.options.max_depth {
                continue;
            }

            let mut added = 0usize;
            for link in links {
                if self.options.max_links_per_page > 0 && added >= self.options.max_links_per_page {
                    break;
                }
                if self.options.max_pages > 0
                    && pages_processed + queue.len() >= self.options.max_pages
                {
                    break;
                }

                let normalized = normalize_index_url(&link);
                let key = url_key(&normalized);
                let mut enqueued = self.enqueued_pages.lock().expect("入队表加锁失败");
                if enqueued.insert(key) {
                    drop(enqueued);
                    queue.push_back((normalized, depth + 1));
                    added += 1;
                }
            }
        }

        self.save_manifest();

        let report = CloneReport {
            output_directory: self.options.output_directory.clone(),
            mode: self.options.mode,
            pages_downloaded: self.pages_downloaded.load(Ordering::SeqCst),
            pages_skipped: self.pages_skipped.load(Ordering::SeqCst),
            assets_downloaded: self.assets_downloaded.load(Ordering::SeqCst),
            assets_skipped: self.assets_skipped.load(Ordering::SeqCst),
            failures: self.failures.load(Ordering::SeqCst),
            bytes_written: self.bytes_written.load(Ordering::SeqCst),
            elapsed: stopwatch.elapsed(),
            errors: self.errors.lock().expect("错误表加锁失败").clone(),
        };

        self.report(CloneProgressKind::Completed, report.summary());
        Ok(report)
    }

    /// 处理单个页面：下载、扫描、抓资源、改写、落盘。返回需要继续跟进的页面链接。
    fn process_page(&self, page_uri: &Url, depth: i32, cancel: &CancelToken) -> Result<Vec<Url>> {
        let mut follow_links: Vec<Url> = Vec::new();
        let target = self.mapper.map(page_uri, ResourceKind::Html);

        let result = self.downloader.download(
            page_uri,
            &target,
            self.manifest_entry(page_uri).as_ref(),
            None,
            false,
            cancel,
        )?;

        if !result.success() {
            self.failures.fetch_add(1, Ordering::SeqCst);
            let message = format!(
                "页面下载失败：{} — {}",
                page_uri,
                result.error_message.clone().unwrap_or_default()
            );
            self.add_error(message.clone());
            self.report(CloneProgressKind::AssetFailed, format!("下载失败 {}：{}", page_uri, result.error_message.clone().unwrap_or_default()));
            return Ok(follow_links);
        }

        if result.status == DownloadStatus::Skipped {
            self.pages_skipped.fetch_add(1, Ordering::SeqCst);
            self.report(CloneProgressKind::PageSkipped, format!("未变更，跳过 {}", page_uri));
        } else {
            self.pages_downloaded.fetch_add(1, Ordering::SeqCst);
            self.report(
                CloneProgressKind::PageDownloaded,
                format!(
                    "已下载页面 {}（{:.1} KB）",
                    page_uri,
                    result.bytes_written as f64 / 1024.0
                ),
            );
        }

        let actual_path =
            self.normalize_extension(
            page_uri,
            &result,
            ResourceKind::Html,
            &target.absolute_path.to_string_lossy(),
        );

        self.record_local_mapping(page_uri, &result, &actual_path);
        self.update_manifest(page_uri, &result, &actual_path);

        // 不是 HTML 的（图片、样式表、脚本等）无需扫描
        let content_kind = url_helper::guess_from_content_type(result.content_type.as_deref());
        if matches!(
            content_kind,
            ResourceKind::Image
                | ResourceKind::Font
                | ResourceKind::Media
                | ResourceKind::Css
                | ResourceKind::JavaScript
        ) {
            return Ok(follow_links);
        }

        let decoded = text_file::read(
            Path::new(&actual_path),
            extract_charset(result.content_type_header.as_deref()).as_deref(),
            true,
        )?;

        let document = HtmlScanner::scan(&decoded.text);
        let base_uri = self.resolve_base_uri(&document, page_uri);

        // 1) 收集页面直接引用的资源
        let mut asset_urls: Vec<Url> = Vec::new();
        let mut css_urls: Vec<Url> = Vec::new();
        self.collect_assets(&decoded.text, &document, &base_uri, &mut asset_urls, &mut css_urls);

        self.download_batch(&asset_urls, Some(page_uri.as_str()), cancel);

        // 2) 处理 CSS 里的 url() 与 @import，按轮次推进
        self.process_css_layers(css_urls, cancel)?;

        // 3) 改写页面
        let mut rewritten = self.rewrite_html(
            &decoded.text,
            &document,
            &base_uri,
            &actual_path,
            depth,
            &mut follow_links,
        );

        if self.options.normalize_to_utf8 {
            rewritten = charset_rewriter::rewrite_html(&rewritten);
            text_file::write_utf8(Path::new(&actual_path), &rewritten)?;
        } else {
            std::fs::write(Path::new(&actual_path), decoded.encoding.encode(&rewritten))?;
        }

        Ok(follow_links)
    }

    /// 按响应的真实 Content-Type 修正落盘扩展名。
    ///
    /// 典型场景：Google Fonts 的动态样式表（`/css?family=...`）URL 没有扩展名，
    /// 初次映射按 HTML 存成 `.html`，而浏览器会因 MIME 类型不是 text/css
    /// 拒绝应用整个样式表（Material Icons 不渲染就是这么来的）。
    fn normalize_extension(
        &self,
        url: &Url,
        result: &DownloadResult,
        url_kind: ResourceKind,
        current_path: &str,
    ) -> String {
        let response_kind = url_helper::guess_from_content_type(result.content_type.as_deref());
        if response_kind == ResourceKind::Other || response_kind == url_kind {
            return current_path.to_owned();
        }

        let retargeted = self.mapper.retarget(url, response_kind);
        let retargeted_path = retargeted.absolute_path.to_string_lossy().into_owned();

        if retargeted_path.eq_ignore_ascii_case(current_path) {
            return current_path.to_owned();
        }

        if !Path::new(current_path).exists() {
            return retargeted_path;
        }

        match std::fs::rename(current_path, &retargeted_path) {
            Ok(()) => retargeted_path,
            Err(e) => {
                self.add_error(format!("修正扩展名失败：{} — {}", url, e));
                current_path.to_owned()
            }
        }
    }

    /// 收集页面引用的资源，CSS 单独归一类以便后续解析。
    fn collect_assets(
        &self,
        html: &str,
        document: &HtmlDocument,
        base_uri: &Url,
        assets: &mut Vec<Url>,
        css_files: &mut Vec<Url>,
    ) {
        let mut seen: HashSet<String> = HashSet::new();

        let mut add = |raw_value: &str, as_css: bool| {
            let absolute = match url_helper::try_make_absolute(raw_value, base_uri) {
                Some(u) => u,
                None => return,
            };

            if !self.is_allowed(&absolute) {
                return;
            }

            let key = url_key(&absolute);
            if !seen.insert(key) {
                return;
            }

            if as_css {
                css_files.push(absolute);
            } else {
                assets.push(absolute);
            }
        };

        for html_url in &document.urls {
            if html_url.tag_name == "base" || html_url.is_style_text() || html_url.is_navigation_link()
            {
                continue;
            }

            if html_url.tag_name == "link"
                && !should_download_rel(html_url.rel.as_deref())
            {
                continue;
            }

            // 跨域 iframe 基本都是统计/广告嵌入（noscript 里的 GTM 之类），
            // 下载下来毫无意义还污染输出目录；同域 iframe 照常本地化。
            if html_url.tag_name == "iframe" {
                if let Some(iframe_target) =
                    url_helper::try_make_absolute(&html_url.decoded_value, base_uri)
                {
                    if !is_same_host(&iframe_target, base_uri) {
                        continue;
                    }
                }
            }

            if html_url.kind == UrlAttributeKind::SrcSet {
                for entry in url_helper::parse_srcset(&html_url.decoded_value) {
                    add(&entry.url, false);
                }
                continue;
            }

            let is_css = html_url.tag_name == "link"
                && html_url
                    .rel
                    .as_deref()
                    .is_some_and(|rel| rel.contains("stylesheet"));

            add(&html_url.decoded_value, is_css);
        }

        // 内联 style 属性与 <style> 标签里的 url() 也要抓
        for region in &document.style_regions {
            let slice = &html[region.start..region.start + region.length];
            let css_text = if region.is_attribute {
                html_entity::decode(slice)
            } else {
                slice.to_owned()
            };

            for css_url in CssScanner::scan(&css_text) {
                // @import 只出现在内联样式里很少见，一并处理
                add(&css_url.value, css_url.is_import);
            }
        }
    }

    fn download_batch(&self, urls: &[Url], referer: Option<&str>, cancel: &CancelToken) {
        if urls.is_empty() {
            return;
        }

        for_each_concurrent(urls, self.options.concurrency, |url| {
            self.fetch_asset(url, referer, cancel);
        });
    }

    /// 下载单个资源并记录到本地路径表。同一 URL 只下载一次。
    fn fetch_asset(&self, url: &Url, referer: Option<&str>, cancel: &CancelToken) -> Option<String> {
        {
            let cache = self.local_path_by_url.lock().expect("路径表加锁失败");
            if let Some(cached) = cache.get(&url_key(url)) {
                return Some(cached.clone());
            }
        }

        let kind = url_helper::guess_from_path(url);
        let target = self.mapper.map(url, kind);

        // HTML / CSS 落盘前会被改写，本地副本不再是服务器原始内容，
        // 增量条件请求只对不会被改写的二进制资源启用。
        let allow_conditional = !matches!(kind, ResourceKind::Css | ResourceKind::Html);

        let result = match self.downloader.download(
            url,
            &target,
            self.manifest_entry(url).as_ref(),
            referer,
            allow_conditional,
            cancel,
        ) {
            Ok(r) => r,
            Err(Error::Cancelled) => return None,
            Err(e) => {
                self.failures.fetch_add(1, Ordering::SeqCst);
                let message = format!("资源下载失败：{} — {}", url, e);
                self.add_error(message.clone());
                self.report(CloneProgressKind::AssetFailed, message);
                return None;
            }
        };

        if !result.success() {
            self.failures.fetch_add(1, Ordering::SeqCst);
            let message = format!(
                "资源下载失败：{} — {}",
                url,
                result.error_message.clone().unwrap_or_default()
            );
            self.add_error(message.clone());
            self.report(CloneProgressKind::AssetFailed, message);
            return None;
        }

        if result.status == DownloadStatus::Skipped {
            self.assets_skipped.fetch_add(1, Ordering::SeqCst);
            self.report(CloneProgressKind::AssetSkipped, format!("未变更，跳过 {}", url));
        } else {
            self.assets_downloaded.fetch_add(1, Ordering::SeqCst);
            self.bytes_written
                .fetch_add(result.bytes_written, Ordering::SeqCst);
            self.report(
                CloneProgressKind::AssetDownloaded,
                format!(
                    "已下载 {}（{:.1} KB{}）",
                    url,
                    result.bytes_written as f64 / 1024.0,
                    if result.was_resumed { "，断点续传" } else { "" }
                ),
            );
        }

        let actual_path = self.normalize_extension(url, &result, kind, &target.absolute_path.to_string_lossy());

        self.record_local_mapping(url, &result, &actual_path);
        self.update_manifest(url, &result, &actual_path);
        Some(actual_path)
    }

    /// 记录 URL 到本地文件的映射。
    /// 服务端重定向时，最终地址（如 `/doc/` 重定向自 `/doc`）也指向同一本地文件，
    /// 否则同一资源会被当成两个页面各存一份，链接之间还会互相指向不同的副本。
    fn record_local_mapping(&self, url: &Url, result: &DownloadResult, local_path: &str) {
        if !result.success() {
            return;
        }

        let mut cache = self.local_path_by_url.lock().expect("路径表加锁失败");
        let key = url_key(url);
        cache.insert(key.clone(), local_path.to_owned());

        if let Some(final_uri) = &result.final_uri {
            let final_key = url_key(final_uri);
            if final_key != key {
                cache.insert(final_key, local_path.to_owned());
            }
        }
    }

    /// 分层处理 CSS：下载完一层 CSS 后扫描其中的 url() 与 @import，
    /// 下载新发现的资源，改写这一层 CSS，再把 @import 进来的新样式表推进到下一轮。
    fn process_css_layers(&self, seed_css: Vec<Url>, cancel: &CancelToken) -> Result<()> {
        let mut current = seed_css;

        for _ in 0..MAX_IMPORT_DEPTH {
            if current.is_empty() {
                break;
            }
            if cancel.is_cancelled() {
                return Err(Error::Cancelled);
            }

            let mut next: Vec<Url> = Vec::new();
            let mut pending: Vec<Url> = Vec::new();

            {
                let mut processed = self.processed_css.lock().expect("CSS 已处理表加锁失败");
                for css_uri in &current {
                    // CSS 落盘前会被重写，每个样式表只扫描一次原始内容，
                    // 后续页面再引用到它时直接跳过，否则会把哈希文件名当成服务器地址去请求。
                    if processed.insert(url_key(css_uri)) {
                        pending.push(css_uri.clone());
                    }
                }
            }

            self.download_batch(&pending, None, cancel);

            for css_uri in &pending {
                let local_path = {
                    let cache = self.local_path_by_url.lock().expect("路径表加锁失败");
                    cache.get(&url_key(css_uri)).cloned()
                };

                let local_path = match local_path {
                    Some(p) if Path::new(&p).exists() => p,
                    _ => continue,
                };

                let decoded = text_file::read(Path::new(&local_path), None, false)?;
                let css_references = CssScanner::scan(&decoded.text);

                let mut to_download: Vec<Url> = Vec::new();
                for reference in &css_references {
                    let absolute = match url_helper::try_make_absolute(&reference.value, css_uri) {
                        Some(u) => u,
                        None => continue,
                    };
                    if !self.is_allowed(&absolute) {
                        continue;
                    }

                    to_download.push(absolute.clone());

                    if reference.is_import
                        && url_helper::guess_from_path(&absolute) == ResourceKind::Css
                    {
                        next.push(absolute);
                    }
                }

                self.download_batch(&to_download, Some(css_uri.as_str()), cancel);

                let rewritten =
                    self.rewrite_css_text(&decoded.text, &css_references, css_uri, &local_path);
                if rewritten != decoded.text {
                    if self.options.normalize_to_utf8 {
                        let rewritten = charset_rewriter::rewrite_css(&rewritten);
                        text_file::write_utf8(Path::new(&local_path), &rewritten)?;
                    } else {
                        std::fs::write(
                            Path::new(&local_path),
                            decoded.encoding.encode(&rewritten),
                        )?;
                    }
                }
            }

            current = next;
        }

        Ok(())
    }

    /// 改写 HTML：把资源引用换成相对路径，把外站链接按模式处理。
    fn rewrite_html(
        &self,
        html: &str,
        document: &HtmlDocument,
        base_uri: &Url,
        local_html_path: &str,
        depth: i32,
        follow_links: &mut Vec<Url>,
    ) -> String {
        let mut replacements: Vec<Replacement> = Vec::new();
        let mut follow_set: HashSet<String> = HashSet::new();

        for html_url in &document.urls {
            // <base href> 统一指向当前目录，否则重写后的相对路径会被带偏
            if html_url.tag_name == "base" && html_url.attribute_name == "href" {
                replacements.push(Replacement::new(html_url.value_start, html_url.value_length, "."));
                continue;
            }

            if html_url.is_style_text() {
                let css_text = &html_url.decoded_value;
                let references = CssScanner::scan(css_text);
                if references.is_empty() {
                    continue;
                }

                let new_css = self.rewrite_css_text(css_text, &references, base_uri, local_html_path);
                if new_css != *css_text {
                    // style="..." 是属性值，引号必须转成实体；
                    // <style> 标签正文是 RAWTEXT，浏览器不解析实体，写 &quot; 反而会把 CSS 弄坏。
                    let value = if html_url.kind == UrlAttributeKind::InlineStyle {
                        html_entity::encode_for_attribute(&new_css)
                    } else {
                        new_css
                    };

                    replacements.push(Replacement::new(
                        html_url.value_start,
                        html_url.value_length,
                        value,
                    ));
                }

                continue;
            }

            if html_url.kind == UrlAttributeKind::SrcSet {
                let entries = url_helper::parse_srcset(&html_url.decoded_value);
                if entries.is_empty() {
                    continue;
                }

                let mut builder = html_url.decoded_value.clone();
                for entry in entries.iter().rev() {
                    let absolute = match url_helper::try_make_absolute(&entry.url, base_uri) {
                        Some(u) => u,
                        None => continue,
                    };

                    if let Some(local) = self.try_get_local_path(&absolute) {
                        let relative = relative_link(Path::new(local_html_path), Path::new(&local));
                        builder.replace_range(entry.start..entry.start + entry.length, &relative);
                    }
                }

                replacements.push(Replacement::new(
                    html_url.value_start,
                    html_url.value_length,
                    html_entity::encode_for_attribute(&builder),
                ));
                continue;
            }

            let target = match url_helper::try_make_absolute(&html_url.decoded_value, base_uri) {
                Some(t) => t,
                None => continue,
            };

            if html_url.is_navigation_link() {
                self.handle_navigation_link(
                    html_url,
                    &target,
                    depth,
                    local_html_path,
                    &mut replacements,
                    follow_links,
                    &mut follow_set,
                );
                continue;
            }

            if let Some(local_asset) = self.try_get_local_path(&target) {
                let relative = relative_link(Path::new(local_html_path), Path::new(&local_asset));
                replacements.push(Replacement::new(
                    html_url.value_start,
                    html_url.value_length,
                    html_entity::encode_for_attribute(&relative),
                ));
            }
        }

        apply_replacements(html, &mut replacements)
    }

    #[allow(clippy::too_many_arguments)]
    fn handle_navigation_link(
        &self,
        html_url: &HtmlUrl,
        target: &Url,
        depth: i32,
        local_html_path: &str,
        replacements: &mut Vec<Replacement>,
        follow_links: &mut Vec<Url>,
        follow_set: &mut HashSet<String>,
    ) {
        let key = url_key(target);

        if self.options.mode == CloneMode::FullSite {
            if depth + 1 > self.options.max_depth {
                return;
            }

            if !self.is_followable(target) {
                // 外站或上级目录：保留原样，不改写
                return;
            }

            // 目标页面已经下载过（含经重定向归一的地址），直接引用本地文件，不再入队
            if let Some(downloaded) = self.try_get_local_path(target) {
                let relative = relative_link(Path::new(local_html_path), Path::new(&downloaded));
                replacements.push(Replacement::new(
                    html_url.value_start,
                    html_url.value_length,
                    html_entity::encode_for_attribute(&relative),
                ));
                return;
            }

            if follow_set.insert(key) {
                follow_links.push(target.clone());
            }

            // 目标页面的本地路径由映射规则唯一确定，即使还没下载也能提前算出相对路径
            let expected = self.mapper.map(target, ResourceKind::Html).absolute_path;
            let relative = relative_link(Path::new(local_html_path), &expected);
            replacements.push(Replacement::new(
                html_url.value_start,
                html_url.value_length,
                html_entity::encode_for_attribute(&relative),
            ));
            return;
        }

        // 单页模式：目标页面不会下载，指向它的链接一律失效处理，避免点了跳外站或 404
        if let Some(local) = self.try_get_local_path(target) {
            let relative = relative_link(Path::new(local_html_path), Path::new(&local));
            replacements.push(Replacement::new(
                html_url.value_start,
                html_url.value_length,
                html_entity::encode_for_attribute(&relative),
            ));
            return;
        }

        if self.options.neutralize_external_links {
            replacements.push(Replacement::new(html_url.value_start, html_url.value_length, "#"));
        }
    }

    /// 改写 CSS 文本中的 url() 与 @import 为相对路径。
    fn rewrite_css_text(
        &self,
        css: &str,
        references: &[CssUrl],
        css_uri: &Url,
        local_css_path: &str,
    ) -> String {
        if references.is_empty() {
            return css.to_owned();
        }

        let mut replacements: Vec<Replacement> = Vec::new();

        for reference in references {
            let absolute = match url_helper::try_make_absolute(&reference.value, css_uri) {
                Some(u) => u,
                None => continue,
            };

            let local = match self.try_get_local_path(&absolute) {
                Some(l) => l,
                None => continue,
            };

            let relative = relative_link(Path::new(local_css_path), Path::new(&local));
            replacements.push(Replacement::new(
                reference.start,
                reference.length,
                quote_for_css(&relative, reference.was_quoted),
            ));
        }

        apply_replacements(css, &mut replacements)
    }

    fn resolve_base_uri(&self, document: &HtmlDocument, page_uri: &Url) -> Url {
        if let Some(base_href) = document.base_href.as_deref().map(str::trim) {
            if !base_href.is_empty() {
                if let Ok(absolute) = Url::parse(base_href) {
                    if url_helper::is_http_scheme(&absolute) {
                        return absolute;
                    }
                }
                if let Ok(resolved) = page_uri.join(base_href) {
                    if url_helper::is_http_scheme(&resolved) {
                        return resolved;
                    }
                }
            }
        }

        page_uri.clone()
    }

    /// 是否允许下载这个资源（跨域策略）。
    fn is_allowed(&self, candidate: &Url) -> bool {
        if self.options.include_cross_origin_assets {
            return true;
        }
        is_same_host(candidate, &self.start_uri)
    }

    /// 整站模式下的跟随判断：同域 + 不向上级目录（wget 的 -np）。
    fn is_followable(&self, candidate: &Url) -> bool {
        is_same_host(candidate, &self.start_uri)
            && is_under_start_directory(candidate, &self.start_uri)
    }

    fn try_get_local_path(&self, url: &Url) -> Option<String> {
        let cache = self.local_path_by_url.lock().expect("路径表加锁失败");
        cache.get(&url_key(url)).cloned()
    }

    fn manifest_entry(&self, url: &Url) -> Option<ManifestEntry> {
        let manifest = self.manifest.lock().expect("清单加锁失败");
        manifest.get(&url_key(url)).cloned()
    }

    fn update_manifest(&self, url: &Url, result: &DownloadResult, local_path: &str) {
        let mut manifest = self.manifest.lock().expect("清单加锁失败");
        manifest.set(ManifestEntry {
            url: url_key(url),
            local_path: relative_from(self.mapper.root(), Path::new(local_path)),
            etag: result.etag.clone(),
            last_modified: result.last_modified.as_ref().map(crate::net::format_http_date),
            length: result.total_bytes,
            content_type: result.content_type.clone(),
            downloaded_at: chrono::Local::now(),
        });
    }

    fn add_error(&self, message: String) {
        let mut errors = self.errors.lock().expect("错误表加锁失败");
        if errors.len() < MAX_ERRORS {
            errors.push(message);
        }
    }

    fn save_manifest(&self) {
        let result = (|| -> std::io::Result<()> {
            std::fs::create_dir_all(&self.options.output_directory)?;
            let mut manifest = self.manifest.lock().expect("清单加锁失败");
            manifest.updated_at = chrono::Local::now();
            let json = serde_json::to_string_pretty(&*manifest)
                .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
            std::fs::write(
                Path::new(&self.options.output_directory).join(CloneManifest::FILE_NAME),
                json,
            )
        })();

        if let Err(e) = result {
            self.add_error(format!("保存清单失败：{}", e));
        }
    }
}

// ---------------------------------------------------------------- 辅助

#[derive(Debug, Clone)]
struct Replacement {
    start: usize,
    length: usize,
    new_text: String,
}

impl Replacement {
    fn new(start: usize, length: usize, new_text: impl Into<String>) -> Self {
        Self {
            start,
            length,
            new_text: new_text.into(),
        }
    }
}

/// 从后往前做替换，避免前面的改动挪动后面下标。
fn apply_replacements(text: &str, replacements: &mut [Replacement]) -> String {
    if replacements.is_empty() {
        return text.to_owned();
    }

    replacements.sort_by(|a, b| b.start.cmp(&a.start));

    let original_len = text.len();
    let mut out = text.to_owned();
    for replacement in replacements.iter() {
        // 越界判定按"原文长度"来，而不是改过之后的长度——否则每替换一次基准就变了
        if replacement.start + replacement.length > original_len {
            continue;
        }
        out.replace_range(
            replacement.start..replacement.start + replacement.length,
            &replacement.new_text,
        );
    }

    out
}

fn quote_for_css(value: &str, was_quoted: bool) -> String {
    if was_quoted {
        return format!("\"{}\"", value.replace('"', "\\\""));
    }

    let needs_quote = value
        .chars()
        .any(|c| matches!(c, ' ' | '(' | ')' | '\'' | '"' | ',' | '\t'));
    if needs_quote {
        format!("\"{}\"", value.replace('"', "\\\""))
    } else {
        value.to_owned()
    }
}

fn is_same_host(a: &Url, b: &Url) -> bool {
    a.host_str().unwrap_or("").eq_ignore_ascii_case(b.host_str().unwrap_or(""))
        && a.port_or_known_default() == b.port_or_known_default()
}

fn is_under_start_directory(candidate: &Url, start: &Url) -> bool {
    let start_path = start.path();
    let start_directory = match start_path.rfind('/') {
        Some(last_slash) => &start_path[..last_slash + 1],
        None => "/",
    };

    let candidate_path = candidate.path();
    candidate_path.len() >= start_directory.len()
        && candidate_path[..start_directory.len()].eq_ignore_ascii_case(start_directory)
}

fn should_download_rel(rel: Option<&str>) -> bool {
    let rel = match rel {
        Some(r) if !r.trim().is_empty() => r,
        _ => return true,
    };

    // rel 可能是多值（如 "preload as=style"），按 token 精确匹配。
    // canonical / alternate / dns-prefetch / preconnect / me 都不是页面资源，
    // 尤其 me 必须整词匹配——用包含判断会误伤 media。
    !rel
        .split(' ')
        .filter(|t| !t.is_empty())
        .any(|token| {
            matches!(
                token.to_ascii_lowercase().as_str(),
                "canonical" | "alternate" | "dns-prefetch" | "preconnect" | "me"
            )
        })
}

/// URL 规范化键：统一小写 scheme/host，去掉 fragment。
/// query 带前导 `?`，与 C# 版 `Uri.Query`（含问号）拼出来的键逐字节一致。
fn url_key(uri: &Url) -> String {
    let mut key = format!(
        "{}://{}",
        uri.scheme().to_ascii_lowercase(),
        uri.host_str().unwrap_or("").to_ascii_lowercase()
    );
    if let Some(port) = uri.port() {
        key.push_str(&format!(":{}", port));
    }
    key.push_str(uri.path());
    if let Some(query) = uri.query().filter(|q| !q.is_empty()) {
        key.push('?');
        key.push_str(query);
    }
    key
}

fn extract_charset(content_type_header: Option<&str>) -> Option<String> {
    let header = content_type_header?;
    let lower = header.to_ascii_lowercase();
    let index = lower.find("charset=")?;
    let value = &header[index + "charset=".len()..];
    let value = match value.find(';') {
        Some(semi) => &value[..semi],
        None => value,
    };
    let trimmed = value.trim().trim_matches(['"', '\'']).trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_owned())
    }
}

fn load_manifest(options: &CloneOptions) -> CloneManifest {
    let path = Path::new(&options.output_directory).join(CloneManifest::FILE_NAME);
    let fresh = || CloneManifest::new(options.start_uri.to_string(), options.mode);

    let text = match std::fs::read_to_string(&path) {
        Ok(t) => t,
        Err(_) => return fresh(),
    };

    let manifest: CloneManifest = match serde_json::from_str(&text) {
        Ok(m) => m,
        Err(_) => return fresh(),
    };

    // 换网址或换模式后，旧清单不再适用
    if manifest.start_url != options.start_uri.to_string()
        || manifest.mode != options.mode.as_str()
    {
        return fresh();
    }

    manifest
}

/// 供 CLI/GUI 复用的"把相对路径转绝对"的入口。
pub fn resolve_output_directory(raw: &str) -> String {
    path_util::get_full_path(raw).to_string_lossy().into_owned()
}
