//! WebClone 核心引擎（Rust 移植版）。
//!
//! 把网页连同其引用的样式表、脚本、图片、字体下载到本地，
//! 并把页面中的链接改写为相对路径，产出可直接离线打开的副本。
//!
//! 模块划分与 C# 版 `WebClone.Core` 一一对应，方便对照阅读：
//!
//! | C# | Rust |
//! |----|------|
//! | `CloneEngine.cs` | [`engine`] |
//! | `CloneOptions.cs` / `ProxySettings.cs` | [`options`] |
//! | `Models/*` | [`models`] |
//! | `Net/ResourceDownloader.cs` | [`net::downloader`] |
//! | `Net/HttpClientFactory.cs` | [`net::http`] |
//! | `Net/Socks5Connector.cs` | 由 reqwest 的 socks 特性承担 |
//! | `Parse/HtmlScanner.cs` / `CssScanner.cs` | [`parse::html_scanner`] / [`parse::css_scanner`] |
//! | `Storage/PathMapper.cs` / `SafeFileName.cs` | [`storage::path_mapper`] / [`storage::safe_name`] |
//! | `Text/EncodingDetector.cs` / `CharsetRewriter.cs` | [`text::encoding`] / [`text::charset`] |

pub mod cancel;
pub mod engine;
pub mod error;
pub mod models;
pub mod net;
pub mod options;
pub mod parallel;
pub mod parse;
pub mod progress;
pub mod report;
pub mod storage;
pub mod text;

pub use cancel::CancelToken;
pub use engine::CloneEngine;
pub use error::{Error, Result};
pub use models::{
    CloneManifest, CloneMode, DownloadResult, DownloadStatus, ManifestEntry, ProxyKind, ResourceKind,
};
pub use options::{CloneOptions, ProxySettings, DEFAULT_USER_AGENT, EDGE_USER_AGENT};
pub use progress::{CloneProgress, CloneProgressKind};
pub use report::CloneReport;
pub use storage::path_util;
