//! 按配置构造 HTTP 客户端。
//!
//! 两套客户端是有意设计的：
//! - 普通客户端开启自动解压，节省带宽；
//! - 续传客户端关闭自动解压。因为 Range 请求针对的是压缩后的字节偏移，
//!   一旦中间发生解压，追加写入的文件就会损坏。续传时显式要求 identity 编码。

use std::time::Duration;

use reqwest::blocking::Client;
use reqwest::header::{
    HeaderMap, HeaderValue, ACCEPT, ACCEPT_ENCODING, ACCEPT_LANGUAGE, USER_AGENT,
};
use reqwest::redirect::Policy;
use reqwest::Proxy;

use crate::error::{Error, Result};
use crate::options::CloneOptions;

pub fn build_client(options: &CloneOptions, disable_decompression: bool) -> Result<Client> {
    let timeout = Duration::from_secs(options.timeout_seconds.max(5));

    let mut headers = HeaderMap::new();
    headers.insert(ACCEPT, HeaderValue::from_static("*/*"));
    headers.insert(
        ACCEPT_LANGUAGE,
        HeaderValue::from_static("zh-CN,zh;q=0.9,en;q=0.8"),
    );
    if disable_decompression {
        headers.insert(ACCEPT_ENCODING, HeaderValue::from_static("identity"));
    }

    // UA 走默认请求头而不是 ClientBuilder::user_agent：
    // 用户自定义的 UA 若含非法字符，只丢这一条头即可，不该让整个客户端构建失败。
    if let Some(value) = header_value_of(&options.user_agent) {
        headers.insert(USER_AGENT, value);
    }

    let mut builder = Client::builder()
        .timeout(timeout)
        .connect_timeout(timeout)
        .redirect(Policy::limited(8))
        .referer(false)
        .pool_max_idle_per_host(options.concurrency.max(2))
        .pool_idle_timeout(Duration::from_secs(60))
        .tcp_nodelay(true)
        .default_headers(headers);

    if disable_decompression {
        builder = builder.gzip(false).brotli(false).deflate(false);
    }

    // 代理：未配置时显式走直连，不读环境变量。
    // （reqwest 默认特性里的 system-proxy 会在这一步引入环境代理，已在 Cargo.toml 关掉，
    //   这里再显式 no_proxy 一次，确保"未填代理 = 直连"的语义稳定。）
    builder = match options.proxy.to_proxy_url() {
        Some(url) => {
            let proxy = Proxy::all(&url).map_err(|e| Error::Other(format!("代理地址无效：{}", e)))?;
            builder.proxy(proxy)
        }
        None => builder.no_proxy(),
    };

    if options.ignore_certificate_errors {
        builder = builder.tls_danger_accept_invalid_certs(true);
    }

    builder
        .build()
        .map_err(|e| Error::Other(format!("创建 HTTP 客户端失败：{}", e)))
}

/// 把字符串转成请求头值；含非法字符时返回 None 而不是报错。
fn header_value_of(value: &str) -> Option<HeaderValue> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return None;
    }
    HeaderValue::from_str(trimmed).ok()
}
