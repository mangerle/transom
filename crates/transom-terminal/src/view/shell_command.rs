//! 面向已有交互式 Shell 的目录切换命令构造。
//!
//! 设计原理（Why）：应用层只能往用户当前 Shell 输入，命令必须按方言生成；普通路径
//! 保持 `cd /path` 的自然形式，只有空格或 Shell 元字符会触发必要转义。统一放在这里，
//! 便于把 Bash、Zsh、Fish 与 PowerShell 的差异纯函数化测试，不触碰终端输入通道。

use crate::ShellKind;

/// 为指定 Shell 生成进入目录的单行命令；无法安全支持的方言返回 `None`。
pub(super) fn directory_change_command(kind: ShellKind, path: &str) -> Option<String> {
    match kind {
        ShellKind::Bash | ShellKind::Zsh | ShellKind::Fish => {
            Some(format!("cd {}", posix_path_argument(path)))
        }
        ShellKind::PowerShell => Some(format!("cd {}", power_shell_path_argument(path))),
        ShellKind::Other => None,
    }
}

/// 在无法确知 Shell 方言时按路径形式推断最安全通用的目录切换命令。
///
/// 设计原理（Why）：远端服务器（如原生 Linux/Unix）未启用 Shell 集成或探针超时时，
/// 不能直接拒绝跳转；若路径为典型 POSIX 路径，可安全按 POSIX 规则转义生成 `cd`。
pub(super) fn fallback_directory_change_command(path: &str) -> Option<String> {
    if path.starts_with('/') {
        Some(format!("cd {}", posix_path_argument(path)))
    } else if path.len() >= 2
        && path.chars().next().is_some_and(|c| c.is_ascii_alphabetic())
        && path.as_bytes().get(1) == Some(&b':')
    {
        Some(format!("cd {}", power_shell_path_argument(path)))
    } else {
        Some(format!("cd {}", posix_path_argument(path)))
    }
}

/// POSIX 路径只在会改变分词或触发展开时加单引号，保持普通路径可读。
fn posix_path_argument(path: &str) -> String {
    if path.starts_with('-') || path.chars().any(is_posix_shell_metachar) {
        quote_posix(path)
    } else {
        path.to_string()
    }
}

/// PowerShell 普通盘符路径不加引号；空格、变量符与通配符等才使用单引号。
fn power_shell_path_argument(path: &str) -> String {
    if path.starts_with('-') || path.chars().any(is_power_shell_metachar) {
        quote_power_shell(path)
    } else {
        path.to_string()
    }
}

/// 判断 POSIX Shell 中需要引用保护的字符；非 ASCII 文本可原样作为普通字面量。
fn is_posix_shell_metachar(ch: char) -> bool {
    ch.is_whitespace()
        || ch.is_control()
        || matches!(
            ch,
            '\'' | '"'
                | '\\'
                | '$'
                | '`'
                | '&'
                | ';'
                | '|'
                | '<'
                | '>'
                | '('
                | ')'
                | '*'
                | '?'
                | '['
                | ']'
                | '{'
                | '}'
                | '!'
                | '#'
                | '~'
        )
}

/// 判断 PowerShell 中需要引用保护的字符；反斜杠属于普通路径分隔符，不在此列。
fn is_power_shell_metachar(ch: char) -> bool {
    ch.is_whitespace()
        || ch.is_control()
        || matches!(
            ch,
            '\'' | '"'
                | '`'
                | '$'
                | '&'
                | ';'
                | '|'
                | '<'
                | '>'
                | '('
                | ')'
                | '*'
                | '?'
                | '['
                | ']'
                | '{'
                | '}'
                | '!'
                | '#'
                | '@'
                | ','
        )
}

/// 用 POSIX 单引号包裹路径，并把内部单引号拆成闭合、转义、重新开启三段。
fn quote_posix(path: &str) -> String {
    format!("'{}'", path.replace('\'', "'\\''"))
}

/// PowerShell 单引号字符串通过连续两个单引号表示一个字面单引号。
fn quote_power_shell(path: &str) -> String {
    format!("'{}'", path.replace('\'', "''"))
}

#[cfg(test)]
mod tests {
    use super::{
        directory_change_command, fallback_directory_change_command, posix_path_argument,
        power_shell_path_argument, quote_posix, quote_power_shell,
    };
    use crate::ShellKind;

    #[test]
    fn posix_quote_escapes_single_quotes() {
        assert_eq!(quote_posix("normal/path"), "'normal/path'");
        assert_eq!(quote_posix("a'b"), "'a'\\''b'");
    }

    #[test]
    fn powershell_quote_doubles_single_quotes() {
        assert_eq!(quote_power_shell("a'b"), "'a''b'");
    }

    #[test]
    fn ordinary_paths_keep_natural_cd_form() {
        for kind in [ShellKind::Bash, ShellKind::Zsh, ShellKind::Fish] {
            assert_eq!(
                directory_change_command(kind, "/home/wangle/Public"),
                Some("cd /home/wangle/Public".to_string())
            );
        }
        assert_eq!(
            directory_change_command(ShellKind::PowerShell, r"C:\Users\wangle\Public"),
            Some(r"cd C:\Users\wangle\Public".to_string())
        );
        assert_eq!(directory_change_command(ShellKind::Other, "/tmp"), None);
    }

    #[test]
    fn special_paths_are_quoted_only_when_required() {
        assert_eq!(posix_path_argument("/tmp/a b"), "'/tmp/a b'");
        assert_eq!(posix_path_argument("/tmp/a'b"), "'/tmp/a'\\''b'");
        assert_eq!(posix_path_argument("/tmp/普通目录"), "/tmp/普通目录");

        assert_eq!(power_shell_path_argument(r"C:\a b"), r"'C:\a b'");
        assert_eq!(
            power_shell_path_argument(r"C:\Users\wangle\Public"),
            r"C:\Users\wangle\Public"
        );
    }

    #[test]
    fn fallback_generates_command_by_path_format() {
        assert_eq!(
            fallback_directory_change_command("/home/absf/builds/6cxoLjC2"),
            Some("cd /home/absf/builds/6cxoLjC2".to_string())
        );
        assert_eq!(
            fallback_directory_change_command("/home/absf/my dir"),
            Some("cd '/home/absf/my dir'".to_string())
        );
        assert_eq!(
            fallback_directory_change_command(r"C:\Users\Public"),
            Some(r"cd C:\Users\Public".to_string())
        );
    }
}
