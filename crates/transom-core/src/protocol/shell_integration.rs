//! Shell 集成脚本与方言识别：供终端侧按需注入，也供本地 Shell 启动时预装。
//!
//! 设计原理（Why）：
//! - 实现初衷：同一份集成脚本要被两处用到——远程会话由终端侧探测到方言后注入，
//!   本地会话改由启动参数预装以做到零可见。两处各留一份必然漂移，
//!   所以脚本与方言识别作为"对外的线格式"统一放在协议层；
//! - 核心优势：预装路径完全不经过终端输入，屏幕上不会出现任何注入痕迹；
//! - 代价与局限：预装只能覆盖启动参数能表达的方言（bash/zsh），fish 等方言
//!   仍需终端侧注入，启动瞬间仍会有一行一闪。

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

    /// 由登录 shell 路径推断方言；未识别时返回 `None`。
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
/// 字节；行首空格让 bash/zsh 在 `HISTCONTROL=ignorespace` 下不进历史；不带行结束符——
/// 注入路径由 `injection_payload` 统一补上换行与清行命令，预装路径由调用方按
/// rcfile 的逐行格式补换行。
pub fn probe_command() -> &'static str {
    r#" printf '\033]777;transom;shell=%s\007' "$SHELL""#
}

/// Bash 集成脚本（单行）。
///
/// `__transom_post` 在每条命令结束后依次发 133;D/A 与 OSC 7 cwd：前者划分命令块，
/// 后者是 SFTP 跟随当前目录的标准通道。`$PWD` 直接拼进 `file://`，目录名里的
/// 百分号等字符可能未编码，终端侧按宽容策略解析。
///
/// 用 `TRANSOM_SI_PENDING` 给 `DEBUG` 陷阱去重：陷阱对同一条命令的每个简单命令都会
/// 触发，只有提示符落位后的第一次才代表"命令开始执行"。`$?` 必须在任何别的命令改写
/// 它之前取走，所以先 `local` 存住。末尾的 `777;transom;ready=` 回报是"确实装上了"的
/// 唯一凭据，各方言各自内联（`concat!` 只接受字面量，故不做公共常量）。
///
/// 行首留一个空格：`HISTCONTROL=ignorespace` 的 bash 不把以空格开头的命令写进历史。
/// 结尾用 LF 而不是 CR 结束整行：CR 只在 tty 的 ICRNL 打开时才被行规程转成行结束，
/// 否则整段脚本会留在内核输入缓冲里被回显在命令行上，既不执行也污染用户的输入行。
pub const BASH_INTEGRATION_SCRIPT: &str = concat!(
    r#" if [ -n "${BASH_VERSION:-}" ] && [ -z "${TRANSOM_SI:-}" ]; then "#,
    r#"export TRANSOM_SI=1; "#,
    r#"__transom_pre() { if [ -n "${TRANSOM_SI_PENDING:-}" ]; then TRANSOM_SI_PENDING=; "#,
    r#"printf '\033]133;C\007'; fi; }; "#,
    r#"__transom_post() { local __transom_ret=$?; TRANSOM_SI_PENDING=1; "#,
    r#"printf '\033]133;D;%d\007\033]133;A\007' "$__transom_ret"; "#,
    r#"printf '\033]7;file://%s\007' "$PWD"; }; "#,
    r#"PROMPT_COMMAND="__transom_post${PROMPT_COMMAND:+;$PROMPT_COMMAND}"; "#,
    r#"trap '__transom_pre' DEBUG; "#,
    r#"printf '\033]777;transom;ready=bash\007'"#,
    "; fi"
);

/// Zsh 集成脚本（单行）。`preexec`/`precmd` 是原生钩子，不需要 DEBUG 陷阱去重。
///
/// 行首空格与结尾 LF 的理由同 Bash 脚本：空格避免进入历史，LF 保证整行被真正提交。
pub const ZSH_INTEGRATION_SCRIPT: &str = concat!(
    r#" if [ -n "${ZSH_VERSION:-}" ] && [ -z "${TRANSOM_SI:-}" ]; then "#,
    r#"export TRANSOM_SI=1; "#,
    r#"__transom_pre() { printf '\033]133;C\007'; }; "#,
    r#"__transom_post() { local __transom_ret=$?; "#,
    r#"printf '\033]133;D;%d\007\033]133;A\007' "$__transom_ret"; "#,
    r#"printf '\033]7;file://%s\007' "$PWD"; }; "#,
    r#"autoload -Uz add-zsh-hook && add-zsh-hook precmd __transom_post "#,
    r#"&& add-zsh-hook preexec __transom_pre && "#,
    r#"printf '\033]777;transom;ready=zsh\007'"#,
    "; fi"
);

/// Fish 集成脚本（单行）。`fish_postexec` 事件里的 `$status` 即刚结束命令的退出码；
/// cwd 改在 `fish_prompt` 事件上报，保证第一条提示符也有目录（postexec 只在命令后触发）。
///
/// 行首空格与结尾 LF 的理由同 Bash 脚本：空格避免进入历史，LF 保证整行被真正提交。
pub const FISH_INTEGRATION_SCRIPT: &str = concat!(
    r#" if not set -q TRANSOM_SI; set -gx TRANSOM_SI 1; "#,
    r#"function __transom_pre --on-event fish_preexec; "#,
    r#"printf '\033]133;C\007'; end; "#,
    r#"function __transom_prompt --on-event fish_prompt; "#,
    r#"printf '\033]7;file://%s\007' $PWD; end; "#,
    r#"function __transom_post --on-event fish_postexec; "#,
    r#"printf '\033]133;D;%d\007\033]133;A\007' $status; end; "#,
    r#"printf '\033]777;transom;ready=fish\007'"#,
    "; end"
);

/// 注入命令末尾追加的清行命令：执行后擦掉这条命令自己的回显。
///
/// 设计原理（Why）：
/// - 命令是写进 Shell 标准输入的，Shell 会把执行过的命令留在屏幕上。注入的是一整段
///   几百字符的脚本，留在屏上极其扎眼；执行完顺手擦掉自己的回显，屏幕上就只剩提示符；
/// - 必须是 Shell 命令而不是裸控制字节：裸字节接在命令后面会被 Shell 当成命令的
///   一部分（转义序列不是合法命令名），整条命令会直接语法错误；
/// - 必须与被注入的命令同处一条：单独再发一条清行命令，它自己的回显同样会留在
///   屏上，只能再擦再擦；命令内部执行 printf 则只擦掉整条命令的回显；
/// - 脚本在窄终端里会折成多行，只上移一行只能擦掉最后一行，前面几行仍会留在屏上。
///   因此上移次数按"提示符列数 + 命令长度"与终端列宽计算，一次把整段回显全部擦掉。
///
/// 只在"写标准输入"这条注入路径上追加：预装走 rcfile，脚本在那里执行时上一行是
/// 用户自己的输出，擦掉会误伤。已滚入 scrollback 的历史同样擦不掉。
fn injection_cleanup(lines: usize) -> String {
    let mut cleanup = String::with_capacity(12 + lines * 8);
    cleanup.push_str("; printf '");
    for _ in 0..lines {
        cleanup.push_str("\\033[1A\\033[2K");
    }
    cleanup.push('\'');
    cleanup
}

/// 一次注入载荷：字节本体与清行命令需要上移的行数。
#[derive(Debug, Clone)]
pub struct InjectionPayload {
    pub bytes: Vec<u8>,
    /// 命令回显在终端里占用的视觉行数；客户端摘掉回显后据此丢弃清行输出。
    pub cleanup_moves: usize,
}

/// 生成注入用的命令字节：命令本体 + 末尾的清行命令 + 换行。
///
/// `columns` 是终端列宽，`prompt_columns` 是注入时提示符占用的列数（光标所在列）。
/// 两者用于把折行后的命令回显一次擦干净；值不可信或过小时按 20 列兜底，宁可多擦
/// 一行也不把脚本留在屏幕上。
///
/// 换行用 LF 而不是 CR：CR 只在 tty 的 ICRNL 打开时才会被行规程转成行结束，
/// 一旦 ICRNL 被关，CR 不会结束整行，命令会留在内核输入缓冲里被回显在命令行上，
/// 既不执行也污染用户的输入行。LF 两种模式下都能真正提交这一行。
pub fn injection_payload(command: &str, columns: usize, prompt_columns: usize) -> InjectionPayload {
    let columns = columns.max(20);
    let prefix = prompt_columns.saturating_add(command.chars().count());
    // 清行命令本身也会被回显，长度会随行数变化；迭代到行数稳定，避免算少一行。
    let mut lines = 1;
    loop {
        let cleanup = injection_cleanup(lines);
        let total = prefix.saturating_add(cleanup.chars().count());
        let next = total.div_ceil(columns).max(1);
        if next == lines {
            let mut bytes = Vec::with_capacity(command.len() + cleanup.len() + 1);
            bytes.extend_from_slice(command.as_bytes());
            bytes.extend_from_slice(cleanup.as_bytes());
            bytes.push(b'\n');
            return InjectionPayload {
                bytes,
                cleanup_moves: lines,
            };
        }
        lines = next;
    }
}

/// 取指定方言的集成脚本；`Other` 没有等价脚本，返回 `None`。
pub fn integration_script(kind: ShellKind) -> Option<&'static str> {
    match kind {
        ShellKind::Bash => Some(BASH_INTEGRATION_SCRIPT),
        ShellKind::Zsh => Some(ZSH_INTEGRATION_SCRIPT),
        ShellKind::Fish => Some(FISH_INTEGRATION_SCRIPT),
        ShellKind::Other => None,
    }
}

#[cfg(test)]
mod tests {
    use super::{
        BASH_INTEGRATION_SCRIPT, FISH_INTEGRATION_SCRIPT, ShellKind, ZSH_INTEGRATION_SCRIPT,
        detect_shell, injection_payload, probe_command,
    };

    /// 每份脚本都必须：自带解释器判定、幂等保护、发 A/C/D 标记、上报 cwd、回报 ready。
    #[test]
    fn integration_scripts_carry_required_parts() {
        for (script, ready) in [
            (BASH_INTEGRATION_SCRIPT, "ready=bash"),
            (ZSH_INTEGRATION_SCRIPT, "ready=zsh"),
            (FISH_INTEGRATION_SCRIPT, "ready=fish"),
        ] {
            assert!(script.contains("133;A"), "缺少提示符标记: {script}");
            assert!(script.contains("133;C"), "缺少执行标记: {script}");
            assert!(script.contains("133;D"), "缺少结束标记: {script}");
            assert!(
                script.contains("7;file://"),
                "缺少 OSC 7 cwd 上报: {script}"
            );
            assert!(script.contains("$PWD"), "cwd 必须取当前目录: {script}");
            assert!(script.contains("777;transom;"), "缺少 ready 回报: {script}");
            assert!(script.contains(ready));
            assert!(
                !script.ends_with('\n'),
                "脚本本体不带换行：注入路径要把它与清行命令拼成一条，预装路径按行写入: {script}"
            );
            assert_eq!(
                script.lines().count(),
                1,
                "多行脚本会被 Shell 当成多条命令逐行回显"
            );
        }
    }

    /// 探针本身不带换行，行结束由注入载荷统一补。
    #[test]
    fn probe_command_carries_no_line_ending() {
        let text = probe_command();
        assert!(!text.ends_with('\n'));
        assert_eq!(text.lines().count(), 1);
        assert!(text.contains("777;transom;shell=%s"));
        // 控制字节必须以 `\033` 形式交给 printf 解释，命令文本本身不含裸 ESC。
        assert!(!text.contains('\x1b'));
        assert!(!text.contains('\x07'));
    }

    /// 注入载荷：命令与清行命令合成一条，并以换行收尾。
    ///
    /// 清行必须与命令同处一条：单独再发一条清行命令，它自己的回显同样会留在屏上。
    #[test]
    fn injection_payload_appends_cleanup_to_same_command() {
        let payload = injection_payload(probe_command(), 80, 20);
        let text = std::str::from_utf8(&payload.bytes).expect("注入载荷必须是合法 UTF-8");
        assert!(text.ends_with('\n'), "载荷必须以换行收尾才能被 Shell 提交");
        assert!(
            text.contains("\\033[1A\\033[2K"),
            "清行命令必须与被注入命令同处一条"
        );
        assert_eq!(text.lines().count(), 1, "清行命令不能另起一行");
        assert!(payload.cleanup_moves >= 1);
    }

    /// 窄终端里脚本会折成多行，清行命令必须按实际行数上移，不能只擦最后一行。
    #[test]
    fn injection_payload_clears_every_wrapped_line() {
        let long_command = "x".repeat(600);
        let payload = injection_payload(&long_command, 80, 20);
        let text = std::str::from_utf8(&payload.bytes).expect("注入载荷必须是合法 UTF-8");
        let moves = text.matches("\\033[1A\\033[2K").count();
        assert!(
            moves >= 7,
            "600 字符在 80 列里至少折成 8 行，实际只清 {moves} 行: {text}"
        );
        assert_eq!(payload.cleanup_moves, moves);
        assert_eq!(text.lines().count(), 1, "清行命令不能另起一行");
    }

    /// 方言识别只看路径末段，`/bin/sh` 不能被误判成 bash。
    #[test]
    fn shell_detection_uses_basename_only() {
        assert_eq!(detect_shell("/bin/bash"), ShellKind::Bash);
        assert_eq!(detect_shell("/usr/local/bin/fish"), ShellKind::Fish);
        assert_eq!(detect_shell("/bin/sh"), ShellKind::Other);
        assert_eq!(detect_shell("/usr/bin/zsh"), ShellKind::Zsh);
    }
}
