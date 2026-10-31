//! 状态快照的异步落盘。
//!
//! # 设计原理（Why）
//! - 会话目录与设置文件都在 UI 线程上被修改，若每次修改都同步 `fsync`，
//!   家目录位于 NFS、磁盘繁忙或文件较大时会把一次点击拖成几十毫秒的卡顿；
//! - 这里采用覆盖式单槽：只保留最新全量快照，写盘线程按自己的节奏取出最新值，
//!   中间态没有保留价值，全量快照天然幂等；
//! - 通道有界且只有一格，UI 侧 `enqueue` 只做一次内存替换，不会无限积压；
//! - Drop 时停机并 join 写盘线程，退出前最后一个快照不丢。
//!
//! # 代价与局限
//! `enqueue` 返回成功只代表“已进入内存与待写槽”，不代表已经落到介质；进程在
//! 写盘完成前被强杀会丢最后一个快照。需要“落盘后再继续”的场景（例如退出前
//! 保存窗口状态）仍走同步写。

use parking_lot::{Condvar, Mutex};
use std::sync::Arc;
use std::thread::JoinHandle;
use transom_core::{AppError, AppSettings, ErrorCode, SessionCatalog};

/// 覆盖式快照写盘器。
pub(crate) struct SnapshotWriter<T: Send + 'static> {
    worker: Arc<WorkerState<T>>,
    /// `None` 表示线程已因启动失败而没有拉起；Drop 时只做停机。
    handle: Option<JoinHandle<()>>,
}

struct WorkerState<T> {
    slot: Mutex<Slot<T>>,
    wake: Condvar,
    save: Box<dyn Fn(&T) + Send + Sync>,
    label: &'static str,
}

struct Slot<T> {
    pending: Option<T>,
    shutdown: bool,
}

impl<T: Send + 'static> SnapshotWriter<T> {
    /// 创建写盘器并拉起常驻线程。
    ///
    /// 线程启动失败时返回 `None`：调用方应保留同步写作为兜底，不能因为
    /// 后台写盘不可用就让设置/会话完全无法保存。
    pub(crate) fn spawn(
        label: &'static str,
        save: impl Fn(&T) + Send + Sync + 'static,
    ) -> Option<Self> {
        let worker = Arc::new(WorkerState {
            slot: Mutex::new(Slot {
                pending: None,
                shutdown: false,
            }),
            wake: Condvar::new(),
            save: Box::new(save),
            label,
        });
        let thread_worker = Arc::clone(&worker);
        let handle = std::thread::Builder::new()
            .name(format!("transom-{label}"))
            .spawn(move || run_writer(thread_worker))
            .map_err(|error| log::error!("启动{label}写盘线程失败，原因: {error}"))
            .ok()?;
        Some(Self {
            worker,
            handle: Some(handle),
        })
    }

    /// 覆盖式入队：新快照替换旧快照并唤醒写盘线程。
    pub(crate) fn enqueue(&self, value: T) -> Result<(), AppError> {
        let mut slot = self.worker.slot.lock();
        if slot.shutdown {
            return Err(AppError::with_detail(
                ErrorCode::Io,
                format!("{}写盘线程已停机", self.worker.label),
            ));
        }
        slot.pending = Some(value);
        self.worker.wake.notify_one();
        Ok(())
    }
}

impl<T: Send + 'static> Drop for SnapshotWriter<T> {
    fn drop(&mut self) {
        {
            let mut slot = self.worker.slot.lock();
            slot.shutdown = true;
            self.worker.wake.notify_all();
        }
        if let Some(handle) = self.handle.take()
            && handle.join().is_err()
        {
            log::error!(
                "{}写盘线程异常退出，最后一个快照可能未落盘",
                self.worker.label
            );
        }
    }
}

fn run_writer<T: Send + 'static>(worker: Arc<WorkerState<T>>) {
    loop {
        let value = {
            let mut slot = worker.slot.lock();
            while slot.pending.is_none() && !slot.shutdown {
                worker.wake.wait(&mut slot);
            }
            let value = slot.pending.take();
            if value.is_none() && slot.shutdown {
                return;
            }
            value
        };
        if let Some(value) = value {
            (worker.save)(&value);
        }
    }
}

/// 应用状态的两条写盘通道：会话目录与设置。
///
/// 分开两个写盘器而不是共用一个线程：会话目录通常远大于设置文件，二者
/// 互不阻塞；两个线程各自只有一格，资源开销固定。
pub(crate) struct Persistence {
    catalog: Option<SnapshotWriter<SessionCatalog>>,
    settings: Option<SnapshotWriter<AppSettings>>,
}

impl Persistence {
    /// 按存储实现创建两条写盘通道；线程启动失败时对应通道为 `None`，
    /// 调用方回退到同步写。
    pub(crate) fn new(
        session_store: Arc<dyn transom_core::SessionStore>,
        settings_store: Arc<dyn transom_core::SettingsStore>,
    ) -> Self {
        let catalog = SnapshotWriter::spawn("catalog", move |catalog| {
            if let Err(error) = session_store.save(catalog) {
                log::error!("会话目录异步落盘失败，原因: {error}");
            }
        });
        let settings = SnapshotWriter::spawn("settings", move |settings| {
            if let Err(error) = settings_store.save(settings) {
                log::error!("设置异步落盘失败，原因: {error}");
            }
        });
        Self { catalog, settings }
    }

    /// 入队会话目录；通道不可用时返回 false，由调用方同步兜底。
    pub(crate) fn enqueue_catalog(&self, catalog: SessionCatalog) -> bool {
        self.catalog
            .as_ref()
            .is_some_and(|writer| writer.enqueue(catalog).is_ok())
    }

    /// 入队设置；通道不可用时返回 false，由调用方同步兜底。
    pub(crate) fn enqueue_settings(&self, settings: AppSettings) -> bool {
        self.settings
            .as_ref()
            .is_some_and(|writer| writer.enqueue(settings).is_ok())
    }
}

#[cfg(test)]
mod tests {
    use super::SnapshotWriter;
    use parking_lot::Mutex;
    use std::sync::Arc;

    /// 覆盖式单槽：连续入队两次，退出时必须落盘最后一条，且至少写一次。
    #[test]
    fn writer_flushes_latest_snapshot_on_drop() {
        let last = Arc::new(Mutex::new(None::<i32>));
        let written = Arc::new(Mutex::new(0usize));
        {
            let last = Arc::clone(&last);
            let written = Arc::clone(&written);
            let writer = SnapshotWriter::spawn("test", move |value: &i32| {
                *last.lock() = Some(*value);
                *written.lock() += 1;
            })
            .expect("测试写盘线程应能启动");
            writer.enqueue(1).expect("第一次入队应成功");
            writer.enqueue(2).expect("第二次入队应成功");
        }
        assert_eq!(*last.lock(), Some(2), "退出前必须写下最后一份快照");
        assert!(*written.lock() >= 1, "退出前至少写一次");
    }
}
