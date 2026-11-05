//! Podman Quadlet 单元发现与 systemd 用户单元操作。
//!
//! 设计原理（Why）：Quadlet 不是引擎 API，而是宿主机 systemd 读取的声明式
//! 单元文件；因此这里只扫描固定目录，不接收任意路径，也不把文件内容拼进
//! Shell。状态查询与启停都通过 systemctl 的 argv 接口完成。
//!
//! 代价与局限：只覆盖当前用户的 systemd 会话；rootful 系统级 Quadlet 需要
//! 用户在宿主侧管理。远程引擎不会显示本机 Quadlet，避免误操作。

use crate::error::ContainerError;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::Duration;
use transom_core::{ContainerQuadletAction, ContainerQuadletUnit};

/// Quadlet 支持的扩展名；`.image`/`.build` 不直接生成可启动服务。
const QUADLET_EXTENSIONS: [&str; 5] = ["container", "pod", "volume", "network", "kube"];
/// 状态查询与启停的超时；systemctl 卡住时不能拖死后台任务。
const SYSTEMCTL_QUERY_TIMEOUT: Duration = Duration::from_secs(3);
const SYSTEMCTL_ACTION_TIMEOUT: Duration = Duration::from_secs(30);

/// 发现当前用户与系统级 Quadlet 单元。
///
/// # Errors
/// 目录读取失败不视为致命错误：某个目录不可读时跳过，全部目录不可读才
/// 返回 [`ContainerError::Cli`] 以便界面提示。
pub fn list() -> Result<Vec<ContainerQuadletUnit>, ContainerError> {
    let mut directories = Vec::with_capacity(2);
    if let Some(config) = directories::BaseDirs::new() {
        directories.push(config.config_dir().join("containers").join("systemd"));
    }
    directories.push(PathBuf::from("/etc/containers/systemd"));
    let mut units = Vec::new();
    let mut readable = false;
    for directory in directories {
        if !directory.is_dir() {
            continue;
        }
        readable = true;
        read_directory(&directory, &mut units);
    }
    if !readable {
        return Ok(Vec::new());
    }
    units.sort_by(|left, right| left.name.cmp(&right.name));
    units.dedup_by(|left, right| left.name == right.name);
    apply_states(&mut units);
    Ok(units)
}

/// 对 Quadlet 单元执行 systemctl --user 操作。
///
/// # Errors
/// 单元名非法、systemctl 启动失败、超时或退出码非零时返回错误。
pub fn action(unit: &str, action: ContainerQuadletAction) -> Result<(), ContainerError> {
    if !valid_unit_name(unit) {
        return Err(ContainerError::InvalidPath {
            path: unit.to_string(),
            reason: "Quadlet 单元名非法".to_string(),
        });
    }
    let verb = match action {
        ContainerQuadletAction::Start => "start",
        ContainerQuadletAction::Stop => "stop",
        ContainerQuadletAction::Restart => "restart",
    };
    let output = run_systemctl(&[verb, unit], SYSTEMCTL_ACTION_TIMEOUT)?;
    if !output.status.success() {
        return Err(ContainerError::Cli {
            command: format!("systemctl --user {verb} {unit}"),
            message: String::from_utf8_lossy(&output.stderr).trim().to_string(),
        });
    }
    Ok(())
}

fn read_directory(directory: &Path, units: &mut Vec<ContainerQuadletUnit>) {
    let Ok(entries) = std::fs::read_dir(directory) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_file() {
            continue;
        }
        let Some(kind) = path.extension().and_then(|value| value.to_str()) else {
            continue;
        };
        if !QUADLET_EXTENSIONS.contains(&kind) {
            continue;
        }
        let Some(stem) = path.file_stem().and_then(|value| value.to_str()) else {
            continue;
        };
        let unit = format!("{stem}.service");
        if !valid_unit_name(&unit) {
            continue;
        }
        units.push(ContainerQuadletUnit {
            name: unit,
            path: path.display().to_string(),
            kind: kind.to_string(),
            active: None,
        });
    }
}

/// 一次查询全部单元状态，避免为每个文件 fork 一次 systemctl。
fn apply_states(units: &mut [ContainerQuadletUnit]) {
    if units.is_empty() {
        return;
    }
    let mut args = Vec::with_capacity(units.len() + 2);
    args.push("is-active".to_string());
    args.extend(units.iter().map(|unit| unit.name.clone()));
    let Ok(output) = run_systemctl_args(&args, SYSTEMCTL_QUERY_TIMEOUT) else {
        return;
    };
    let stdout = String::from_utf8_lossy(&output.stdout);
    let states: Vec<&str> = stdout.lines().map(str::trim).collect();
    for (unit, state) in units.iter_mut().zip(states) {
        unit.active = Some(matches!(state, "active" | "activating" | "reloading"));
    }
}

fn run_systemctl(args: &[&str], timeout: Duration) -> Result<std::process::Output, ContainerError> {
    let owned: Vec<String> = args.iter().map(|arg| (*arg).to_string()).collect();
    run_systemctl_args(&owned, timeout)
}

fn run_systemctl_args(
    args: &[String],
    timeout: Duration,
) -> Result<std::process::Output, ContainerError> {
    let mut child = Command::new("systemctl")
        .arg("--user")
        .args(args)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|error| ContainerError::Cli {
            command: format!("systemctl --user {}", args.join(" ")),
            message: error.to_string(),
        })?;
    let start = std::time::Instant::now();
    loop {
        if let Ok(Some(_)) = child.try_wait() {
            return child
                .wait_with_output()
                .map_err(|error| ContainerError::Cli {
                    command: format!("systemctl --user {}", args.join(" ")),
                    message: error.to_string(),
                });
        }
        if start.elapsed() > timeout {
            let _ = child.kill();
            let _ = child.wait();
            return Err(ContainerError::Cli {
                command: format!("systemctl --user {}", args.join(" ")),
                message: format!("systemctl 超过 {} 秒未返回", timeout.as_secs()),
            });
        }
        std::thread::sleep(Duration::from_millis(25));
    }
}

/// 单元名只允许常见安全字符，且必须是以 `.service` 结尾的单段名称。
fn valid_unit_name(unit: &str) -> bool {
    let Some(stem) = unit.strip_suffix(".service") else {
        return false;
    };
    !stem.is_empty()
        && !stem.starts_with('-')
        && stem
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'))
}

#[cfg(test)]
mod tests {
    use super::{read_directory, valid_unit_name};

    #[test]
    fn directory_scan_keeps_only_supported_quadlet_files() {
        let directory = tempfile::tempdir().expect("临时目录");
        std::fs::write(directory.path().join("web.container"), b"[Container]").expect("写入");
        std::fs::write(directory.path().join("data.volume"), b"[Volume]").expect("写入");
        std::fs::write(directory.path().join("notes.txt"), b"ignore").expect("写入");
        std::fs::write(directory.path().join("-bad.container"), b"ignore").expect("写入");
        let mut units = Vec::new();
        read_directory(directory.path(), &mut units);
        let mut names: Vec<&str> = units.iter().map(|unit| unit.name.as_str()).collect();
        names.sort();
        assert_eq!(names, vec!["data.service", "web.service"]);
        assert!(
            units
                .iter()
                .any(|unit| unit.name == "web.service" && unit.kind == "container")
        );
    }

    #[test]
    fn unit_names_are_strictly_validated() {
        assert!(valid_unit_name("web.service"));
        assert!(valid_unit_name("web-1.service"));
        assert!(!valid_unit_name("web.container"));
        assert!(!valid_unit_name("--help.service"));
        assert!(!valid_unit_name("../etc/passwd.service"));
        assert!(!valid_unit_name("web.service;rm -rf /"));
    }
}
