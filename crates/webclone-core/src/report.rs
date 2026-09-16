//! 一次克隆任务的汇总报告。

use crate::models::CloneMode;

#[derive(Debug, Clone)]
pub struct CloneReport {
    pub output_directory: String,
    pub mode: CloneMode,
    pub pages_downloaded: usize,
    pub pages_skipped: usize,
    pub assets_downloaded: usize,
    pub assets_skipped: usize,
    pub failures: usize,
    pub bytes_written: u64,
    pub elapsed: std::time::Duration,
    pub errors: Vec<String>,
}

impl CloneReport {
    pub fn total_items(&self) -> usize {
        self.pages_downloaded + self.pages_skipped + self.assets_downloaded + self.assets_skipped
    }

    /// 与 C# 版逐字一致的汇总文案。
    pub fn summary(&self) -> String {
        format!(
            "完成：页面 {} 个（跳过 {}），资源 {} 个（跳过 {}），失败 {} 项，共 {:.2} MB，耗时 {:.1} 秒。",
            self.pages_downloaded,
            self.pages_skipped,
            self.assets_downloaded,
            self.assets_skipped,
            self.failures,
            self.bytes_written as f64 / 1024.0 / 1024.0,
            self.elapsed.as_secs_f64(),
        )
    }
}
