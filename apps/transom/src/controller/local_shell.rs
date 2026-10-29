//! 本地 Shell 的集成预装：用启动参数把集成脚本装进 Shell，屏幕上不留任何痕迹。
//!
//! 设计原理（Why）：
//! - 实现初衷：把脚本写进终端 stdin 会被 readline 回显在命令行上；本地 Shell 的
//!   启动参数由我们决定，直接在 rcfile 里把钩子装好，就绕开了终端输入这条路；
//! - 核心优势：预装完全不经过终端输入，屏幕上不会出现注入痕迹，也没有一闪而过；
//! - 代价与局限：只有启动参数能表达的方言支持预装（bash 的 `--rcfile`、zsh 的
//!   `ZDOTDIR`），fish 等方言没有等价手段，只能回退到终端侧注入，启动瞬间仍会
//!   有一行一闪；预装只改当前会话的启动文件读取位置，用户原有配置由临时 rcfile
//!   显式 source 回来，行为与不预装时等价。

use std::fs;
use std::path::PathBuf;

use transom_core::SessionCleanup;
use transom_core::protocol::shell_integration::{ShellKind, detect_shell, integration_script};

/// 预装产物：装好的方言、附加启动参数、环境变量与清理守卫。
pub(super) struct LocalShellPreload {
    /// 预装成功的方言；`None` 表示该方言无法用启动参数预装，调用方应回退终端侧注入。
    pub(super) kind: Option<ShellKind>,
    /// 追加到 Shell 命令行后的参数。
    pub(super) args: Vec<String>,
    /// 追加到子进程的环境变量。
    pub(super) env: Vec<(String, String)>,
    /// 临时文件清理守卫；`None` 表示没有产生临时文件。
    pub(super) cleanup: Option<SessionCleanup>,
}

impl LocalShellPreload {
    /// 不预装：调用方回退到终端侧注入。
    pub(super) fn none() -> Self {
        Self {
            kind: None,
            args: Vec::new(),
            env: Vec::new(),
            cleanup: None,
        }
    }
}

/// 按方言准备预装产物；任何一步失败都回退到终端侧注入，不影响会话建立。
pub(super) fn prepare(shell_path: &str) -> LocalShellPreload {
    let attempt = match detect_shell(shell_path) {
        ShellKind::Bash => prepare_bash(),
        ShellKind::Zsh => prepare_zsh(),
        // fish 没有等价的启动参数注入手段；它的注入只能写 stdin，会闪一行。
        _ => return LocalShellPreload::none(),
    };
    match attempt {
        Ok(preload) => preload,
        Err(error) => {
            log::warn!("本地 Shell 集成预装失败，回退到终端侧注入: {error}");
            LocalShellPreload::none()
        }
    }
}

/// bash：用 `--rcfile` 指向临时启动文件，文件内先 source 用户原有 rc 再装钩子。
fn prepare_bash() -> Result<LocalShellPreload, String> {
    let script = integration_script(ShellKind::Bash).ok_or("缺少 bash 集成脚本")?;
    let dir = create_temp_dir("transom-bash")?;
    let rcfile = dir.join("bashrc");
    write_file(
        &rcfile,
        &format!(
            "# 由 Transom 生成的临时 rcfile：先加载用户原有配置，再装 Shell 集成钩子。\n{}{script}\n",
            source_user_file(".bashrc")
        ),
    )?;
    let rcfile_path = rcfile.to_string_lossy().into_owned();
    Ok(LocalShellPreload {
        kind: Some(ShellKind::Bash),
        // `-i` 显式要求交互式：没有它 bash 根本不读 rcfile，钩子装不上。
        args: vec!["--rcfile".to_string(), rcfile_path, "-i".to_string()],
        env: Vec::new(),
        cleanup: Some(SessionCleanup::new(move || {
            let _ = fs::remove_dir_all(&dir);
        })),
    })
}

/// zsh：把 `ZDOTDIR` 指向临时目录，逐个 source 用户原有的启动文件后再装钩子。
///
/// 局限：用户的 `.zshenv` 若自己改写了 `ZDOTDIR`，后续文件会从新位置读取，
/// 临时 `.zshrc` 便不会被读到——集成随之降级为未生效，但用户配置仍然正常。
fn prepare_zsh() -> Result<LocalShellPreload, String> {
    let script = integration_script(ShellKind::Zsh).ok_or("缺少 zsh 集成脚本")?;
    let dir = create_temp_dir("transom-zsh")?;
    // zsh 按 ZDOTDIR 依次找 .zshenv/.zprofile/.zshrc/.zlogin，缺哪个就不读哪个。
    for name in [".zshenv", ".zprofile", ".zlogin"] {
        write_file(&dir.join(name), &source_user_file(name))?;
    }
    write_file(
        &dir.join(".zshrc"),
        &format!("{}{script}\n", source_user_file(".zshrc")),
    )?;
    let zdotdir = dir.to_string_lossy().into_owned();
    Ok(LocalShellPreload {
        kind: Some(ShellKind::Zsh),
        args: Vec::new(),
        env: vec![("ZDOTDIR".to_string(), zdotdir)],
        cleanup: Some(SessionCleanup::new(move || {
            let _ = fs::remove_dir_all(&dir);
        })),
    })
}

/// 生成"source 用户同名启动文件"的一行；文件不存在时静默跳过。
fn source_user_file(name: &str) -> String {
    format!("if [ -f \"$HOME/{name}\" ]; then . \"$HOME/{name}\"; fi\n")
}

/// 创建本次会话专属的临时目录。
fn create_temp_dir(prefix: &str) -> Result<PathBuf, String> {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|delta| delta.as_nanos())
        .unwrap_or_default();
    let dir = std::env::temp_dir().join(format!("{prefix}-{}-{nanos}", std::process::id()));
    fs::create_dir_all(&dir).map_err(|error| format!("创建临时目录失败: {error}"))?;
    Ok(dir)
}

/// 写入启动文件。内容只有 source 语句与钩子脚本，不含任何机密。
fn write_file(path: &std::path::Path, content: &str) -> Result<(), String> {
    fs::write(path, content).map_err(|error| format!("写入 {} 失败: {error}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::{ShellKind, prepare_bash, prepare_zsh, source_user_file};

    /// 用户启动文件不存在时要静默跳过，不能让 Shell 因一条 source 报错而中断。
    #[test]
    fn source_line_is_guarded() {
        let line = source_user_file(".bashrc");
        assert!(line.contains("[ -f \"$HOME/.bashrc\" ]"));
        assert!(
            line.contains("; then ."),
            "必须带 then 分支，缺失文件时不执行"
        );
        assert!(line.ends_with('\n'));
    }

    /// bash 预装：临时 rcfile 先加载用户原有配置再装钩子，命令行带 `--rcfile` 与 `-i`。
    #[test]
    fn bash_preload_writes_rcfile_with_user_config_first() {
        let preload = prepare_bash().expect("bash 预装应当成功");
        assert_eq!(preload.kind, Some(ShellKind::Bash));
        let rcfile = preload
            .args
            .iter()
            .position(|arg| arg == "--rcfile")
            .and_then(|index| preload.args.get(index + 1))
            .expect("必须给出 rcfile 路径");
        assert!(
            preload.args.iter().any(|arg| arg == "-i"),
            "没有 -i bash 不会读 rcfile"
        );
        let content = std::fs::read_to_string(rcfile).expect("rcfile 应当可读");
        let user_config = content
            .find("$HOME/.bashrc")
            .expect("必须先加载用户原有配置");
        let hook = content.find("ready=bash").expect("必须装上集成脚本");
        assert!(user_config < hook, "用户配置必须排在钩子之前");
    }

    /// zsh 预装：`ZDOTDIR` 指向临时目录，临时 `.zshrc` 同样先加载用户配置再装钩子。
    #[test]
    fn zsh_preload_points_zdotdir_at_temp_dir() {
        let preload = prepare_zsh().expect("zsh 预装应当成功");
        assert_eq!(preload.kind, Some(ShellKind::Zsh));
        let zdotdir = preload
            .env
            .iter()
            .find(|(key, _)| key == "ZDOTDIR")
            .map(|(_, value)| value.clone())
            .expect("必须设置 ZDOTDIR");
        let zshrc = std::path::Path::new(&zdotdir).join(".zshrc");
        let content = std::fs::read_to_string(&zshrc).expect("临时 .zshrc 应当可读");
        let user_config = content
            .find("$HOME/.zshrc")
            .expect("必须先加载用户原有配置");
        let hook = content.find("ready=zsh").expect("必须装上集成脚本");
        assert!(user_config < hook, "用户配置必须排在钩子之前");
    }
}
