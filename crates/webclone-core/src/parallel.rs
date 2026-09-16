//! 极简并发执行器。
//!
//! 对应 C# 的 Parallel.ForEachAsync + MaxDegreeOfParallelism：
//! 起 N 个工作线程抢一个原子递增的下标，所有线程退出后本函数才返回。
//! 不引 rayon/tokio，几十行就能覆盖"一批 URL 限流并发下载"这个唯一需求。

use std::sync::atomic::{AtomicUsize, Ordering};

pub fn for_each_concurrent<T, F>(items: &[T], concurrency: usize, f: F)
where
    T: Sync,
    F: Fn(&T) + Sync,
{
    if items.is_empty() {
        return;
    }

    let workers = concurrency.max(1).min(items.len());
    if workers == 1 {
        for item in items {
            f(item);
        }
        return;
    }

    let next = AtomicUsize::new(0);
    std::thread::scope(|scope| {
        for _ in 0..workers {
            scope.spawn(|| loop {
                let index = next.fetch_add(1, Ordering::Relaxed);
                if index >= items.len() {
                    break;
                }
                f(&items[index]);
            });
        }
    });
}
