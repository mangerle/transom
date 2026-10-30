//! OSC 序列数据类型契约与命令追踪器。

use std::time::Instant;

/// OSC 载荷缓冲上限。
///
/// 取 128 KiB 是为了承载 OSC 52 剪贴板写入：base64 会膨胀约三分之一，实际
/// 可传文本约 96 KiB，覆盖 tmux 等常见的剪贴板转发用量级。上限仍必须存在：
/// 未终止的序列会持续占用缓冲，不设界就是一条内存耗尽通道。
pub(crate) const MAX_PAYLOAD_BYTES: usize = 128 * 1024;

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
    /// 程序上报的当前工作目录（OSC 7），已从 `file://` URI 解码为裸路径。
    ///
    /// 这是 cwd 的事实标准通道；窗口标题（OSC 0/2）只描述"终端里在跑什么"，
    /// 不再拿来猜路径。
    Cwd { path: String },
    /// 程序显式请求的桌面通知（OSC 777 / OSC 9）。
    Notify { title: String, body: String },
    /// 远端程序请求写入本地剪贴板（OSC 52），已解码为文本。
    ///
    /// 是否真的写入由终端视图按会话授权决定，解析层只负责解出内容：读请求
    /// （`?`）与主选择区（`p`/`s`）在解析阶段就被丢弃，不会到达这里。
    ClipboardStore { text: String },
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

/// 运行期标题门控的判定结果。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum TitleAction {
    /// 这是程序在命令运行期间设置的标题，可以更新标签。
    Publish,
    /// 运行期标题应清除，标签回落到会话名。
    Clear,
    /// 不属于运行期标题，忽略。
    Ignore,
}

/// 运行期标题门控：只放行命令执行期间设置的 OSC 0/2 标题。
///
/// 设计原理（Why）：
/// - 通用终端把 Shell 提示符和 TUI 写的标题一视同仁，标签会长期停在
///   `用户@主机: 目录`；Transom 的标签承载会话身份，不能让提示符标题接管；
/// - OSC 133 的命令边界提供了可靠证据：`B`/`C`（命令开始）之后、`D`/`A`
///   （命令结束/新提示符）之前设置的标题属于运行中的程序；边界之外是提示符
///   自己的标题，必须忽略；
/// - 关闭 Shell 集成时没有边界证据，门控保持关闭：宁可少显示 TUI 标题，
///   也不让 `用户@主机` 重新覆盖标签。
///
/// 代价与局限：嵌套 Shell 或 TUI 内部的提示符仍处于外层命令边界内，其标题
/// 会被视为程序标题；命令在下一帧前就结束时，标题不会发布。
#[derive(Debug, Default)]
pub(crate) struct TitleGate {
    command_running: bool,
    published: bool,
}

impl TitleGate {
    /// 处理一条 OSC 133 边界，返回是否需要清除已发布的运行期标题。
    pub(crate) fn on_boundary(&mut self, event: &OscEvent) -> TitleAction {
        match event {
            OscEvent::CommandStart | OscEvent::PreExec { .. } => {
                self.command_running = true;
                TitleAction::Ignore
            }
            OscEvent::CommandFinish { .. } | OscEvent::PromptStart => {
                self.command_running = false;
                self.clear()
            }
            _ => TitleAction::Ignore,
        }
    }

    /// 处理一条 OSC 0/2 标题；只有命令运行期间、且不像 Shell 提示符时才允许发布。
    ///
    /// 第二层过滤是给缺少 OSC 133 证据或边界顺序异常的会话兜底的：即使程序标题
    /// 在外层命令运行期间到达，`用户@主机[: 目录]` 这种典型提示符标题也不能覆盖
    /// 标签。它只认带 `@` 的最常见形态，正常的 TUI 标题（文件名、会话名、任务名）
    /// 不受影响。
    pub(crate) fn on_title(&mut self, title: &str) -> TitleAction {
        if !self.command_running || looks_like_shell_prompt_title(title) {
            return TitleAction::Ignore;
        }
        self.published = true;
        TitleAction::Publish
    }

    /// 处理标题栈重置或恢复默认标题。
    pub(crate) fn on_reset(&mut self) -> TitleAction {
        self.clear()
    }

    fn clear(&mut self) -> TitleAction {
        if self.published {
            self.published = false;
            TitleAction::Clear
        } else {
            TitleAction::Ignore
        }
    }
}

/// 是否是常见的 Shell 提示符标题形态：`用户@主机` 或 `用户@主机: 目录`。
///
/// 只匹配 `@` 前没有空白、且 `@` 后主机名以结尾或冒号收尾的形态，避免把
/// `user@example.com - mutt` 这类真正包含 `@` 的 TUI 标题误杀。
fn looks_like_shell_prompt_title(title: &str) -> bool {
    let title = title.trim();
    let Some((user, rest)) = title.split_once('@') else {
        return false;
    };
    if user.is_empty() || user.chars().any(|c| c.is_whitespace() || c == ':') {
        return false;
    }
    let host = rest
        .split(|c: char| c.is_whitespace() || c == ':')
        .next()
        .unwrap_or_default();
    if host.is_empty() {
        return false;
    }
    let after_host = &rest[host.len()..];
    if after_host.is_empty() || after_host.starts_with(':') {
        return true;
    }
    // `[user@host ~]$`、`user@host /path` 这类提示符形态；`user@host - nvim`
    // 的剩余部分以连字符开头，不当成提示符。
    let tail = after_host.trim_start();
    after_host.starts_with(' ')
        && (tail.starts_with('~')
            || tail.starts_with('/')
            || tail.starts_with('[')
            || tail.starts_with(']'))
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
            // `B` 是提示符区里的命令开始标记，部分集成会给；我们的 bash/zsh/fish
            // 钩子只发 `C`（PreExec）。两者都作为计时起点，但 `C` 才是命令真正
            // 开始执行的时刻，后到时覆盖 `B`，避免把用户输入的时间算进耗时。
            OscEvent::CommandStart => {
                if self.started_at.is_none() {
                    self.started_at = Some(now);
                }
                None
            }
            OscEvent::PreExec { .. } => {
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
            OscEvent::CommandRecorded { .. } => None,
            OscEvent::Cwd { .. } => None,
            OscEvent::PromptStart => {
                let started_at = self.started_at.take()?;
                let elapsed = now.checked_duration_since(started_at)?.as_millis() as u64;
                Some(TrackedCommand {
                    exit_code: None,
                    duration_ms: elapsed,
                })
            }
            OscEvent::Notify { .. } => None,
            // 剪贴板请求与命令耗时无关，也不构成边界。
            OscEvent::ClipboardStore { .. } => None,
            // 集成握手不参与"命令耗时/退出码"归并：它发生在任何命令边界之外。
            OscEvent::ShellReport { .. } | OscEvent::IntegrationReady { .. } => None,
        }
    }
}
