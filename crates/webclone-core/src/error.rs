//! 错误类型。
//!
//! 与 C# 版对齐的三类结果：取消、I/O 失败、其他可读错误。
//! CLI 用 `Error::Cancelled` 区分退出码 130。

use std::fmt;

pub type Result<T> = std::result::Result<T, Error>;

#[derive(Debug)]
pub enum Error {
    /// 用户主动取消（对应 C# 的 OperationCanceledException）。
    Cancelled,
    Io(std::io::Error),
    Url(String),
    Other(String),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Cancelled => write!(f, "任务已取消。"),
            Error::Io(e) => write!(f, "{}", e),
            Error::Url(m) => write!(f, "{}", m),
            Error::Other(m) => write!(f, "{}", m),
        }
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Error::Io(e) => Some(e),
            _ => None,
        }
    }
}

impl From<std::io::Error> for Error {
    fn from(e: std::io::Error) -> Self {
        Error::Io(e)
    }
}

impl From<url::ParseError> for Error {
    fn from(e: url::ParseError) -> Self {
        Error::Url(e.to_string())
    }
}

impl From<String> for Error {
    fn from(e: String) -> Self {
        Error::Other(e)
    }
}

impl From<&str> for Error {
    fn from(e: &str) -> Self {
        Error::Other(e.to_owned())
    }
}

/// 把 reqwest 的错误压成人类可读的一行，尽量贴近 .NET 的异常消息粒度。
pub fn describe_reqwest_error(e: &reqwest::Error) -> String {
    if e.is_timeout() {
        return format!("请求超时（{}）", e);
    }
    if e.is_connect() {
        return format!("连接失败（{}）", e);
    }
    if e.is_redirect() {
        return format!("重定向过多（{}）", e);
    }
    if e.is_decode() {
        return format!("响应解码失败（{}）", e);
    }
    e.to_string()
}
