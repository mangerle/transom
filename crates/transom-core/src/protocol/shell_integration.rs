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
/// 字节；行首空格让 bash/zsh 在 `HISTCONTROL=ignorespace` 下不进历史；结尾用 LF
/// 提交整行——CR 只在 tty 的 ICRNL 打开时才被行规程转成行结束，否则整行会留在
/// 内核输入缓冲里被回显在命令行上，既不执行也污染用户的输入行。
pub fn probe_command() -> &'static [u8] {
    // 原始字节串不解析转义，所以换行要作为普通字符串段单独拼进来。
    concat!(r#" printf '\033]777;transom;shell=%s\007' "$SHELL""#, "\n").as_bytes()
}

/// Bash 集成脚本（单行）。
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
    r#"printf '\033]133;D;%d\007\033]133;A\007' "$__transom_ret"; }; "#,
    r#"PROMPT_COMMAND="__transom_post${PROMPT_COMMAND:+;$PROMPT_COMMAND}"; "#,
    r#"trap '__transom_pre' DEBUG; "#,
    r#"printf '\033]777;transom;ready=bash\007'"#,
    "; fi",
    "\n"
);

/// Zsh 集成脚本（单行）。`preexec`/`precmd` 是原生钩子，不需要 DEBUG 陷阱去重。
///
/// 行首空格与结尾 LF 的理由同 Bash 脚本：空格避免进入历史，LF 保证整行被真正提交。
pub const ZSH_INTEGRATION_SCRIPT: &str = concat!(
    r#" if [ -n "${ZSH_VERSION:-}" ] && [ -z "${TRANSOM_SI:-}" ]; then "#,
    r#"export TRANSOM_SI=1; "#,
    r#"__transom_pre() { printf '\033]133;C\007'; }; "#,
    r#"__transom_post() { local __transom_ret=$?; "#,
    r#"printf '\033]133;D;%d\007\033]133;A\007' "$__transom_ret"; }; "#,
    r#"autoload -Uz add-zsh-hook && add-zsh-hook precmd __transom_post "#,
    r#"&& add-zsh-hook preexec __transom_pre && "#,
    r#"printf '\033]777;transom;ready=zsh\007'"#,
    "; fi",
    "\n"
);

/// Fish 集成脚本（单行）。`fish_postexec` 事件里的 `$status` 即刚结束命令的退出码。
///
/// 行首空格与结尾 LF 的理由同 Bash 脚本：空格避免进入历史，LF 保证整行被真正提交。
pub const FISH_INTEGRATION_SCRIPT: &str = concat!(
    r#" if not set -q TRANSOM_SI; set -gx TRANSOM_SI 1; "#,
    r#"function __transom_pre --on-event fish_preexec; "#,
    r#"printf '\033]133;C\007'; end; "#,
    r#"function __transom_post --on-event fish_postexec; "#,
    r#"printf '\033]133;D;%d\007\033]133;A\007' $status; end; "#,
    r#"printf '\033]777;transom;ready=fish\007'"#,
    "; end",
    "\n"
);

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
        detect_shell, probe_command,
    };

    /// 每份脚本都必须：自带解释器判定、幂等保护、发 A/C/D 标记、回报 ready。
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
            assert!(script.contains("777;transom;"), "缺少 ready 回报: {script}");
            assert!(script.contains(ready));
            assert!(
                script.ends_with("\n"),
                "脚本必须自带换行，CR 在 ICRNL 关闭时不会结束整行: {script}"
            );
            assert_eq!(
                script.lines().count(),
                1,
                "多行脚本会被 Shell 当成多条命令逐行回显"
            );
        }
    }

    /// 探针同样必须以换行收尾，否则会留在内核输入缓冲里被回显。
    #[test]
    fn probe_command_ends_with_newline() {
        let probe = probe_command();
        let text = std::str::from_utf8(probe).expect("探针必须是合法 UTF-8");
        assert!(text.ends_with('\n'));
        assert_eq!(text.lines().count(), 1);
        assert!(text.contains("777;transom;shell=%s"));
        // 控制字节必须以 `\033` 形式交给 printf 解释，命令文本本身不含裸 ESC。
        assert!(!text.contains('\x1b'));
        assert!(!text.contains('\x07'));
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
