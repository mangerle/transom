//! 容器镜像下载管理器：有界并发拉取队列与分层进度跟踪。
//!
//! 设计原理（Why）：
//! - 将镜像拉取从原先的一次性阻塞弹窗升级为全局有界并发任务队列；
//! - 限制最大并发拉取数为 2（超过的任务自动进入 Queued 排队），避免同时拉取打满宿主机
//!   带宽或触发 Docker daemon 的 IO 抖动；
//! - 实时维护各分层（Layer）进度并汇聚总体下载/解压完成度。

use std::collections::HashMap;
use transom_core::{ImageLayerProgress, TabId};

/// 镜像拉取任务状态。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum PullTaskState {
    /// 在队列中等待可用并发槽位。
    Queued,
    /// 正在与远端 Registry 进行分层流式拉取或解压。
    Pulling,
    /// 镜像拉取并载入本地完成。
    Success,
    /// 拉取失败，附带错误说明。
    Failed(String),
    /// 已被用户主动取消。
    Cancelled,
}

impl PullTaskState {
    pub(crate) fn is_active(&self) -> bool {
        matches!(self, Self::Queued | Self::Pulling)
    }

    pub(crate) fn is_finished(&self) -> bool {
        matches!(self, Self::Success | Self::Failed(_) | Self::Cancelled)
    }

    pub(crate) fn label(&self) -> String {
        match self {
            Self::Queued => rust_i18n::t!("container.pull_manager.status_queued").to_string(),
            Self::Pulling => rust_i18n::t!("container.pull_manager.status_pulling").to_string(),
            Self::Success => rust_i18n::t!("container.pull_manager.status_success").to_string(),
            Self::Failed(msg) => {
                rust_i18n::t!("container.pull_manager.status_failed", msg = msg).to_string()
            }
            Self::Cancelled => rust_i18n::t!("container.pull_manager.status_cancelled").to_string(),
        }
    }
}

/// 单个镜像拉取任务。
#[derive(Debug, Clone)]
pub(crate) struct ImagePullTask {
    pub(crate) id: String,
    pub(crate) tab_id: TabId,
    /// 目标镜像名及 tag（如 `nginx:alpine`, `redis:latest`）。
    pub(crate) reference: String,
    /// 来源仓库名称（如 `Docker Hub (官方)`, `私有 Harbor`）。
    pub(crate) registry_name: String,
    pub(crate) state: PullTaskState,
    /// 当前最新的一行状态描述（如 "Extracting", "Download complete"）。
    pub(crate) status_line: String,
    /// 各层详细进度映射（短 SHA -> 进度）。
    pub(crate) layers: HashMap<String, ImageLayerProgress>,
}

impl ImagePullTask {
    pub(crate) fn new(
        id: String,
        tab_id: TabId,
        reference: String,
        registry_name: String,
        state: PullTaskState,
    ) -> Self {
        Self {
            id,
            tab_id,
            reference,
            registry_name,
            state,
            status_line: String::new(),
            layers: HashMap::with_capacity(8),
        }
    }

    /// 计算任务整体下载进度百分比（0.0 ~ 1.0）。若未知则返回 None。
    pub(crate) fn progress_ratio(&self) -> Option<f32> {
        if self.state == PullTaskState::Success {
            return Some(1.0);
        }
        if self.layers.is_empty() {
            return None;
        }
        let mut total_bytes = 0u64;
        let mut current_bytes = 0u64;
        for layer in self.layers.values() {
            if layer.total > 0 {
                total_bytes = total_bytes.saturating_add(layer.total);
                current_bytes = current_bytes.saturating_add(layer.current.min(layer.total));
            }
        }
        if total_bytes == 0 {
            None
        } else {
            Some((current_bytes as f32 / total_bytes as f32).clamp(0.0, 1.0))
        }
    }
}

/// 镜像拉取调度管理器。
#[derive(Debug, Clone)]
pub(crate) struct ImagePullManager {
    pub(crate) tasks: Vec<ImagePullTask>,
    /// 最大并发拉取数（默认 2）。
    pub(crate) max_concurrent: usize,
}

impl Default for ImagePullManager {
    fn default() -> Self {
        Self {
            tasks: Vec::new(),
            max_concurrent: 2,
        }
    }
}

impl ImagePullManager {
    pub(crate) fn new() -> Self {
        Self::default()
    }

    /// 获取指定 ID 的任务。
    pub(crate) fn get(&self, task_id: &str) -> Option<&ImagePullTask> {
        self.tasks.iter().find(|t| t.id == task_id)
    }

    /// 当前正在下载中的任务数量。
    pub(crate) fn pulling_count(&self) -> usize {
        self.tasks
            .iter()
            .filter(|t| t.state == PullTaskState::Pulling)
            .count()
    }

    /// 加入一个新拉取任务。
    /// 若当前活跃下载数未达到上限，则直接置为 Pulling 并返回 (task_id, true) 提示调用方立即发指令；
    /// 否则置为 Queued，返回 (task_id, false)。
    pub(crate) fn enqueue(
        &mut self,
        tab_id: TabId,
        reference: String,
        registry_name: String,
    ) -> (String, bool) {
        let task_id = transom_core::ContainerRequestId::new().to_string();
        let should_start = self.pulling_count() < self.max_concurrent;
        let initial_state = if should_start {
            PullTaskState::Pulling
        } else {
            PullTaskState::Queued
        };

        let task = ImagePullTask::new(
            task_id.clone(),
            tab_id,
            reference,
            registry_name,
            initial_state,
        );
        self.tasks.push(task);
        (task_id, should_start)
    }

    /// 更新分层进度与状态行。
    pub(crate) fn on_progress(
        &mut self,
        task_id: &str,
        status: String,
        layers: Vec<ImageLayerProgress>,
    ) {
        if let Some(task) = self.tasks.iter_mut().find(|t| t.id == task_id) {
            task.status_line = status;
            for layer in layers {
                task.layers.insert(layer.id.clone(), layer);
            }
        }
    }

    /// 标记拉取任务结束。
    /// 若产生空闲并发槽位，自动调度下一个排队中的任务，返回其 (task_id, tab_id, reference)。
    pub(crate) fn on_finished(
        &mut self,
        task_id: &str,
        error: Option<String>,
    ) -> Option<(String, TabId, String)> {
        if let Some(task) = self.tasks.iter_mut().find(|t| t.id == task_id) {
            task.state = match error {
                Some(err) => PullTaskState::Failed(err),
                None => PullTaskState::Success,
            };
        }
        self.schedule_next()
    }

    /// 主动取消指定任务。
    /// 若被取消的任务处于 Pulling 状态，同样调度下一个排队中的任务。
    pub(crate) fn cancel_task(&mut self, task_id: &str) -> Option<(String, TabId, String)> {
        let mut was_pulling = false;
        if let Some(task) = self.tasks.iter_mut().find(|t| t.id == task_id) {
            was_pulling = task.state == PullTaskState::Pulling;
            task.state = PullTaskState::Cancelled;
        }
        if was_pulling {
            self.schedule_next()
        } else {
            None
        }
    }

    /// 移除单个非活跃任务。
    pub(crate) fn remove_task(&mut self, task_id: &str) {
        self.tasks.retain(|t| t.id != task_id || t.state.is_active());
    }

    /// 清空所有已完成、已失败或已取消的任务。
    pub(crate) fn clear_finished(&mut self) {
        self.tasks.retain(|t| !t.state.is_finished());
    }

    /// 内部调度：拉起下一个排队中的任务。
    fn schedule_next(&mut self) -> Option<(String, TabId, String)> {
        if self.pulling_count() >= self.max_concurrent {
            return None;
        }
        let next_queued = self
            .tasks
            .iter_mut()
            .find(|t| t.state == PullTaskState::Queued)?;
        next_queued.state = PullTaskState::Pulling;
        Some((
            next_queued.id.clone(),
            next_queued.tab_id,
            next_queued.reference.clone(),
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pull_manager_limits_concurrency_and_schedules() {
        let mut manager = ImagePullManager::new();
        let tab = TabId::new();

        // 任务 1：立即执行
        let (t1, start1) = manager.enqueue(tab, "nginx:latest".into(), "Docker Hub".into());
        assert!(start1);
        assert_eq!(manager.pulling_count(), 1);

        // 任务 2：立即执行（达到上限 2）
        let (t2, start2) = manager.enqueue(tab, "redis:alpine".into(), "Docker Hub".into());
        assert!(start2);
        assert_eq!(manager.pulling_count(), 2);

        // 任务 3：排队中
        let (t3, start3) = manager.enqueue(tab, "postgres:15".into(), "Docker Hub".into());
        assert!(!start3);
        assert_eq!(manager.pulling_count(), 2);

        // 任务 1 完成 -> 自动调度任务 3
        let next = manager.on_finished(&t1, None);
        assert_eq!(next, Some((t3.clone(), tab, "postgres:15".into())));
        assert_eq!(manager.pulling_count(), 2);

        // 任务 2 取消 -> 此时没有新任务可排队
        let next2 = manager.cancel_task(&t2);
        assert_eq!(next2, None);
        assert_eq!(manager.pulling_count(), 1);

        // 任务 3 完成
        manager.on_finished(&t3, None);
        assert_eq!(manager.pulling_count(), 0);
    }
}
