//! AI 托管任务中心（Managed Task Runner）。
//!
//! 设计原理（Why）：
//! - 将耗时命令（本机/远端 Shell、大目录上传、容器/Compose 部署）与子 Agent 统一纳入后台任务表；
//! - 每个任务拥有独立 `task_id`、独立 `CancellationToken`、实时流式日志通道以及可选的远端进程组 PID；
//! - 用户在 UI 上点击单个任务的“停止”按钮时，可精确中止该任务而不打断整轮主对话，并对远端进程组发信号清理。

use std::collections::{HashMap, VecDeque};
use std::future::Future;
use std::panic::AssertUnwindSafe;
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use futures::FutureExt as _;
use parking_lot::Mutex;
use tokio_util::sync::CancellationToken;
use tokio_util::task::TaskTracker;
use transom_core::{AiAppEvent, AiManagedTaskDto, AiManagedTaskStatus, AppEvent};
use transom_net::SshRemote;
use uuid::Uuid;

struct ActiveTaskEntry {
    cancel: CancellationToken,
    detach: CancellationToken,
    remote: Option<SshRemote>,
    remote_pid: Arc<AtomicU32>,
    steer_queue: Option<Arc<Mutex<VecDeque<String>>>>,
}

/// 控制平面托管任务注册表。
pub(in crate::controller) struct ManagedTaskRegistry {
    event_tx: flume::Sender<AppEvent>,
    tasks: Mutex<HashMap<String, ActiveTaskEntry>>,
    background: TaskTracker,
}

impl ManagedTaskRegistry {
    pub(in crate::controller) fn new(event_tx: flume::Sender<AppEvent>) -> Self {
        Self {
            event_tx,
            tasks: Mutex::new(HashMap::new()),
            background: TaskTracker::new(),
        }
    }

    /// 将后台 Future 纳入统一生命周期跟踪，并记录 panic。
    pub(in crate::controller) fn spawn_background<F>(&self, future: F)
    where
        F: Future<Output = ()> + Send + 'static,
    {
        let handle = self.background.spawn(async move {
            if let Err(payload) = AssertUnwindSafe(future).catch_unwind().await {
                log::error!("AI 后台任务异常崩溃: {}", panic_payload(&payload));
            }
        });
        // TaskTracker 已持有任务登记；JoinHandle 本身不需要由调用方保存。
        drop(handle);
    }

    /// 关闭后台任务跟踪，停止接收新的登记。
    pub(in crate::controller) fn close_background(&self) {
        self.background.close();
    }

    /// 等待所有已登记后台任务退出。
    pub(in crate::controller) async fn wait_background(&self) {
        self.background.wait().await;
    }

    fn emit_event(&self, event: AppEvent) {
        match self.event_tx.try_send(event) {
            Ok(()) => {}
            Err(flume::TrySendError::Full(event)) => {
                let event_tx = self.event_tx.clone();
                self.spawn_background(async move {
                    if let Err(error) = event_tx.send_async(event).await {
                        log::warn!("发送 AI 托管任务状态事件失败: {error}");
                    }
                });
            }
            Err(flume::TrySendError::Disconnected(_)) => {
                log::warn!("发送 AI 托管任务状态事件失败: 事件通道已关闭");
            }
        }
    }

    /// 注册并启动一个新的托管任务，推送 `ManagedTaskStarted` 事件给 UI。
    pub(in crate::controller) fn start_task(
        self: &Arc<Self>,
        kind: &str,
        title: &str,
        target_label: &str,
        parent_cancel: &CancellationToken,
        remote: Option<SshRemote>,
    ) -> ManagedTaskHandle {
        let task_id = format!("task-{}", Uuid::new_v4().simple());
        let cancel = parent_cancel.child_token();
        let detach = CancellationToken::new();
        let remote_pid = Arc::new(AtomicU32::new(0));
        self.tasks.lock().insert(
            task_id.clone(),
            ActiveTaskEntry {
                cancel: cancel.clone(),
                detach: detach.clone(),
                remote,
                remote_pid: remote_pid.clone(),
                steer_queue: None,
            },
        );
        let dto = AiManagedTaskDto {
            task_id: task_id.clone(),
            kind: kind.to_owned(),
            title: title.to_owned(),
            target_label: target_label.to_owned(),
            status: AiManagedTaskStatus::Running,
            started_at_ms: now_epoch_ms(),
            finished_at_ms: None,
            exit_code: None,
        };
        self.emit_event(AppEvent::Ai(Box::new(AiAppEvent::ManagedTaskStarted(dto))));
        ManagedTaskHandle {
            task_id,
            cancel,
            detach,
            remote_pid,
            registry: self.clone(),
            finished: false,
            line_buf: Mutex::new(String::new()),
            pgid_parsed: std::sync::atomic::AtomicBool::new(false),
        }
    }

    /// 为子 Agent 等可交互任务绑定中途引导（Steer）指令队列。
    pub(in crate::controller) fn attach_steer_queue(
        &self,
        task_id: &str,
        queue: Arc<Mutex<VecDeque<String>>>,
    ) {
        if let Some(entry) = self.tasks.lock().get_mut(task_id) {
            entry.steer_queue = Some(queue);
        }
    }

    /// 向正在运行的子 Agent 注入追加指令，并同步推送一条日志标记。
    pub(in crate::controller) fn steer_task(&self, task_id: &str, message: String) -> bool {
        let queue = self
            .tasks
            .lock()
            .get(task_id)
            .and_then(|e| e.steer_queue.clone());
        let Some(queue) = queue else {
            return false;
        };
        let trimmed = message.trim().to_owned();
        if trimmed.is_empty() {
            return false;
        }
        queue.lock().push_back(trimmed.clone());
        self.emit_event(AppEvent::Ai(Box::new(AiAppEvent::ManagedTaskOutput {
            task_id: task_id.to_owned(),
            chunk: format!("[追加指令] {trimmed}\n"),
        })));
        true
    }

    /// 触发单个任务“转入后台不阻塞主对话”。
    pub(in crate::controller) fn detach_task(&self, task_id: &str) -> bool {
        let guard = self.tasks.lock();
        if let Some(entry) = guard.get(task_id) {
            entry.detach.cancel();
            true
        } else {
            false
        }
    }

    /// 取消单个托管任务；若存在远端进程组 PID，则返回 `(SshRemote, pid)` 供异步清理。
    pub(in crate::controller) fn cancel_task(&self, task_id: &str) -> Option<(SshRemote, u32)> {
        let guard = self.tasks.lock();
        let entry = guard.get(task_id)?;
        entry.cancel.cancel();
        let pid = entry.remote_pid.load(Ordering::Acquire);
        entry
            .remote
            .clone()
            .filter(|_| pid > 0)
            .map(|remote| (remote, pid))
    }

    /// 取消当前全部在途托管任务（控制平面停机时调用）。
    pub(in crate::controller) fn cancel_all(&self) {
        let guard = self.tasks.lock();
        for entry in guard.values() {
            entry.cancel.cancel();
        }
    }
}

/// 单个托管任务的运行期句柄（支持实时流式日志与完成回收）。
pub(in crate::controller) struct ManagedTaskHandle {
    pub(in crate::controller) task_id: String,
    pub(in crate::controller) cancel: CancellationToken,
    pub(in crate::controller) detach: CancellationToken,
    remote_pid: Arc<AtomicU32>,
    registry: Arc<ManagedTaskRegistry>,
    finished: bool,
    line_buf: Mutex<String>,
    pgid_parsed: std::sync::atomic::AtomicBool,
}

impl ManagedTaskHandle {
    /// 推送一段实时输出（自动剥离内部 `__TRANSOM_PGID__:<pid>` 标记行）。
    pub(in crate::controller) fn emit_chunk(&self, raw: &str) {
        if raw.is_empty() {
            return;
        }
        let chunk = if !self.pgid_parsed.load(Ordering::Acquire) {
            let mut buf = self.line_buf.lock();
            buf.push_str(raw);
            if let Some(nl) = buf.find('\n') {
                let first_line = buf[..nl].trim();
                if let Some(pid_str) = first_line.strip_prefix("__TRANSOM_PGID__:") {
                    if let Ok(pid) = pid_str.trim().parse::<u32>() {
                        self.remote_pid.store(pid, Ordering::Release);
                    }
                    self.pgid_parsed.store(true, Ordering::Release);
                    buf[nl + 1..].to_owned()
                } else {
                    self.pgid_parsed.store(true, Ordering::Release);
                    std::mem::take(&mut *buf)
                }
            } else if buf.len() > 64 || !buf.starts_with("__TRANSOM_PGID__:") {
                self.pgid_parsed.store(true, Ordering::Release);
                std::mem::take(&mut *buf)
            } else {
                String::new()
            }
        } else {
            raw.to_owned()
        };

        if !chunk.is_empty() {
            match self.registry.event_tx.try_send(AppEvent::Ai(Box::new(
                AiAppEvent::ManagedTaskOutput {
                    task_id: self.task_id.clone(),
                    chunk,
                },
            ))) {
                Ok(()) => {}
                Err(flume::TrySendError::Full(_)) => {
                    log::trace!("AI 托管任务输出事件通道已满，丢弃实时输出分片");
                }
                Err(flume::TrySendError::Disconnected(_)) => {
                    log::warn!("发送 AI 托管任务输出失败，事件通道已关闭");
                }
            }
        }
    }

    /// 标记任务结束并发送 `ManagedTaskFinished` 事件。
    pub(in crate::controller) fn finish(
        mut self,
        status: AiManagedTaskStatus,
        exit_code: Option<i32>,
        summary: impl Into<String>,
    ) {
        self.finished = true;
        self.registry.tasks.lock().remove(&self.task_id);
        self.registry
            .emit_event(AppEvent::Ai(Box::new(AiAppEvent::ManagedTaskFinished {
                task_id: self.task_id.clone(),
                status,
                exit_code,
                summary: summary.into(),
            })));
    }
}

impl Drop for ManagedTaskHandle {
    fn drop(&mut self) {
        if !self.finished {
            self.registry.tasks.lock().remove(&self.task_id);
            let status = if self.cancel.is_cancelled() {
                AiManagedTaskStatus::Cancelled
            } else {
                AiManagedTaskStatus::Failed
            };
            self.registry
                .emit_event(AppEvent::Ai(Box::new(AiAppEvent::ManagedTaskFinished {
                    task_id: self.task_id.clone(),
                    status,
                    exit_code: None,
                    summary: "任务已结束".to_owned(),
                })));
        }
    }
}

/// 为远端命令包裹进程组/子进程 PID 上报头，保持退出码不变。
pub(in crate::controller) fn wrap_remote_command_with_pgid(command: &str) -> String {
    let escaped = command.replace('\'', "'\"'\"'");
    format!("sh -c '{escaped}' & _tp=$!; printf '__TRANSOM_PGID__:%s\\n' \"$_tp\"; wait \"$_tp\"")
}

/// 剥离捕获输出首行可能残留的 `__TRANSOM_PGID__:<pid>` 标记行。
pub(in crate::controller) fn strip_pgid_marker(stdout: &str) -> String {
    if let Some(rest) = stdout.strip_prefix("__TRANSOM_PGID__:")
        && let Some(nl) = rest.find('\n')
    {
        return rest[nl + 1..].to_owned();
    }
    stdout.to_owned()
}

/// 限时向远端进程发送 `TERM` / `KILL` 信号以清理残留进程组。
pub(in crate::controller) async fn kill_remote_pid(remote: SshRemote, pid: u32) {
    let cmd = format!(
        "kill -TERM -{pid} {pid} 2>/dev/null || true; sleep 1; kill -KILL -{pid} {pid} 2>/dev/null || true"
    );
    match tokio::time::timeout(Duration::from_secs(5), remote.exec_command_capture(&cmd)).await {
        Ok(Ok(result)) if result.exit_status == Some(0) => {}
        Ok(Ok(result)) => log::warn!(
            "清理远端托管任务进程组返回异常退出码，PID: {pid}，退出码: {:?}，stderr: {}",
            result.exit_status,
            result.stderr
        ),
        Ok(Err(err)) => log::warn!("清理远端托管任务进程组失败，PID: {pid}，原因: {err}"),
        Err(_) => log::warn!("清理远端托管任务进程组超时（5 秒），PID: {pid}"),
    }
}

fn now_epoch_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

fn panic_payload(payload: &Box<dyn std::any::Any + Send>) -> String {
    if let Some(message) = payload.downcast_ref::<&str>() {
        (*message).to_owned()
    } else if let Some(message) = payload.downcast_ref::<String>() {
        message.clone()
    } else {
        "未知 panic 载荷".to_owned()
    }
}
