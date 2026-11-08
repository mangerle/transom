//! UI 侧阻塞任务调度：把可能运行数十秒的本地 IO 移出 GPUI 后台执行器。
//!
//! # 设计原理（Why）
//! GPUI 的后台执行器同时承担缩略图解码等短任务，占住其工作线程会拖慢其它
//! 后台工作。这里为每次阻塞操作开一条一次性专用线程，异步侧收到结果即 join，
//! 线程生命周期全程可追踪，不产生脱缰线程。
//!
//! # 归属
//! 本模块是 crate 级基础设施，被 sftp、container、settings、record、audit 与
//! diff 共同使用；不再寄居在任一业务模块下，避免业务模块之间经由 sftp 反向
//! 依赖通用能力。
//!
//! # 代价与局限
//! 每次调用创建一条线程，适合低频、长耗时的文件操作；高频小任务仍应使用
//! GPUI 自带的后台执行器，避免线程创建开销。
use std::future::Future;

/// 在专用线程执行阻塞的本地文件操作。
///
/// 返回值是 `Option<T>`：线程创建失败时返回 `None`，调用方据此走取消分支，
/// 而不是把错误伪装成一个合法结果。
pub(crate) fn spawn_blocking_local<T, F>(job: F) -> impl Future<Output = Option<T>>
where
    T: Send + 'static,
    F: FnOnce() -> T + Send + 'static,
{
    let (tx, rx) = flume::bounded(1);
    let spawned = std::thread::Builder::new()
        .name("transom-blocking-io".into())
        .spawn(move || {
            let _ = tx.send(job());
        });
    async move {
        let handle = match spawned {
            Ok(handle) => handle,
            Err(error) => {
                log::error!("创建本地文件操作线程失败，本次操作已取消: {error}");
                return None;
            }
        };
        let result = rx.recv_async().await.ok();
        let _ = handle.join();
        result
    }
}
