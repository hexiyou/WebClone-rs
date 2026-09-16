//! 文本文件的读写。读取时自动识别编码，写出时统一 UTF-8 无 BOM。

use std::path::Path;

use super::encoding::{self, DecodedText};

pub fn read(path: &Path, http_charset: Option<&str>, assume_html: bool) -> std::io::Result<DecodedText> {
    let bytes = std::fs::read(path)?;
    Ok(encoding::decode(&bytes, http_charset, assume_html))
}

pub fn read_bytes(path: &Path) -> std::io::Result<Vec<u8>> {
    std::fs::read(path)
}

/// 统一以 UTF-8 无 BOM 写出。
pub fn write_utf8(path: &Path, content: &str) -> std::io::Result<()> {
    std::fs::write(path, content.as_bytes())
}
