//! Git 克隆命令与进度批处理。

use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use super::super::state::{ControllerCtx, ControllerState};
use super::runner::{send_event, to_app_error};
use transom_core::{AppEvent, GitAppEvent, GitRequestId};

/// 派发一次克隆。
///
/// 设计原理（Why）：这是唯一一条会持续产出内容的命令，所以不能沿用
/// `spawn_read`/`spawn_write` 的"一个任务发一个事件"形态——克隆要边跑边给界面
/// 反馈。取消沿用同一枚标志：引擎在轮询到置位时真的去 `start_kill` 子进程，
/// 而不是像读请求那样"跑完再丢结果"，否则会留下半个仓库和一个在写的 git。
pub(super) fn spawn_clone(
    state: &mut ControllerState,
    ctx: &ControllerCtx,
    request_id: GitRequestId,
    url: String,
    dest: PathBuf,
    depth: Option<u32>,
) {
    let service = Arc::clone(&ctx.git);
    let config = ctx.git_config_snapshot();
    let event_tx = ctx.event_tx.clone();
    let flag = service.register(request_id);
    state.git_set.spawn(async move {
        let mut batch = ProgressBatch::new(&event_tx, request_id);
        let outcome =
            transom_git::clone_with_progress(&url, &dest, depth, &flag, &config, |line| {
                batch.push(line);
            })
            .await;
        // 收尾前把窗口里剩的半批也发出去，否则最后几行进度永远不显示。
        batch.flush();
        service.finish(request_id);
        let error = match outcome {
            Ok(()) => None,
            Err(error) => {
                log::error!(
                    "克隆远端仓库失败，地址: {url}，目标: {}，原因: {error}",
                    dest.display()
                );
                Some(to_app_error(error))
            }
        };
        send_event(
            &event_tx,
            GitAppEvent::CloneDone {
                request_id,
                dest,
                error,
            },
        );
    });
}

/// 克隆进度的时间窗合并器。
///
/// 设计原理（Why）：事件通道有界且用 `try_send`（打满就丢并记一条警告），
/// 而 `git clone --progress` 在慢网络上每秒能刷几十行，逐行发等于自己把队列
/// 灌满、把真正的完成事件挤掉。这里攒够 150 毫秒一次性发一条，界面看到的
/// 是连续的进度而不是断档，队列也始终留有余量。
struct ProgressBatch {
    tx: flume::Sender<AppEvent>,
    request_id: GitRequestId,
    buffer: String,
    since: Instant,
}

impl ProgressBatch {
    fn new(tx: &flume::Sender<AppEvent>, request_id: GitRequestId) -> Self {
        Self {
            tx: tx.clone(),
            request_id,
            buffer: String::new(),
            since: Instant::now(),
        }
    }

    /// 追加一行；攒到时间窗就发一批。
    fn push(&mut self, line: &str) {
        if !self.buffer.is_empty() {
            self.buffer.push('\n');
        }
        self.buffer.push_str(line);
        if self.since.elapsed() >= Duration::from_millis(150) {
            self.flush();
        }
    }

    /// 把攒下的内容发出去；空批不发。
    fn flush(&mut self) {
        if self.buffer.is_empty() {
            return;
        }
        self.since = Instant::now();
        let text = std::mem::take(&mut self.buffer);
        send_event(
            &self.tx,
            GitAppEvent::CloneProgress {
                request_id: self.request_id,
                text,
            },
        );
    }
}
