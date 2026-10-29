//! OSC 序列数据类型契约与命令追踪器。

use std::time::Instant;

/// OSC 载荷缓冲上限。
pub(crate) const MAX_PAYLOAD_BYTES: usize = 4096;

pub(crate) const OSC_INTRO: u8 = 0x5d;

/// 一条 OSC 序列携带的、上层关心的语义。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum OscEvent {
    /// Shell 提示符开始（上一条命令已结束，等待输入）。
    PromptStart,
    /// 命令开始执行（用户已提交输入）。
    CommandStart,
    /// 命令执行结束。退出码与耗时都可能缺失。
    CommandFinish {
        exit_code: Option<i32>,
        duration_ms: Option<u64>,
    },
    /// 命令即将执行（`C` 或 WezTerm 的 `E`）。
    PreExec { command: Option<String> },
    /// 采集到的一条完整命令文本，用于命令历史与建议。
    CommandRecorded { command: String },
    /// 程序显式请求的桌面通知（OSC 777）。
    Notify { title: String, body: String },
    /// Shell 集成探针的回报（`OSC 777;transom;shell=<路径>`）。
    ///
    /// 值为空串表示远端 `$SHELL` 未设置；空串与"没有回报"是两种不同结论，
    /// 前者说明探针确实被执行了，只是拿不到方言，必须区分才能停止重试。
    ShellReport { shell: String },
    /// 注入脚本自报已安装钩子（`OSC 777;transom;ready=<方言>`）。
    IntegrationReady { shell: String },
}

/// 一条已完成命令的归并结果。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct TrackedCommand {
    pub(crate) exit_code: Option<i32>,
    pub(crate) duration_ms: u64,
}

/// 把零散的 OSC 133 标记归并成"一条命令跑了多久"。
#[derive(Debug, Default)]
pub(crate) struct CommandTracker {
    started_at: Option<Instant>,
}

impl CommandTracker {
    /// 处理一个 OSC 事件；`now` 由调用方注入，便于测试。
    pub(crate) fn on_event(&mut self, event: &OscEvent, now: Instant) -> Option<TrackedCommand> {
        match event {
            OscEvent::CommandStart => {
                self.started_at = Some(now);
                None
            }
            OscEvent::CommandFinish {
                exit_code,
                duration_ms,
            } => {
                let started_at = self.started_at.take()?;
                let measured = duration_ms.unwrap_or_else(|| {
                    now.checked_duration_since(started_at)
                        .map_or(0, |elapsed| elapsed.as_millis() as u64)
                });
                Some(TrackedCommand {
                    exit_code: *exit_code,
                    duration_ms: measured,
                })
            }
            OscEvent::PreExec { .. } | OscEvent::CommandRecorded { .. } => None,
            OscEvent::PromptStart => {
                let started_at = self.started_at.take()?;
                let elapsed = now.checked_duration_since(started_at)?.as_millis() as u64;
                Some(TrackedCommand {
                    exit_code: None,
                    duration_ms: elapsed,
                })
            }
            OscEvent::Notify { .. } => None,
            // 集成握手不参与"命令耗时/退出码"归并：它发生在任何命令边界之外。
            OscEvent::ShellReport { .. } | OscEvent::IntegrationReady { .. } => None,
        }
    }
}
