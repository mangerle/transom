//! 本地终端可用的 Shell 目录。
//!
//! 设计原理（Why）：设置页需要展示“系统默认、已安装 Shell、自定义路径”，而
//! 终端控制器只接受最终可执行文件与参数。把发现与去重放在平台层，UI 不必各自
//! 解析 `/etc/shells`、Windows PATH、Git for Windows 安装布局或 WSL 启动器。
//!
//! 代价与局限：目录在应用启动时快照一次；用户在进程运行期间新安装 Shell 后，
//! 需要重启 Transom 才能在选择器中出现。这样可以避免渲染路径同步扫描文件系统。

#[cfg(unix)]
use std::collections::HashSet;
use std::path::{Path, PathBuf};

/// 一个可直接用于本地 PTY 的 Shell 选项。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ShellOption {
    /// 稳定的可执行文件路径或命令名，写入设置后不再依赖显示文案。
    pub program: String,
    /// 启动 Shell 时需要附加的固定参数。
    pub args: Vec<String>,
    /// 面向用户的短名称。
    pub label: String,
    /// 是否能通过 Windows 提权 PTY 代理以管理员身份运行。
    ///
    /// WSL 启动器本身可以提权，但发行版内部仍是普通用户，因此显式标为不支持，
    /// 避免用户误以为标签内已经是 root。
    pub supports_elevation: bool,
}

/// 发现当前系统可用的 Shell。
///
/// 返回值只包含实际存在的可执行文件，并按平台习惯排序；调用方应在此基础上
/// 自行追加“系统默认”和“自定义路径”两个逻辑选项。
pub fn discover_shells() -> Vec<ShellOption> {
    #[cfg(windows)]
    {
        discover_windows_shells()
    }
    #[cfg(unix)]
    {
        discover_unix_shells()
    }
    #[cfg(not(any(unix, windows)))]
    {
        Vec::new()
    }
}

/// 没有显式配置时使用的 Shell 程序。
pub fn system_default_program() -> String {
    #[cfg(unix)]
    {
        std::env::var("SHELL")
            .ok()
            .filter(|path| !path.trim().is_empty())
            .unwrap_or_else(|| "/bin/bash".to_string())
    }
    #[cfg(windows)]
    {
        // Windows 没有 POSIX 风格的登录 Shell 环境变量；保持与旧行为一致的
        // PowerShell 默认，避免升级后所有现有用户突然落到 cmd.exe。
        "powershell.exe".to_string()
    }
    #[cfg(not(any(unix, windows)))]
    {
        "sh".to_string()
    }
}

#[cfg(unix)]
fn discover_unix_shells() -> Vec<ShellOption> {
    let mut programs = Vec::with_capacity(8);
    if let Ok(shell) = std::env::var("SHELL") {
        push_existing(&mut programs, PathBuf::from(shell));
    }
    if let Ok(contents) = std::fs::read_to_string("/etc/shells") {
        for line in contents.lines() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            push_existing(&mut programs, PathBuf::from(line));
        }
    }
    for fallback in ["/bin/bash", "/bin/zsh", "/usr/bin/fish", "/bin/sh"] {
        push_existing(&mut programs, PathBuf::from(fallback));
    }

    let mut seen = HashSet::with_capacity(programs.len());
    let mut shells = Vec::with_capacity(programs.len());
    for program in programs {
        let key = std::fs::canonicalize(&program)
            .unwrap_or_else(|_| program.clone())
            .to_string_lossy()
            .into_owned();
        if !seen.insert(key) {
            continue;
        }
        shells.push(ShellOption {
            label: display_name(&program),
            program: program.to_string_lossy().into_owned(),
            args: Vec::new(),
            supports_elevation: false,
        });
    }
    shells
}

#[cfg(unix)]
fn push_existing(programs: &mut Vec<PathBuf>, program: PathBuf) {
    if program.is_file() && is_executable(&program) {
        programs.push(program);
    }
}

#[cfg(unix)]
fn is_executable(path: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    path.metadata()
        .map(|metadata| metadata.permissions().mode() & 0o111 != 0)
        .unwrap_or(false)
}

#[cfg(windows)]
fn discover_windows_shells() -> Vec<ShellOption> {
    let mut candidates = Vec::with_capacity(8);

    if let Some(path) = find_in_path("pwsh.exe") {
        candidates.push(ShellOption {
            label: "PowerShell 7".to_string(),
            program: path.to_string_lossy().into_owned(),
            args: Vec::new(),
            supports_elevation: true,
        });
    }
    let powershell = std::env::var_os("SystemRoot")
        .map(PathBuf::from)
        .map(|root| root.join("System32/WindowsPowerShell/v1.0/powershell.exe"))
        .filter(|path| path.is_file())
        .or_else(|| find_in_path("powershell.exe"));
    if let Some(path) = powershell {
        push_windows_unique(&mut candidates, "Windows PowerShell", path, true);
    }
    let cmd = std::env::var_os("COMSPEC")
        .map(PathBuf::from)
        .filter(|path| path.is_file())
        .or_else(|| find_in_path("cmd.exe"));
    if let Some(path) = cmd {
        push_windows_unique(&mut candidates, "Command Prompt", path, true);
    }
    for path in git_bash_candidates() {
        push_windows_unique(&mut candidates, "Git Bash", path, true);
    }
    for path in [find_in_path("wsl.exe"), find_in_path("wsl")]
        .into_iter()
        .flatten()
    {
        push_windows_unique(&mut candidates, "WSL", path, false);
    }
    candidates
}

#[cfg(windows)]
fn git_bash_candidates() -> Vec<PathBuf> {
    let mut candidates = Vec::with_capacity(5);
    if let Some(git) = find_in_path("git.exe").or_else(|| find_in_path("git"))
        && let Some(command_dir) = git.parent()
    {
        candidates.push(command_dir.join("bash.exe"));
        if let Some(git_root) = command_dir.parent() {
            candidates.push(git_root.join("bin/bash.exe"));
            candidates.push(git_root.join("usr/bin/bash.exe"));
        }
    }
    for variable in ["ProgramFiles", "ProgramFiles(x86)", "LOCALAPPDATA"] {
        if let Some(root) = std::env::var_os(variable) {
            let root = PathBuf::from(root);
            candidates.push(root.join("Git/bin/bash.exe"));
            candidates.push(root.join("Programs/Git/bin/bash.exe"));
        }
    }
    candidates
        .into_iter()
        .filter(|path| path.is_file())
        .collect()
}

#[cfg(windows)]
fn push_windows_unique(
    shells: &mut Vec<ShellOption>,
    label: &str,
    program: PathBuf,
    supports_elevation: bool,
) {
    let key = program.to_string_lossy().to_ascii_lowercase();
    if shells
        .iter()
        .any(|shell| shell.program.to_ascii_lowercase() == key)
    {
        return;
    }
    shells.push(ShellOption {
        program: program.to_string_lossy().into_owned(),
        args: Vec::new(),
        label: label.to_string(),
        supports_elevation,
    });
}

#[cfg(windows)]
fn find_in_path(program: &str) -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    let extensions = std::env::var("PATHEXT").unwrap_or_else(|_| ".COM;.EXE;.BAT;.CMD".to_string());
    for directory in std::env::split_paths(&path) {
        let direct = directory.join(program);
        if direct.is_file() {
            return Some(direct);
        }
        if Path::new(program).extension().is_none() {
            for extension in extensions.split(';').filter(|value| !value.is_empty()) {
                let candidate = directory.join(format!("{program}{extension}"));
                if candidate.is_file() {
                    return Some(candidate);
                }
            }
        }
    }
    None
}

#[cfg(unix)]
fn display_name(program: &Path) -> String {
    let name = program
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("shell");
    match name.to_ascii_lowercase().as_str() {
        "bash" => "Bash".to_string(),
        "zsh" => "Zsh".to_string(),
        "fish" => "Fish".to_string(),
        "sh" => "sh".to_string(),
        "ksh" => "Ksh".to_string(),
        "tcsh" => "Tcsh".to_string(),
        "nu" => "Nushell".to_string(),
        _ => name.to_string(),
    }
}

#[cfg(test)]
mod tests {
    #[cfg(unix)]
    use super::discover_shells;
    #[cfg(windows)]
    use super::discover_shells;
    #[cfg(unix)]
    use super::display_name;
    #[cfg(unix)]
    use std::path::Path;

    #[cfg(unix)]
    #[test]
    fn known_shell_names_are_human_readable() {
        assert_eq!(display_name(Path::new("/usr/bin/zsh")), "Zsh");
        assert_eq!(display_name(Path::new("/usr/bin/pwsh.exe")), "pwsh.exe");
    }

    #[cfg(unix)]
    #[test]
    fn unix_discovery_returns_only_existing_programs() {
        let shells = discover_shells();
        assert!(!shells.is_empty(), "Unix 至少应发现 /bin/sh 或 $SHELL");
        assert!(
            shells
                .iter()
                .all(|shell| Path::new(&shell.program).is_file())
        );
    }

    #[cfg(windows)]
    #[test]
    fn windows_discovery_finds_command_prompt() {
        let shells = discover_shells();
        assert!(
            shells.iter().any(|shell| shell.label == "Command Prompt"),
            "Windows 至少应发现 cmd.exe"
        );
    }
}
