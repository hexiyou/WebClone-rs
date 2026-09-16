//! 资源下载器。
//!
//! 增量与续传的三条路径：
//! 1. 本地已有完整文件且记录了 ETag/Last-Modified → 发条件请求，服务端回 304 就直接跳过；
//! 2. 本地只有 .part 半成品 → 用 Range + If-Range 续传。If-Range 带上旧 ETag，
//!    服务端一旦发现资源变了会自动降级成 200 全量返回，不需要客户端自己校验一致性；
//! 3. 都没有 → 全量下载。
//!
//! 无论走哪条路径，都先写 .part 临时文件，写完之后才原子地 rename 成正式文件，
//! 避免中途失败在目录里留下一个残缺的 style.css 之类的假成品。

use std::io::{Read, Write};
use std::path::Path;
use std::time::Duration;

use reqwest::blocking::{Client, Response};
use reqwest::header::{
    HeaderValue, CONTENT_TYPE, ETAG, IF_MODIFIED_SINCE, IF_NONE_MATCH, IF_RANGE, LAST_MODIFIED,
    RANGE, REFERER,
};
use reqwest::StatusCode;

use super::{format_http_date, parse_http_date};
use crate::cancel::CancelToken;
use crate::error::{describe_reqwest_error, Error, Result};
use crate::models::{DownloadResult, DownloadStatus, ManifestEntry};
use crate::net::http::build_client;
use crate::options::CloneOptions;
use crate::storage::path_mapper::LocalTarget;

const PART_SUFFIX: &str = ".part";
const BUFFER_SIZE: usize = 81920;

pub struct ResourceDownloader {
    options: CloneOptions,
    /// 开启自动解压，日常下载用。
    client: Client,
    /// 关闭自动解压，续传用。
    raw_client: Client,
}

impl ResourceDownloader {
    pub fn new(options: &CloneOptions) -> Result<Self> {
        Ok(Self {
            options: options.clone(),
            client: build_client(options, false)?,
            raw_client: build_client(options, true)?,
        })
    }

    pub fn download(
        &self,
        url: &url::Url,
        target: &LocalTarget,
        existing: Option<&ManifestEntry>,
        referer: Option<&str>,
        allow_conditional: bool,
        cancel: &CancelToken,
    ) -> Result<DownloadResult> {
        std::fs::create_dir_all(&target.directory)?;

        let part_path = Path::new(&format!("{}{}", target.absolute_path.to_string_lossy(), PART_SUFFIX))
            .to_path_buf();
        let part_length = std::fs::metadata(&part_path).map(|m| m.len()).unwrap_or(0);
        let has_complete_file = target.absolute_path.exists();

        // HTML / CSS 会被改写后落盘，本地文件已经不是服务器的原始内容，
        // 拿它做条件请求会污染下一次的扫描结果，因此这类文件必须每次取原始内容。
        let use_conditional = allow_conditional
            && self.options.enable_incremental
            && has_complete_file
            && existing.is_some_and(|e| e.etag.is_some() || e.last_modified.is_some());

        let can_resume = part_length > 0 && existing.and_then(|e| e.etag.as_deref()).is_some();

        let mut last_error = String::new();
        let mut last_status = 0u16;

        for attempt in 0..=self.options.retry_count {
            if cancel.is_cancelled() {
                return Err(Error::Cancelled);
            }

            if attempt > 0 {
                sleep_backoff(attempt, cancel)?;
            }

            match self.attempt(
                url,
                target,
                &part_path,
                existing,
                referer,
                use_conditional,
                can_resume,
                part_length,
                has_complete_file,
                cancel,
            ) {
                Ok(result) => return Ok(result),
                Err(AttemptError::Cancelled) => return Err(Error::Cancelled),
                Err(AttemptError::Status { code, message }) => {
                    last_status = code;
                    last_error = message;
                    if should_retry(code) && attempt < self.options.retry_count {
                        continue;
                    }
                    return Ok(DownloadResult::failed(
                        url.clone(),
                        target.absolute_path.to_string_lossy().into_owned(),
                        last_error,
                        last_status,
                    ));
                }
                Err(AttemptError::Transport(message)) => {
                    last_error = message;
                    if attempt < self.options.retry_count {
                        continue;
                    }
                    return Ok(DownloadResult::failed(
                        url.clone(),
                        target.absolute_path.to_string_lossy().into_owned(),
                        last_error,
                        last_status,
                    ));
                }
            }
        }

        Ok(DownloadResult::failed(
            url.clone(),
            target.absolute_path.to_string_lossy().into_owned(),
            last_error,
            last_status,
        ))
    }

    #[allow(clippy::too_many_arguments)]
    fn attempt(
        &self,
        url: &url::Url,
        target: &LocalTarget,
        part_path: &Path,
        existing: Option<&ManifestEntry>,
        referer: Option<&str>,
        use_conditional: bool,
        can_resume: bool,
        part_length: u64,
        has_complete_file: bool,
        cancel: &CancelToken,
    ) -> std::result::Result<DownloadResult, AttemptError> {
        if cancel.is_cancelled() {
            return Err(AttemptError::Cancelled);
        }

        // 续传必须走关闭自动解压的客户端（Range 的偏移量是按压缩后字节算的）
        let resume = can_resume && !use_conditional;
        let client = if resume { &self.raw_client } else { &self.client };

        let mut request = client.get(url.clone());
        if let Some(referer) = referer.filter(|r| !r.is_empty()) {
            if let Ok(value) = HeaderValue::from_str(referer) {
                request = request.header(REFERER, value);
            }
        }

        let mut resume_offset = 0u64;

        if use_conditional {
            let existing = existing.expect("条件请求必然带旧记录");
            if let Some(etag) = existing.etag.as_deref() {
                if let Ok(value) = HeaderValue::from_str(etag) {
                    request = request.header(IF_NONE_MATCH, value);
                }
            }
            if let Some(modified) = existing.last_modified.as_deref().and_then(parse_http_date) {
                if let Ok(value) = HeaderValue::from_str(&format_http_date(&modified)) {
                    request = request.header(IF_MODIFIED_SINCE, value);
                }
            }
        } else if resume {
            let etag = existing.and_then(|e| e.etag.clone()).unwrap_or_default();
            request = request.header(RANGE, format!("bytes={}-", part_length));
            // 资源若已变化，服务端会忽略 Range 直接返回 200 全量
            if let Ok(value) = HeaderValue::from_str(&etag) {
                request = request.header(IF_RANGE, value);
            }
            resume_offset = part_length;
        }

        let response = match request.send() {
            Ok(r) => r,
            Err(e) => return Err(AttemptError::Transport(describe_reqwest_error(&e))),
        };

        Self::handle_response(
            url,
            target,
            part_path,
            response,
            existing,
            has_complete_file,
            resume_offset,
            cancel,
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn handle_response(
        url: &url::Url,
        target: &LocalTarget,
        part_path: &Path,
        response: Response,
        existing: Option<&ManifestEntry>,
        has_complete_file: bool,
        resume_offset: u64,
        cancel: &CancelToken,
    ) -> std::result::Result<DownloadResult, AttemptError> {
        let status = response.status();
        let status_code = status.as_u16();
        let mut response = response;

        if status == StatusCode::NOT_MODIFIED {
            return Ok(DownloadResult {
                status: DownloadStatus::Skipped,
                url: url.clone(),
                local_path: target.absolute_path.to_string_lossy().into_owned(),
                status_code: 304,
                bytes_written: 0,
                total_bytes: if has_complete_file {
                    std::fs::metadata(&target.absolute_path).map(|m| m.len()).unwrap_or(0)
                } else {
                    0
                },
                content_type: None,
                content_type_header: None,
                etag: existing.and_then(|e| e.etag.clone()),
                last_modified: existing
                    .and_then(|e| e.last_modified.as_deref())
                    .and_then(parse_http_date),
                final_uri: Some(response.url().clone()),
                error_message: None,
                was_resumed: false,
            });
        }

        if !status.is_success() {
            let reason = status.canonical_reason().unwrap_or("");
            return Err(AttemptError::Status {
                code: status_code,
                message: format!("HTTP {} {}", status_code, reason).trim_end().to_owned(),
            });
        }

        let headers = response.headers().clone();
        let final_uri = response.url().clone();
        let declared_length = response.content_length();

        let append = status == StatusCode::PARTIAL_CONTENT && resume_offset > 0;

        let written = match write_body(&mut response, part_path, append, cancel) {
            Ok(n) => n,
            Err(e) => return Err(e),
        };

        std::fs::rename(part_path, &target.absolute_path)
            .map_err(|e| AttemptError::Transport(format!("落盘失败：{}", e)))?;

        let content_type_header = headers
            .get(CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            .map(|s| s.to_owned());
        let content_type = content_type_header
            .as_deref()
            .map(|s| s.split(';').next().unwrap_or("").trim().to_owned())
            .filter(|s| !s.is_empty());

        let total = match declared_length {
            Some(len) => {
                if append {
                    len + resume_offset
                } else {
                    len
                }
            }
            None => written,
        };

        Ok(DownloadResult {
            status: DownloadStatus::Downloaded,
            url: url.clone(),
            local_path: target.absolute_path.to_string_lossy().into_owned(),
            status_code,
            bytes_written: written,
            total_bytes: total,
            content_type,
            content_type_header,
            etag: headers.get(ETAG).and_then(|v| v.to_str().ok()).map(|s| s.to_owned()),
            last_modified: headers
                .get(LAST_MODIFIED)
                .and_then(|v| v.to_str().ok())
                .and_then(parse_http_date),
            final_uri: Some(final_uri),
            error_message: None,
            was_resumed: append,
        })
    }
}

/// 把响应体写进 .part 文件，返回最终写入的文件长度。
fn write_body(
    response: &mut Response,
    part_path: &Path,
    append: bool,
    cancel: &CancelToken,
) -> std::result::Result<u64, AttemptError> {
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create(true)
        .append(append)
        .truncate(!append)
        .open(part_path)
        .map_err(|e| AttemptError::Transport(format!("无法创建临时文件：{}", e)))?;

    let mut buffer = vec![0u8; BUFFER_SIZE];
    loop {
        if cancel.is_cancelled() {
            return Err(AttemptError::Cancelled);
        }

        let read = response
            .read(&mut buffer)
            .map_err(|e| AttemptError::Transport(format!("读取响应失败：{}", e)))?;
        if read == 0 {
            break;
        }

        file.write_all(&buffer[..read])
            .map_err(|e| AttemptError::Transport(format!("写入文件失败：{}", e)))?;
    }

    file.flush()
        .map_err(|e| AttemptError::Transport(format!("刷新文件失败：{}", e)))?;

    Ok(file
        .metadata()
        .map(|m| m.len())
        .map_err(|e| AttemptError::Transport(format!("读取文件长度失败：{}", e)))?)
}

enum AttemptError {
    Cancelled,
    Status { code: u16, message: String },
    Transport(String),
}

/// 与 C# 版一致的"值得重试"状态码集合。
fn should_retry(code: u16) -> bool {
    matches!(code, 408 | 429 | 500 | 502 | 503 | 504)
}

/// 指数退避：2^(attempt-1) 秒，封顶 30 秒，再加 0~500 毫秒抖动。
fn sleep_backoff(attempt: u32, cancel: &CancelToken) -> Result<()> {
    let seconds = 2u64.saturating_pow(attempt - 1).min(30);
    let jitter_ms = pseudo_random_ms();
    let total = Duration::from_secs(seconds) + Duration::from_millis(jitter_ms);

    // 分片休眠，取消能及时生效
    let step = Duration::from_millis(100);
    let deadline = std::time::Instant::now() + total;
    while std::time::Instant::now() < deadline {
        if cancel.is_cancelled() {
            return Err(Error::Cancelled);
        }
        std::thread::sleep(step.min(deadline.saturating_duration_since(std::time::Instant::now())));
    }

    Ok(())
}

/// 0~499 的伪随机毫秒数。不引 rand 依赖，用纳秒时钟低位做抖动足够。
fn pseudo_random_ms() -> u64 {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.subsec_nanos() as u64)
        .unwrap_or(0);
    nanos % 500
}
