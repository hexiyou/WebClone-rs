//! 取消令牌。
//!
//! 对应 C# 的 CancellationToken：一个可以跨线程共享的布尔开关。
//! 下载/扫描循环在关键节点上轮询它，命中就返回 `Error::Cancelled`。

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

#[derive(Clone, Default)]
pub struct CancelToken(Arc<AtomicBool>);

impl CancelToken {
    pub fn new() -> Self {
        Self(Arc::new(AtomicBool::new(false)))
    }

    pub fn cancel(&self) {
        self.0.store(true, Ordering::SeqCst);
    }

    pub fn is_cancelled(&self) -> bool {
        self.0.load(Ordering::SeqCst)
    }
}
