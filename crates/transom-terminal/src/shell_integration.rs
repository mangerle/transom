//! Shell Integration：探测远端 Shell 并注入 OSC 133 钩子，把提示符生命周期接进语义块。
//!
//! 设计原理（Why）：
//! - 实现初衷：OSC 133 的 `A/B/C/D` 标记必须由 Shell 自己发出，客户端不做任何事就
//!   永远拿不到命令边界。可选路线有三条：改远端 rc 文件（要先有写权限，且污染用户
//!   配置）、另开 exec 通道查询登录 shell（服务器审计里多出一条命令，exec 被禁用即
//!   失效）、向交互式 PTY 注入脚本（零依赖、零落盘）。本模块选第三条，并用下面两点
//!   把它的风险压到可接受：
//!   1. **探针自报**：先写一行 `printf` 让 Shell 通过 OSC 把 `$SHELL` 报回来。OSC
//!      序列被解析器消费、屏幕上不产生任何输出，因此探测本身零视觉污染；拿到确切
//!      Shell 类型后才注入对应方言，避免把 bash 语法喂给 fish 造成报错刷屏。
//!   2. **闭环取证**：注入脚本执行完毕后回报 `ready=<kind>`。没有回报就判定失败并
//!      停在终态，绝不把"我已经发过脚本"当成"集成已生效"。
//! - 核心优势：不写远端文件、不额外开通道、四种方言各自精确；失败静默降级，正常
//!   交互完全不受影响。
//! - 代价与局限：
//!   1. 探针与注入各会在屏幕上留下一行"被执行过的命令文本"（其输出为空）。这是
//!      交互式注入的固有代价，只发生一次；
//!   2. 注入必须落在提示符上。控制器只在"已有输出且输出停歇、用户尚未按键、不在
//!      备屏"时动手，且整轮只试一次；若用户在集成完成前就敲了键，本轮直接放弃；
//!   3. `$SHELL` 反映登录 shell，用户随后手切到别的 Shell（在 bash 里打 `fish`）时
//!      探测会失配；此时注入的方言带自我校验（`BASH_VERSION`/`ZSH_VERSION`），失配
//!      就静默不装也不谎报成功；
//!   4. 只支持 bash/zsh/fish。`sh`/`csh` 等无 DEBUG/precmd 等价钩子的方言不注入；
//!   5. bash 侧用 `PROMPT_COMMAND` 字符串拼接模拟 precmd，若用户把它用成 5.1+ 的数组
//!      形式，会被降级成标量（不报错，但数组元素丢失）。

use std::time::{Duration, Instant};

/// 支持自动集成的 Shell 类型。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ShellKind {
    Bash,
    Zsh,
    Fish,
    /// 未识别的方言：不注入，避免把 bash 语法喂给别的解释器。
    Other,
}

impl ShellKind {
    /// 用于日志与回报标记的稳定名称。
    pub fn label(self) -> &'static str {
        match self {
            ShellKind::Bash => "bash",
            ShellKind::Zsh => "zsh",
            ShellKind::Fish => "fish",
            ShellKind::Other => "other",
        }
    }

    /// 由登录 shell 路径推断方言。
    pub fn from_path(path: &str) -> Option<Self> {
        match detect_shell(path) {
            ShellKind::Other => None,
            kind => Some(kind),
        }
    }
}

/// 根据 Shell 可执行文件路径推断类型。
pub fn detect_shell(shell_path: &str) -> ShellKind {
    let lower = shell_path.to_ascii_lowercase();
    // 只看路径末段：`/usr/local/bin/fish` 里含 "fish"，而 `/bin/sh` 含 "sh" 太宽。
    let name = lower.rsplit('/').next().unwrap_or(lower.as_str());
    if name.contains("zsh") {
        ShellKind::Zsh
    } else if name.contains("fish") {
        ShellKind::Fish
    } else if name.contains("bash") {
        ShellKind::Bash
    } else {
        ShellKind::Other
    }
}

/// 探测用 OSC 载荷前缀：`OSC 777;transom;<键>=<值>`，与 `notify` 分支互不重叠。
pub const HANDSHAKE_TOKEN: &str = "transom";

/// 探针命令：让 Shell 把 `$SHELL` 通过 OSC 报回来。
///
/// 三个刻意的设计：`printf` 的 `\033`/`\007` 由格式化解释，命令文本里不含真实控制
/// 字节；行首空格让 bash/zsh 在 `HISTCONTROL=ignorespace` 下不进历史；`\r` 提交。
pub fn probe_command() -> &'static [u8] {
    // 原始字节串不解析转义，所以回车要作为普通字符串段单独拼进来。
    concat!(r#" printf '\033]777;transom;shell=%s\007' "$SHELL""#, "\r").as_bytes()
}

/// Bash 集成脚本（单行）。
///
/// 用 `TRANSOM_SI_PENDING` 给 `DEBUG` 陷阱去重：陷阱对同一条命令的每个简单命令都会
/// 触发，只有提示符落位后的第一次才代表"命令开始执行"。`$?` 必须在任何别的命令改写
/// 它之前取走，所以先 `local` 存住。末尾的 `777;transom;ready=` 回报是"确实装上了"的
/// 唯一凭据，各方言各自内联（`concat!` 只接受字面量，故不做公共常量）。
pub const BASH_INTEGRATION_SCRIPT: &str = concat!(
    r#"if [ -n "${BASH_VERSION:-}" ] && [ -z "${TRANSOM_SI:-}" ]; then "#,
    r#"export TRANSOM_SI=1; "#,
    r#"__transom_pre() { if [ -n "${TRANSOM_SI_PENDING:-}" ]; then TRANSOM_SI_PENDING=; "#,
    r#"printf '\033]133;C\007'; fi; }; "#,
    r#"__transom_post() { local __transom_ret=$?; TRANSOM_SI_PENDING=1; "#,
    r#"printf '\033]133;D;%d\007\033]133;A\007' "$__transom_ret"; }; "#,
    r#"PROMPT_COMMAND="__transom_post${PROMPT_COMMAND:+;$PROMPT_COMMAND}"; "#,
    r#"trap '__transom_pre' DEBUG; "#,
    r#"printf '\033]777;transom;ready=bash\007'"#,
    "; fi",
    "\r"
);

/// Zsh 集成脚本（单行）。`preexec`/`precmd` 是原生钩子，不需要 DEBUG 陷阱去重。
pub const ZSH_INTEGRATION_SCRIPT: &str = concat!(
    r#"if [ -n "${ZSH_VERSION:-}" ] && [ -z "${TRANSOM_SI:-}" ]; then "#,
    r#"export TRANSOM_SI=1; "#,
    r#"__transom_pre() { printf '\033]133;C\007'; }; "#,
    r#"__transom_post() { local __transom_ret=$?; "#,
    r#"printf '\033]133;D;%d\007\033]133;A\007' "$__transom_ret"; }; "#,
    r#"autoload -Uz add-zsh-hook && add-zsh-hook precmd __transom_post "#,
    r#"&& add-zsh-hook preexec __transom_pre && "#,
    r#"printf '\033]777;transom;ready=zsh\007'"#,
    "; fi",
    "\r"
);

/// Fish 集成脚本（单行）。`fish_postexec` 事件里的 `$status` 即刚结束命令的退出码。
pub const FISH_INTEGRATION_SCRIPT: &str = concat!(
    r#"if not set -q TRANSOM_SI; set -gx TRANSOM_SI 1; "#,
    r#"function __transom_pre --on-event fish_preexec; "#,
    r#"printf '\033]133;C\007'; end; "#,
    r#"function __transom_post --on-event fish_postexec; "#,
    r#"printf '\033]133;D;%d\007\033]133;A\007' $status; end; "#,
    r#"printf '\033]777;transom;ready=fish\007'"#,
    "; end",
    "\r"
);

/// 获取指定 Shell 的集成注入脚本；未识别方言返回 `None`。
pub fn get_integration_script(kind: ShellKind) -> Option<&'static [u8]> {
    match kind {
        ShellKind::Bash => Some(BASH_INTEGRATION_SCRIPT.as_bytes()),
        ShellKind::Zsh => Some(ZSH_INTEGRATION_SCRIPT.as_bytes()),
        ShellKind::Fish => Some(FISH_INTEGRATION_SCRIPT.as_bytes()),
        ShellKind::Other => None,
    }
}

/// 集成推进过程中的外部观察量，由视图每帧/每次定时采集。
///
/// 之所以由调用方注入而不是让状态机去读引擎：状态机要保持纯函数可测，同时让
/// "什么算安全时机"这条策略在视图层集中可见。
#[derive(Debug, Clone, Copy)]
pub struct IntegrationContext {
    pub now: Instant,
    /// 已经收到过远端输出：说明 Shell 至少开始画图了。
    pub output_started: bool,
    /// 输出已停歇一个窗口：提示符大概率已经落位。
    pub output_idle: bool,
    /// 用户已经亲自按键：此时写进去的东西会拼到用户半截的命令行上，必须放弃本轮。
    pub user_typed: bool,
    /// 处于备屏（vim/top/tmux 等）：探针会被当成程序输入，绝不能写。
    pub alt_screen: bool,
    /// 会话已退出。
    pub exited: bool,
}

/// 集成失败终态的原因，用于日志与状态提示。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IntegrationHalt {
    /// 设置里关闭了自动注入。
    Disabled,
    /// 探针无回应：登录 shell 未就绪、被程序抢走，或网络抖动超时。
    ProbeTimeout,
    /// 探针回应里 `$SHELL` 为空或无法解析。
    NoShellReport,
    /// 探测到的是未支持方言。
    UnsupportedShell,
    /// 远端已经在发 OSC 133 标记，无需重复注入。
    PreIntegrated,
    /// 脚本已注入但没有回报 ready。
    InjectTimeout,
    /// 会话结束。
    Exited,
}

/// Shell 集成回报的两类信号（由视图从 OSC 事件里翻译过来，避免本模块依赖 OSC 内部类型）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IntegrationSignal {
    /// 探针回应：远端上报登录 shell 路径。
    ShellReport { shell: String },
    /// 注入脚本自报安装完成。
    Ready { shell: String },
}

/// 状态机要求视图执行的一次动作。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IntegrationAction {
    /// 什么都不做，继续等下一次定时检查。
    None,
    /// 发送探针。
    Probe,
    /// 发送指定方言的注入脚本。
    Inject(ShellKind),
}

/// 等待首个安全时机的最长时间：超时后本轮整体放弃。
///
/// 登录慢（MOTD、LDAP、Kerberos）的机器上，提示符可能要十几秒才出现，给足余量；
/// 但绝不能无限等——一个永不落地的定时器是资源泄漏。
const WAIT_DEADLINE: Duration = Duration::from_secs(60);
/// 探针/注入回报的等待上限。一次 SSH 往返加 Shell 解析，5 秒不回即视为失败。
const REPLY_TIMEOUT: Duration = Duration::from_secs(5);

/// 一轮 Shell 集成的状态机。
///
/// 用 `enum` 而不是若干布尔标志位，是为了让"已注入但还没回报"这类中间态无法被
/// 误当成"已生效"：`Active` 只能由 `ready` 回报推进出来，编译器保证。
#[derive(Debug)]
pub struct ShellIntegration {
    phase: IntegrationPhase,
    started_at: Instant,
    /// 探针报回来的登录 shell 路径，失败时用于写清楚现场。
    reported_shell: Option<String>,
}

/// 当前处于哪一步。终态为 `Active` 与 `Unavailable`。
#[derive(Debug, Clone, Copy, PartialEq)]
enum IntegrationPhase {
    /// 等一个可以安全写入的时机。
    Waiting,
    /// 探针已发出，等 Shell 回报 `$SHELL`。
    Probing { at: Instant },
    /// 脚本已注入，等 ready 回报。
    Injecting { kind: ShellKind, at: Instant },
    /// 集成已生效（拿到 ready 证据）。
    Active(ShellKind),
    /// 本轮终止，不再尝试。
    Unavailable(IntegrationHalt),
}

impl ShellIntegration {
    /// 新建一轮集成；`enabled` 为 false 时直接落 `Disabled` 终态，不做任何写入。
    pub fn new(enabled: bool) -> Self {
        Self {
            phase: if enabled {
                IntegrationPhase::Waiting
            } else {
                IntegrationPhase::Unavailable(IntegrationHalt::Disabled)
            },
            started_at: Instant::now(),
            reported_shell: None,
        }
    }

    /// 集成是否已确认生效。
    pub fn is_active(&self) -> bool {
        matches!(self.phase, IntegrationPhase::Active(_))
    }

    /// 是否还停在"等一个安全时机"的初始阶段（探针与脚本都尚未写出）。
    pub fn is_waiting(&self) -> bool {
        matches!(self.phase, IntegrationPhase::Waiting)
    }

    /// 按设置重置流程：只在初始阶段生效，其余阶段一律不动。
    ///
    /// 探测在途或已经装上钩子时改设置不会重写脚本——半途重置只会留下半套钩子，
    /// 同一条命令出现两套 `A/C/D` 标记比不集成更糟。
    pub fn apply_setting(&mut self, enabled: bool) {
        if self.is_waiting() {
            self.phase = if enabled {
                IntegrationPhase::Waiting
            } else {
                IntegrationPhase::Unavailable(IntegrationHalt::Disabled)
            };
        }
    }

    /// 是否已进入终态（视图据此停掉定时器）。
    pub fn is_terminal(&self) -> bool {
        matches!(
            self.phase,
            IntegrationPhase::Active(_) | IntegrationPhase::Unavailable(_)
        )
    }

    /// 终态原因；非终态返回 `None`。
    pub fn halt(&self) -> Option<IntegrationHalt> {
        match self.phase {
            IntegrationPhase::Unavailable(reason) => Some(reason),
            _ => None,
        }
    }

    /// 生效的方言；未生效返回 `None`。
    pub fn shell_kind(&self) -> Option<ShellKind> {
        match self.phase {
            IntegrationPhase::Active(kind) => Some(kind),
            _ => None,
        }
    }

    /// 定时检查点：按外部观察量决定下一步动作。
    ///
    /// `shell_markers` 表示引擎已经收到过 OSC 133 标记（远端自带集成），此时不再注入。
    pub fn step(&mut self, ctx: &IntegrationContext, shell_markers: bool) -> IntegrationAction {
        if ctx.exited {
            self.stop(IntegrationHalt::Exited);
            return IntegrationAction::None;
        }
        match self.phase {
            IntegrationPhase::Waiting => {
                // 截止时间先判：否则"用户一直在打字"会让本轮永远达不到检查点，
                // 留下一个常驻定时器。
                if ctx.now.saturating_duration_since(self.started_at) >= WAIT_DEADLINE {
                    self.stop(IntegrationHalt::ProbeTimeout);
                    return IntegrationAction::None;
                }
                if shell_markers {
                    self.stop(IntegrationHalt::PreIntegrated);
                    return IntegrationAction::None;
                }
                if !ctx.output_started || ctx.alt_screen || ctx.user_typed || !ctx.output_idle {
                    return IntegrationAction::None;
                }
                self.phase = IntegrationPhase::Probing { at: ctx.now };
                IntegrationAction::Probe
            }
            IntegrationPhase::Probing { at } => {
                if ctx.now.saturating_duration_since(at) >= REPLY_TIMEOUT {
                    self.stop(IntegrationHalt::ProbeTimeout);
                }
                IntegrationAction::None
            }
            IntegrationPhase::Injecting { at, .. } => {
                if ctx.now.saturating_duration_since(at) >= REPLY_TIMEOUT {
                    self.stop(IntegrationHalt::InjectTimeout);
                }
                IntegrationAction::None
            }
            IntegrationPhase::Active(_) | IntegrationPhase::Unavailable(_) => {
                IntegrationAction::None
            }
        }
    }

    /// 消费一条来自远端的回报信号。
    ///
    /// 返回值只在"探针回应即刻可注入"时携带脚本，其余情况由下一次定时检查驱动，
    /// 保持视图侧只有一条发送出口。
    pub fn on_signal(&mut self, signal: &IntegrationSignal, now: Instant) -> IntegrationAction {
        match (signal, self.phase) {
            (IntegrationSignal::ShellReport { shell }, IntegrationPhase::Probing { .. }) => {
                self.reported_shell = Some(shell.clone());
                let kind = match ShellKind::from_path(shell) {
                    Some(kind) => kind,
                    None if shell.trim().is_empty() => {
                        self.stop(IntegrationHalt::NoShellReport);
                        return IntegrationAction::None;
                    }
                    None => {
                        self.stop(IntegrationHalt::UnsupportedShell);
                        return IntegrationAction::None;
                    }
                };
                self.phase = IntegrationPhase::Injecting { kind, at: now };
                IntegrationAction::Inject(kind)
            }
            (IntegrationSignal::Ready { shell }, IntegrationPhase::Injecting { kind, .. }) => {
                // 回报里的方言名只用于核对现场；推进以"拿到 ready"为准，因为脚本自带
                // 解释器判定，能打印出 ready 就说明确实装上了。
                log::info!("Shell 集成注入生效，方言: {shell}");
                self.phase = IntegrationPhase::Active(kind);
                IntegrationAction::None
            }
            (IntegrationSignal::Ready { shell }, IntegrationPhase::Waiting) => {
                // 远端 rc 里已经装了本方案同款脚本：直接认定生效，不再重复注入。
                log::info!("检测到既有 Shell 集成，跳过注入，方言: {shell}");
                let kind = ShellKind::from_path(shell).unwrap_or(ShellKind::Other);
                self.phase = IntegrationPhase::Active(kind);
                IntegrationAction::None
            }
            _ => IntegrationAction::None,
        }
    }

    fn stop(&mut self, reason: IntegrationHalt) {
        let shell = self
            .reported_shell
            .as_deref()
            .map(|shell| format!("，上报 shell: {shell}"))
            .unwrap_or_default();
        match reason {
            IntegrationHalt::Disabled => log::debug!("Shell 集成按设置关闭"),
            IntegrationHalt::Exited => log::debug!("会话已结束，Shell 集成终止"),
            IntegrationHalt::PreIntegrated => {
                log::info!("远端已在输出 OSC 133 标记，跳过注入，改用既有集成")
            }
            IntegrationHalt::UnsupportedShell => {
                log::warn!("登录 shell 方言不在支持列表（bash/zsh/fish），跳过注入{shell}")
            }
            IntegrationHalt::NoShellReport => {
                log::warn!("Shell 探针回报的 SHELL 为空，跳过注入")
            }
            IntegrationHalt::ProbeTimeout => {
                log::warn!("Shell 探针未在预期窗口内回报，已放弃本轮自动注入{shell}")
            }
            IntegrationHalt::InjectTimeout => {
                log::warn!("注入脚本已发送但未收到 ready 回报{shell}")
            }
        }
        self.phase = IntegrationPhase::Unavailable(reason);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ctx_at(now: Instant) -> IntegrationContext {
        IntegrationContext {
            now,
            output_started: true,
            output_idle: true,
            user_typed: false,
            alt_screen: false,
            exited: false,
        }
    }

    /// 推进到"探针已发出"的状态，供后续断言复用。
    fn probing(integration: &mut ShellIntegration) {
        integration.step(&ctx_at(Instant::now()), false);
    }

    #[test]
    fn test_detect_shell() {
        assert_eq!(detect_shell("/bin/bash"), ShellKind::Bash);
        assert_eq!(detect_shell("/usr/bin/zsh"), ShellKind::Zsh);
        assert_eq!(detect_shell("/usr/local/bin/fish"), ShellKind::Fish);
        assert_eq!(detect_shell("/bin/sh"), ShellKind::Other);
        assert_eq!(detect_shell("/usr/bin/BASH"), ShellKind::Bash);
        // 只看末段：目录名里带 bash 不能误判成 bash。
        assert_eq!(detect_shell("/opt/bash-tools/run"), ShellKind::Other);
    }

    #[test]
    fn test_disabled_never_acts() {
        let mut integration = ShellIntegration::new(false);
        assert_eq!(integration.halt(), Some(IntegrationHalt::Disabled));
        assert_eq!(
            integration.step(&ctx_at(Instant::now()), false),
            IntegrationAction::None
        );
        assert!(integration.is_terminal());
    }

    #[test]
    fn test_waits_for_safe_moment_then_probes() {
        let now = Instant::now();
        let mut integration = ShellIntegration::new(true);
        // 输出还没开始：不动。
        let mut ctx = ctx_at(now);
        ctx.output_started = false;
        assert_eq!(integration.step(&ctx, false), IntegrationAction::None);
        // 用户在打字：写进去会拼到半截命令行上，不动。
        let mut ctx = ctx_at(now);
        ctx.user_typed = true;
        assert_eq!(integration.step(&ctx, false), IntegrationAction::None);
        // 备屏程序里：探针会被当成程序输入，不动。
        let mut ctx = ctx_at(now);
        ctx.alt_screen = true;
        assert_eq!(integration.step(&ctx, false), IntegrationAction::None);
        // 输出未停歇：提示符大概率还没落位，不动。
        let mut ctx = ctx_at(now);
        ctx.output_idle = false;
        assert_eq!(integration.step(&ctx, false), IntegrationAction::None);
        // 全部满足：发探针。
        assert_eq!(
            integration.step(&ctx_at(now), false),
            IntegrationAction::Probe
        );
        // 已发出后不重复发。
        assert_eq!(
            integration.step(&ctx_at(now), false),
            IntegrationAction::None
        );
    }

    #[test]
    fn test_existing_markers_skip_injection() {
        let mut integration = ShellIntegration::new(true);
        integration.step(&ctx_at(Instant::now()), true);
        assert_eq!(integration.halt(), Some(IntegrationHalt::PreIntegrated));
    }

    #[test]
    fn test_probe_then_inject_then_active() {
        let now = Instant::now();
        let mut integration = ShellIntegration::new(true);
        assert_eq!(
            integration.step(&ctx_at(now), false),
            IntegrationAction::Probe
        );
        assert_eq!(
            integration.on_signal(
                &IntegrationSignal::ShellReport {
                    shell: "/usr/bin/zsh".into()
                },
                now
            ),
            IntegrationAction::Inject(ShellKind::Zsh)
        );
        assert_eq!(
            get_integration_script(ShellKind::Zsh),
            Some(ZSH_INTEGRATION_SCRIPT.as_bytes())
        );
        assert_eq!(
            integration.on_signal(
                &IntegrationSignal::Ready {
                    shell: "zsh".into()
                },
                now
            ),
            IntegrationAction::None
        );
        assert!(integration.is_active());
        assert_eq!(integration.shell_kind(), Some(ShellKind::Zsh));
        assert!(integration.is_terminal());
    }

    #[test]
    fn test_unsupported_shell_halts_without_injecting() {
        let now = Instant::now();
        let mut integration = ShellIntegration::new(true);
        probing(&mut integration);
        assert_eq!(
            integration.on_signal(
                &IntegrationSignal::ShellReport {
                    shell: "/bin/csh".into()
                },
                now
            ),
            IntegrationAction::None
        );
        assert_eq!(integration.halt(), Some(IntegrationHalt::UnsupportedShell));
        assert!(get_integration_script(ShellKind::Other).is_none());
    }

    #[test]
    fn test_empty_shell_report_halts() {
        let now = Instant::now();
        let mut integration = ShellIntegration::new(true);
        probing(&mut integration);
        integration.on_signal(
            &IntegrationSignal::ShellReport {
                shell: "  ".to_string(),
            },
            now,
        );
        assert_eq!(integration.halt(), Some(IntegrationHalt::NoShellReport));
    }

    #[test]
    fn test_probe_timeout_is_honored() {
        let now = Instant::now();
        let mut integration = ShellIntegration::new(true);
        integration.step(&ctx_at(now), false);
        // 探针发出但未超时：继续等，且不会二次发送。
        assert_eq!(
            integration.step(&ctx_at(now + Duration::from_secs(1)), false),
            IntegrationAction::None
        );
        integration.step(&ctx_at(now + REPLY_TIMEOUT), false);
        assert_eq!(integration.halt(), Some(IntegrationHalt::ProbeTimeout));
    }

    #[test]
    fn test_wait_deadline_gives_up_before_probing() {
        let now = Instant::now();
        let mut integration = ShellIntegration::new(true);
        integration.started_at = now;
        integration.phase = IntegrationPhase::Waiting;
        let mut ctx = ctx_at(now + WAIT_DEADLINE);
        // 提示符迟迟不出现（输出一直不停歇），到期后放弃，不留常驻定时器。
        ctx.output_idle = false;
        assert_eq!(integration.step(&ctx, false), IntegrationAction::None);
        assert_eq!(integration.halt(), Some(IntegrationHalt::ProbeTimeout));
    }

    #[test]
    fn test_inject_without_ready_marks_failure() {
        let now = Instant::now();
        let mut integration = ShellIntegration::new(true);
        integration.step(&ctx_at(now), false);
        integration.on_signal(
            &IntegrationSignal::ShellReport {
                shell: "/bin/bash".into(),
            },
            now,
        );
        assert!(!integration.is_active(), "注入后未回报不得声称生效");
        integration.step(
            &ctx_at(now + REPLY_TIMEOUT + Duration::from_millis(1)),
            false,
        );
        assert_eq!(integration.halt(), Some(IntegrationHalt::InjectTimeout));
    }

    #[test]
    fn test_ready_while_waiting_reuses_existing_integration() {
        let mut integration = ShellIntegration::new(true);
        integration.on_signal(
            &IntegrationSignal::Ready {
                shell: "fish".into(),
            },
            Instant::now(),
        );
        assert!(integration.is_active());
        assert_eq!(integration.shell_kind(), Some(ShellKind::Fish));
    }

    #[test]
    fn test_out_of_order_signals_are_ignored() {
        // 尚未发探针就收到 shell 回报、或已终态后收到 ready，都不改变状态。
        let now = Instant::now();
        let mut integration = ShellIntegration::new(true);
        assert_eq!(
            integration.on_signal(
                &IntegrationSignal::ShellReport {
                    shell: "/bin/bash".into()
                },
                now
            ),
            IntegrationAction::None
        );
        assert_eq!(integration.phase, IntegrationPhase::Waiting);
        integration.stop(IntegrationHalt::Disabled);
        assert_eq!(
            integration.on_signal(
                &IntegrationSignal::Ready {
                    shell: "zsh".into()
                },
                now
            ),
            IntegrationAction::None
        );
        assert_eq!(integration.halt(), Some(IntegrationHalt::Disabled));
    }

    #[test]
    fn test_exit_halts_any_phase() {
        let now = Instant::now();
        let mut integration = ShellIntegration::new(true);
        integration.step(&ctx_at(now), false);
        let mut ctx = ctx_at(now);
        ctx.exited = true;
        integration.step(&ctx, false);
        assert_eq!(integration.halt(), Some(IntegrationHalt::Exited));
    }

    #[test]
    fn test_scripts_self_identify_and_report_ready() {
        // 每份脚本都必须：自带解释器判定、幂等保护、发 A/C/D 标记、回报 ready。
        for (script, ready) in [
            (BASH_INTEGRATION_SCRIPT, "ready=bash"),
            (ZSH_INTEGRATION_SCRIPT, "ready=zsh"),
            (FISH_INTEGRATION_SCRIPT, "ready=fish"),
        ] {
            assert!(script.contains("133;A"), "缺少提示符标记: {script}");
            assert!(script.contains("133;C"), "缺少执行标记: {script}");
            assert!(script.contains("133;D"), "缺少结束标记: {script}");
            assert!(script.contains("777;transom;"), "缺少 ready 回报: {script}");
            assert!(script.contains(ready));
            assert!(script.ends_with("\r"), "脚本必须自带回车: {script}");
            assert_eq!(
                script.lines().count(),
                1,
                "多行脚本会被 Shell 当成多条命令逐行回显"
            );
        }
        assert!(BASH_INTEGRATION_SCRIPT.contains("BASH_VERSION"));
        assert!(ZSH_INTEGRATION_SCRIPT.contains("ZSH_VERSION"));
        assert!(BASH_INTEGRATION_SCRIPT.contains("TRANSOM_SI"));
    }

    #[test]
    fn test_probe_command_shape() {
        let probe = probe_command();
        let text = std::str::from_utf8(probe).expect("探针必须是合法 UTF-8");
        assert!(text.ends_with('\r'));
        assert_eq!(text.lines().count(), 1);
        assert!(text.contains("777;transom;shell=%s"));
        // 控制字节必须以 `\033` 形式交给 printf 解释，命令文本本身不含裸 ESC。
        assert!(!text.contains('\x1b'));
        assert!(!text.contains('\x07'));
    }
}
