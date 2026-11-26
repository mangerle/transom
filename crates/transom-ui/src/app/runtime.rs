//! UI 侧阻塞任务调度：把本地 IO 移出 GPUI 后台执行器。
//!
//! # 设计原理（Why）
//! GPUI 的后台执行器同时承担缩略图解码等短任务，占住其工作线程会拖慢其它
//! 后台工作。这里使用进程级固定大小线程池：队列有界，任务可排队上限明确，
//! 避免每次文件操作都临时创建线程。
//!
//! # 代价与局限
//! 系统文件调用通常无法中途取消。Future 被丢弃时只保证尚未开始的任务不会
//! 执行；已经进入系统调用的任务会运行到返回，但线程数量仍受进程级上限约束。

use std::future::Future;
use std::sync::Arc;
use std::sync::OnceLock;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread::JoinHandle;

/// 排队任务上限；达到上限后异步侧等待，形成明确背压。
const BLOCKING_QUEUE_CAPACITY: usize = 128;
/// 阻塞任务最多同时占用 4 条线程，避免文件系统卡顿时耗尽线程资源。
const MAX_BLOCKING_WORKERS: usize = 4;

type BlockingJob = Box<dyn FnOnce() + Send + 'static>;

struct BlockingPool {
    sender: flume::Sender<BlockingJob>,
    /// 进程级工作线程句柄；随静态线程池存活到进程退出。
    _workers: Vec<JoinHandle<()>>,
}

impl BlockingPool {
    fn start() -> Option<Self> {
        let worker_count = std::thread::available_parallelism().map_or(2, |parallelism| {
            parallelism.get().clamp(2, MAX_BLOCKING_WORKERS)
        });
        let (sender, receiver) = flume::bounded(BLOCKING_QUEUE_CAPACITY);
        let mut workers = Vec::with_capacity(worker_count);
        for index in 0..worker_count {
            let receiver = receiver.clone();
            let spawned = std::thread::Builder::new()
                .name(format!("transom-blocking-io-{index}"))
                .spawn(move || run_worker(receiver));
            match spawned {
                Ok(handle) => workers.push(handle),
                Err(error) => log::error!("启动阻塞 IO 工作线程失败，序号: {index}，原因: {error}"),
            }
        }
        if workers.is_empty() {
            return None;
        }
        Some(Self {
            sender,
            _workers: workers,
        })
    }
}

fn run_worker(receiver: flume::Receiver<BlockingJob>) {
    while let Ok(job) = receiver.recv() {
        job();
    }
}

fn blocking_pool() -> Option<&'static BlockingPool> {
    static POOL: OnceLock<Option<BlockingPool>> = OnceLock::new();
    POOL.get_or_init(BlockingPool::start).as_ref()
}

/// 取消守卫：调用方 Future 被丢弃时，让尚未开始的排队任务直接跳过。
struct CancelOnDrop(Arc<AtomicBool>);

impl Drop for CancelOnDrop {
    fn drop(&mut self) {
        self.0.store(true, Ordering::Release);
    }
}

/// 在进程级阻塞线程池执行本地文件操作。
///
/// 返回值是 `Option<T>`：线程池或结果通道不可用时返回 `None`，调用方据此走
/// 取消分支，而不是把错误伪装成一个合法结果。
pub(crate) fn spawn_blocking_local<T, F>(job: F) -> impl Future<Output = Option<T>>
where
    T: Send + 'static,
    F: FnOnce() -> T + Send + 'static,
{
    let (result_tx, result_rx) = flume::bounded(1);
    let cancel = Arc::new(AtomicBool::new(false));
    let queued_cancel = Arc::clone(&cancel);
    let task = move || {
        if queued_cancel.load(Ordering::Acquire) {
            return;
        }
        if result_tx.send(job()).is_err() {
            log::debug!("阻塞 IO 结果接收端已关闭，任务结果被丢弃");
        }
    };
    let queued: BlockingJob = Box::new(task);
    let pool = blocking_pool();

    async move {
        let _cancel_guard = CancelOnDrop(cancel);
        let Some(pool) = pool else {
            log::error!("阻塞 IO 线程池不可用，本次操作已取消");
            return None;
        };
        if pool.sender.send_async(queued).await.is_err() {
            log::error!("阻塞 IO 任务队列已关闭，本次操作已取消");
            return None;
        }
        result_rx.recv_async().await.ok()
    }
}

#[cfg(test)]
mod tests {
    use super::{BlockingPool, CancelOnDrop};
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::time::Duration;

    #[test]
    fn blocking_pool_returns_job_result() {
        let pool = BlockingPool::start().expect("阻塞线程池应可启动");
        let (sender, receiver) = flume::bounded(1);
        pool.sender
            .send(Box::new(move || {
                let _ = sender.send(42);
            }))
            .expect("任务应可入队");
        assert_eq!(
            receiver.recv_timeout(Duration::from_secs(1)),
            Ok(42),
            "阻塞线程池应执行任务并返回结果"
        );
    }

    #[test]
    fn cancel_guard_marks_queued_job_cancelled() {
        let cancelled = Arc::new(AtomicBool::new(false));
        {
            let _guard = CancelOnDrop(Arc::clone(&cancelled));
        }
        assert!(cancelled.load(Ordering::Acquire), "守卫析构时必须标记取消");
    }
}
