//! 进度事件。

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CloneProgressKind {
    Started,
    PageDiscovered,
    PageDownloaded,
    PageSkipped,
    AssetDownloaded,
    AssetSkipped,
    AssetFailed,
    Rewriting,
    Completed,
    Failed,
}

/// 进度事件。回调里不要做耗时操作。
#[derive(Debug, Clone)]
pub struct CloneProgress {
    pub kind: CloneProgressKind,
    pub message: String,
    pub completed: usize,
    pub total: usize,
}

impl CloneProgress {
    pub fn new(kind: CloneProgressKind, message: impl Into<String>) -> Self {
        Self {
            kind,
            message: message.into(),
            completed: 0,
            total: 0,
        }
    }

    pub fn is_error(&self) -> bool {
        matches!(
            self.kind,
            CloneProgressKind::AssetFailed | CloneProgressKind::Failed
        )
    }
}
