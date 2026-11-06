//! 请求取消登记表。
//!
//! 设计原理（Why）：读请求（日志、差异）由界面按需发起，标签页可能在子进程
//! 返回前就被关闭。这里不做强行 abort——git 子进程自带超时，abort 反而会让
//! 输出管道半路废弃；改为「完成时检查标志，已取消则丢弃结果」，既省掉跨任务
//! 传递句柄的复杂度，又保证迟到的数据不会写进新视图。

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use transom_core::GitRequestId;

/// Git 请求登记表（随控制器单例）。
#[derive(Default)]
pub struct GitService {
    cancels: parking_lot::Mutex<HashMap<GitRequestId, Arc<AtomicBool>>>,
}

impl GitService {
    pub fn new() -> Self {
        Self::default()
    }

    /// 登记一次可取消请求，返回取消标志。
    pub fn register(&self, request_id: GitRequestId) -> Arc<AtomicBool> {
        let flag = Arc::new(AtomicBool::new(false));
        self.cancels.lock().insert(request_id, Arc::clone(&flag));
        flag
    }

    /// 置位取消标志；请求已结束返回 `false`。
    pub fn cancel(&self, request_id: GitRequestId) -> bool {
        match self.cancels.lock().remove(&request_id) {
            Some(flag) => {
                flag.store(true, Ordering::Relaxed);
                true
            }
            None => false,
        }
    }

    /// 请求自然结束后移除登记，避免表随翻页无限增长。
    pub fn finish(&self, request_id: GitRequestId) {
        self.cancels.lock().remove(&request_id);
    }

    /// 标志是否已置位（未登记视为未取消）。
    pub fn is_cancelled(flag: &Arc<AtomicBool>) -> bool {
        flag.load(Ordering::Relaxed)
    }
}

#[cfg(test)]
mod tests {
    use super::GitService;
    use transom_core::GitRequestId;

    #[test]
    fn cancel_flag_survives_removal_from_registry() {
        let service = GitService::new();
        let id = GitRequestId::new();
        let flag = service.register(id);
        assert!(!GitService::is_cancelled(&flag));
        assert!(service.cancel(id));
        assert!(GitService::is_cancelled(&flag), "已取走的标志仍须可见取消");
        assert!(!service.cancel(id), "重复取消必须返回 false");
    }

    #[test]
    fn finish_clears_the_registry() {
        let service = GitService::new();
        let id = GitRequestId::new();
        let flag = service.register(id);
        service.finish(id);
        assert!(!GitService::is_cancelled(&flag));
        assert!(!service.cancel(id));
    }
}
