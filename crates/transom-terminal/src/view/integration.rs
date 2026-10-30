//! Shell 集成的视图侧驱动：把会话观察量喂给状态机，并按它的指令写探针与脚本。
//!
//! 设计原理（Why）：
//! - 状态机（[`crate::shell_integration`]）刻意做成纯函数，不看网格也不看通道；
//!   本文件是它唯一的外部接口，负责"什么时候算安全时机"这类只能从活的会话里
//!   取到的事实。拆开之后策略可单测、时序可集中审。
//! - 用低频轮询而不是事件驱动：需要的信号是"输出停歇了"，本质就是时间条件，
//!   再叠一层回调只会把同一个判断散成两处。
//! - 轮询任务在状态机进入终态后立即退出（生效或放弃都会落终态），不留常驻定时器；
//!   视图被回收时 `update` 返回 `Err`，循环同样退出。

use super::TerminalView;
use super::silent_echo::MARKER;
use crate::shell_integration::{
    IntegrationAction, IntegrationContext, IntegrationSignal, probe_command,
};
use gpui_kit::*;
use std::time::{Duration, Instant};
use transom_core::protocol::shell_integration::injection_payload;

/// 状态机检查周期。250ms 足够让用户在第一次回车之前就装好钩子，
/// 又不会在早已结束的轮次上形成有感知的空转。
const INTEGRATION_POLL: Duration = Duration::from_millis(250);

/// 提示符落位判定：输出至少停歇这么久才动手。
///
/// 比检索重扫的停歇窗口短，但不能短到把登录脚本的短暂停顿当成提示符：
/// 探针写进尚未就绪的 Shell 会被启动脚本吞掉。1 秒是延迟与命中率之间的
/// 折中；慢机器仍可能错过，但比 400ms 稳得多。
const OUTPUT_IDLE_WINDOW: Duration = Duration::from_secs(1);

impl TerminalView {
    /// 启动集成轮询任务；句柄必须由视图持有，避免脱缰任务。
    pub(super) fn spawn_integration_task(cx: &mut Context<Self>) -> Task<()> {
        cx.spawn(async move |this: WeakEntity<Self>, cx: &mut AsyncApp| {
            loop {
                // 先睡再查：任务在 `TerminalView` 构造期间就被 spawn，立即回调会
                // 撞上尚未初始化完的实体；先让登录画屏跑起来也更符合本意。
                cx.background_executor().timer(INTEGRATION_POLL).await;
                let keep_running = this
                    .update(cx, |view, cx| {
                        view.integration_step(cx);
                        !view.shell_integration.is_terminal()
                    })
                    .unwrap_or(false);
                if !keep_running {
                    return;
                }
            }
        })
    }

    /// 按设置重置集成流程；已在途或已定论的轮次不受影响。
    pub(crate) fn set_shell_integration_enabled(&mut self, enabled: bool) {
        self.shell_integration.apply_setting(enabled);
    }

    /// 一次状态机检查：采集观察量、推进状态、落地动作。
    fn integration_step(&mut self, cx: &mut Context<Self>) {
        if self.shell_integration.is_terminal() {
            return;
        }
        let now = Instant::now();
        // 引擎锁只在这个表达式里持有：取完标量立刻放，跨 await 持锁会挂住线程池。
        let (output_started, output_idle, alt_screen, shell_markers) = {
            let engine = self.state.engine.lock();
            (
                engine.output_started(),
                engine.output_idle(now, OUTPUT_IDLE_WINDOW),
                engine.is_alt_screen(),
                engine.shell_markers_seen(),
            )
        };
        let context = IntegrationContext {
            now,
            output_started,
            output_idle,
            user_typed: self.user_typed,
            alt_screen,
            exited: self.exited,
        };
        let action = self.shell_integration.step(&context, shell_markers);
        self.apply_integration_action(action, cx);
    }

    /// 消费一条集成回报信号（由 OSC 旁路扫描器翻译而来）。
    pub(crate) fn on_integration_signal(
        &mut self,
        signal: IntegrationSignal,
        cx: &mut Context<Self>,
    ) {
        if self.shell_integration.is_terminal() {
            return;
        }
        let action = self.shell_integration.on_signal(&signal, Instant::now());
        if let IntegrationAction::Inject(kind) = action {
            log::info!("探测到远端登录 Shell，注入集成脚本，方言: {}", kind.label());
        }
        self.apply_integration_action(action, cx);
    }

    /// 把状态机动作落到 PTY 上。
    fn apply_integration_action(&mut self, action: IntegrationAction, cx: &mut Context<Self>) {
        let command: Option<&'static str> = match action {
            IntegrationAction::Probe => {
                log::debug!("Shell 集成：发送登录 Shell 探针");
                Some(probe_command())
            }
            IntegrationAction::Inject(kind) => {
                crate::shell_integration::get_integration_script(kind)
            }
            IntegrationAction::None => None,
        };
        let Some(command) = command else {
            cx.notify();
            return;
        };
        // 载荷把命令与清行命令合成一条：命令是写进标准输入的，执行后它的回显会留在
        // 屏上，一整段几百字符的脚本留在那里极其扎眼。预装路径不走这里（见协议层说明）。
        // 终端宽度与当前提示符列数用于把折行后的回显一次擦干净。
        let (columns, prompt_columns) = {
            let engine = self.state.engine.lock();
            (
                usize::from(engine.size().cols),
                engine.cursor_point().column.0,
            )
        };
        // 开头加一个无副作用的 `true <标记>`：客户端按这个包装前缀摘掉整行回显，
        // 避免远端行编辑器在脚本中间插入重绘/ANSI 时精确匹配失效。行首空格保留，
        // 让 bash/zsh 不把注入命令写进历史。
        let wrapper = format!(" true {MARKER}");
        let marked = format!("{wrapper};{command}");
        let payload = injection_payload(&marked, columns, prompt_columns);
        // 回显过滤器按整行缓冲识别标记：只要这一行出现标记就整行摘除，
        // 不依赖命令前缀是否被 ANSI/重绘切断。
        self.state
            .silent_echo
            .lock()
            .arm(MARKER.as_bytes(), payload.cleanup_moves);
        // 走 write_input 但不置 user_typed：那是"用户亲自按键"的标记，
        // 自己写进去的脚本不能反过来把自己的时机判死。
        // 写失败说明通道已断（写入线程会随之上报 Exit），这里不额外造状态，
        // 下一轮检查自然按"会话已结束"落终态。
        if !self.write_input(&payload.bytes) {
            self.state.silent_echo.lock().disarm();
            log::warn!("Shell 集成写入通道已断开，本轮放弃");
        }
        cx.notify();
    }
}
