//! 编排（Compose）项目的起停：引擎没有 Compose 的 REST 接口，落到 CLI 实现。
//!
//! # 设计原理（Why）
//! - 跨平台与远端引擎支持：容器标签 `working_dir` 在远端 Docker 或不同操作系统下本地可能不存在；
//!   对于 `stop`、`restart`、`down` 等生命周期操作，只要提供工程名 `-p <project>` 即可直接控制，无需依赖本地目录；
//! - 引擎端点上下文透传：通过透传 `-H`（以及 TLS 认证标志），确保 Compose 命令始终作用于目标容器引擎，杜绝误操作本地默认守护进程；
//! - 进程生命周期纳管：子进程配置 `kill_on_drop` 与显式 `child.kill()`，支持取消令牌中断，避免遗留僵尸孤儿进程。
//!
//! # 代价与局限（Trade-offs）
//! - `up` 操作需要解析本地 Compose 配置文件与构建上下文，当连接远端引擎且本地无源码时无法执行 `up`。

use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use tokio::io::AsyncReadExt;
use transom_core::{ComposeAction, ComposeProject, ContainerEndpoint};

use crate::engine::Engine;
use crate::error::ContainerError;

/// 单次编排命令的总超时：拉镜像可能很久。
const COMPOSE_TIMEOUT: Duration = Duration::from_secs(300);
/// 输出回显上限。
const OUTPUT_LIMIT: usize = 8 * 1024;
/// 取消检查节拍：compose 卡住时也要能及时收尾。
const CANCEL_TICK: Duration = Duration::from_millis(250);

/// 执行一次编排项目命令，返回 CLI 输出尾部摘要。
///
/// # Errors
/// 项目名称为空、缺少必要工作目录（仅针对 Up 操作）、CLI 不存在、超时、取消或非零退出码时返回 [`ContainerError`]。
pub async fn run(
    engine: &Engine,
    project: &ComposeProject,
    action: ComposeAction,
    cancel: &Arc<AtomicBool>,
) -> Result<String, ContainerError> {
    let endpoint = &engine.info.engine.endpoint;
    let is_remote = matches!(
        endpoint,
        ContainerEndpoint::Tcp { .. } | ContainerEndpoint::Ssh { .. }
    );
    let local_dir = validate_project(project, action, is_remote)?;

    let (program, command_prefix) = compose_program(endpoint, engine.info.engine.kind);
    let endpoint_args = endpoint_cli_args(endpoint);

    log::info!(
        "执行编排命令，项目: {}，命令: {}，工作目录: {:?}，端点参数: {:?}",
        project.name,
        action.label(),
        local_dir,
        endpoint_args
    );

    let first = spawn_compose(
        &program,
        project,
        action,
        &endpoint_args,
        &command_prefix,
        local_dir.as_deref(),
        cancel,
    )
    .await?;
    if first.status.success() {
        return Ok(tail_text(&first.stdout, &first.stderr));
    }

    // 只装了独立版 compose 的机器上 `docker compose` 会直接失败，回落到 docker-compose。
    let Some(fallback) = fallback_program(&program) else {
        return Err(ContainerError::Cli {
            command: display_command(&program, action, &endpoint_args),
            message: tail_text(&first.stdout, &first.stderr),
        });
    };

    let retry = spawn_compose(
        &fallback,
        project,
        action,
        &endpoint_args,
        &command_prefix,
        local_dir.as_deref(),
        cancel,
    )
    .await?;
    if retry.status.success() {
        return Ok(tail_text(&retry.stdout, &retry.stderr));
    }

    Err(ContainerError::Cli {
        command: display_command(&fallback, action, &endpoint_args),
        message: tail_text(&retry.stdout, &retry.stderr),
    })
}

/// 前置校验：跨平台校验工作目录与配置文件。
///
/// 对于 `down`/`stop`/`restart` 操作，Docker Compose V2 允许只依据工程名 `-p` 停止容器，
/// 本地不存在工作目录（如远程引擎项目）时降级为无目录执行；
/// 对于 `up` 操作，若本地目录不存在则报错。
fn validate_project(
    project: &ComposeProject,
    action: ComposeAction,
    is_remote: bool,
) -> Result<Option<String>, ContainerError> {
    let operation = format!("编排命令 {}：项目 {}", action.label(), project.name);
    if project.name.trim().is_empty() {
        return Err(ContainerError::Unsupported {
            operation: format!("{operation}：项目名称为空"),
        });
    }

    let local_dir = project.working_dir.as_deref().and_then(|dir| {
        let path = Path::new(dir);
        if path.is_dir() {
            Some(dir.to_string())
        } else {
            None
        }
    });

    if local_dir.is_none() && matches!(action, ComposeAction::Up) {
        if is_remote {
            return Err(ContainerError::Unsupported {
                operation: format!(
                    "{operation}：远程引擎项目的工作目录在本地不可访问，无法重新创建容器"
                ),
            });
        }
        return Err(ContainerError::Unsupported {
            operation: format!("{operation}：本地工作目录不存在或已被删除"),
        });
    }

    Ok(local_dir)
}

/// 根据引擎端点配置生成 docker / podman CLI 的全局前置参数（如 -H 与 TLS 参数）。
pub fn endpoint_cli_args(endpoint: &ContainerEndpoint) -> Vec<String> {
    let mut args = Vec::new();
    match endpoint {
        ContainerEndpoint::Auto => {}
        ContainerEndpoint::Unix { path } => {
            args.push("-H".to_string());
            args.push(format!("unix://{}", path.display()));
        }
        ContainerEndpoint::NamedPipe { path } => {
            args.push("-H".to_string());
            args.push(format!("npipe://{path}"));
        }
        ContainerEndpoint::Tcp { url, tls } => {
            args.push("-H".to_string());
            args.push(url.clone());
            if let Some(tls) = tls
                && !tls.is_empty()
            {
                if !tls.ca_cert.is_empty() {
                    args.push("--tlscacert".to_string());
                    args.push(tls.ca_cert.clone());
                    args.push("--tlsverify".to_string());
                } else {
                    args.push("--tls".to_string());
                }
                if !tls.client_cert.is_empty() {
                    args.push("--tlscert".to_string());
                    args.push(tls.client_cert.clone());
                }
                if !tls.client_key.is_empty() {
                    args.push("--tlskey".to_string());
                    args.push(tls.client_key.clone());
                }
            }
        }
        ContainerEndpoint::Ssh {
            host, port, user, ..
        } => {
            let mut target = String::from("ssh://");
            if let Some(user) = user {
                target.push_str(user);
                target.push('@');
            }
            target.push_str(host);
            if let Some(port) = port {
                target.push(':');
                target.push_str(&port.to_string());
            }
            args.push("-H".to_string());
            args.push(target);
        }
        ContainerEndpoint::Wsl { .. } | ContainerEndpoint::Wslc | ContainerEndpoint::Cli { .. } => {
        }
    }
    args
}

/// 选择 compose 的实际程序与前缀参数。
///
/// WSL 端点必须通过 `wsl.exe -d <distro> -e docker` 进入发行版；
/// 自定义 CLI 端点直接复用配置里的 program/args；其余端点走引擎 CLI。
fn compose_program(
    endpoint: &ContainerEndpoint,
    kind: transom_core::ContainerEngineKind,
) -> (String, Vec<String>) {
    match endpoint {
        ContainerEndpoint::Wsl { distro } => (
            "wsl.exe".to_string(),
            vec![
                "-d".to_string(),
                distro.clone(),
                "-e".to_string(),
                kind.cli_program().to_string(),
            ],
        ),
        ContainerEndpoint::Wslc => ("wslc".to_string(), Vec::new()),
        ContainerEndpoint::Cli { program, args } => (program.clone(), args.clone()),
        _ => (kind.cli_program().to_string(), Vec::new()),
    }
}

/// 组装 CLI 参数：端点参数、CLI 前缀先行，工程名与配置文件作为独立 argv 传入，
/// 不经过 Shell。
fn build_args(
    program: &str,
    project: &ComposeProject,
    action: ComposeAction,
    endpoint_args: &[String],
    command_prefix: &[String],
    local_dir: Option<&str>,
) -> Vec<String> {
    let mut args = Vec::with_capacity(
        project.config_files.len() * 2 + endpoint_args.len() + command_prefix.len() + 6,
    );
    // 1. 全局端点参数与 CLI 前缀必须位于子命令之前
    args.extend(endpoint_args.iter().cloned());
    args.extend(command_prefix.iter().cloned());

    // 2. compose 子命令
    if !is_standalone(program) {
        args.push("compose".to_string());
    }

    // 3. 项目名称
    args.extend(["-p".to_string(), project.name.clone()]);

    // 4. 配置文件（仅当本地工作目录可用时加载配置文件路径）
    if local_dir.is_some() {
        for config in &project.config_files {
            args.extend(["-f".to_string(), config.clone()]);
        }
    }

    // 5. 动作参数
    args.extend(action.args().iter().map(|arg| (*arg).to_string()));
    args
}

fn is_standalone(program: &str) -> bool {
    program.ends_with("-compose")
}

/// 插件形态才有独立版回落。
fn fallback_program(program: &str) -> Option<String> {
    match program {
        "docker" => Some("docker-compose".to_string()),
        "podman" => Some("podman-compose".to_string()),
        _ => None,
    }
}

/// 日志与错误里显示的命令形态。
fn display_command(program: &str, action: ComposeAction, endpoint_args: &[String]) -> String {
    let ep = if endpoint_args.is_empty() {
        String::new()
    } else {
        format!("{} ", endpoint_args.join(" "))
    };
    if is_standalone(program) {
        format!("{program} {ep}{}", action.subcommand())
    } else {
        format!("{program} {ep}compose {}", action.subcommand())
    }
}

struct Completed {
    status: std::process::ExitStatus,
    stdout: Vec<u8>,
    stderr: Vec<u8>,
}

async fn spawn_compose(
    program: &str,
    project: &ComposeProject,
    action: ComposeAction,
    endpoint_args: &[String],
    command_prefix: &[String],
    working_dir: Option<&str>,
    cancel: &Arc<AtomicBool>,
) -> Result<Completed, ContainerError> {
    let args = build_args(
        program,
        project,
        action,
        endpoint_args,
        command_prefix,
        working_dir,
    );
    let mut cmd = tokio::process::Command::new(program);
    cmd.args(&args);

    if let Some(dir) = working_dir {
        cmd.current_dir(dir);
    } else {
        cmd.current_dir(std::env::temp_dir());
    }

    cmd.stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped());

    let mut child = crate::cli::spawn_grouped(&mut cmd).map_err(|error| ContainerError::Cli {
        command: display_command(program, action, endpoint_args),
        message: error.to_string(),
    })?;

    let stdout_pipe = child.inner().stdout.take();
    let stderr_pipe = child.inner().stderr.take();
    let stdout_task = tokio::spawn(read_capped(stdout_pipe));
    let stderr_task = tokio::spawn(read_capped(stderr_pipe));

    let deadline = tokio::time::Instant::now() + COMPOSE_TIMEOUT;
    let status = loop {
        if cancel.load(Ordering::Relaxed) {
            crate::cli::terminate_child(&mut child).await;
            return Err(ContainerError::Cancelled);
        }
        if tokio::time::Instant::now() >= deadline {
            crate::cli::terminate_child(&mut child).await;
            return Err(ContainerError::Cli {
                command: display_command(program, action, endpoint_args),
                message: format!("编排命令超过 {} 秒未结束", COMPOSE_TIMEOUT.as_secs()),
            });
        }
        tokio::select! {
            waited = child.wait() => break waited.map_err(|error| ContainerError::Cli {
                command: display_command(program, action, endpoint_args),
                message: error.to_string(),
            })?,
            _ = tokio::time::sleep(CANCEL_TICK) => continue,
        }
    };

    Ok(Completed {
        status,
        stdout: stdout_task.await.unwrap_or_default(),
        stderr: stderr_task.await.unwrap_or_default(),
    })
}

/// 读输出并截断到上限。
async fn read_capped(pipe: Option<impl tokio::io::AsyncRead + Unpin + Send>) -> Vec<u8> {
    let Some(mut pipe) = pipe else {
        return Vec::new();
    };
    let mut buffer = Vec::with_capacity(1024);
    let mut chunk = [0u8; 4096];
    while let Ok(count) = pipe.read(&mut chunk).await {
        if count == 0 {
            break;
        }
        buffer.extend_from_slice(&chunk[..count]);
        if buffer.len() > OUTPUT_LIMIT {
            let excess = buffer.len() - OUTPUT_LIMIT;
            buffer.drain(..excess);
        }
    }
    buffer
}

/// 输出尾部摘要：优先取 stderr。
fn tail_text(stdout: &[u8], stderr: &[u8]) -> String {
    let preferred = if stderr.iter().any(|byte| !byte.is_ascii_whitespace()) {
        stderr
    } else {
        stdout
    };
    let text = String::from_utf8_lossy(preferred).trim().to_string();
    if text.is_empty() {
        return "无输出".to_string();
    }
    if text.len() > 1024 {
        let mut start = text.len() - 1024;
        while !text.is_char_boundary(start) {
            start += 1;
        }
        return format!("…{}", &text[start..]);
    }
    text
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    use transom_core::TlsConfig;

    #[test]
    fn test_endpoint_cli_args_mapping() {
        let unix = ContainerEndpoint::Unix {
            path: PathBuf::from("/custom/docker.sock"),
        };
        assert_eq!(
            endpoint_cli_args(&unix),
            vec!["-H", "unix:///custom/docker.sock"]
        );

        let tcp = ContainerEndpoint::Tcp {
            url: "tcp://192.168.1.50:2376".to_string(),
            tls: Some(TlsConfig {
                ca_cert: "/ca.pem".to_string(),
                client_cert: "/cert.pem".to_string(),
                client_key: "/key.pem".to_string(),
            }),
        };
        let args = endpoint_cli_args(&tcp);
        assert!(args.contains(&"-H".to_string()));
        assert!(args.contains(&"tcp://192.168.1.50:2376".to_string()));
        assert!(args.contains(&"--tlsverify".to_string()));
        assert!(args.contains(&"/ca.pem".to_string()));

        let ssh = ContainerEndpoint::Ssh {
            host: "server.com".to_string(),
            port: Some(2222),
            user: Some("deploy".to_string()),
            identity_file: None,
            password: None,
            use_sudo: false,
            sudo_password: None,
        };
        assert_eq!(
            endpoint_cli_args(&ssh),
            vec!["-H", "ssh://deploy@server.com:2222"]
        );
    }

    #[test]
    fn test_build_args_with_remote_and_local_dirs() {
        let project = ComposeProject {
            name: "web_app".to_string(),
            working_dir: Some("/non/existent/remote/dir".to_string()),
            config_files: vec!["docker-compose.yml".to_string()],
            running: 2,
            total: 2,
        };

        let endpoint_args = vec!["-H".to_string(), "tcp://remote:2375".to_string()];

        // 远端且本地无目录：down 操作不带 -f
        let down_args = build_args(
            "docker",
            &project,
            ComposeAction::Down,
            &endpoint_args,
            &[],
            None,
        );
        assert_eq!(
            down_args,
            vec![
                "-H",
                "tcp://remote:2375",
                "compose",
                "-p",
                "web_app",
                "down"
            ]
        );

        // 本地有目录：包含 -f
        let up_args = build_args(
            "docker",
            &project,
            ComposeAction::Up,
            &endpoint_args,
            &[],
            Some("/local/path"),
        );
        assert_eq!(
            up_args,
            vec![
                "-H",
                "tcp://remote:2375",
                "compose",
                "-p",
                "web_app",
                "-f",
                "docker-compose.yml",
                "up",
                "-d"
            ]
        );
    }

    #[test]
    fn wsl_endpoint_runs_compose_inside_distro() {
        let (program, prefix) = compose_program(
            &ContainerEndpoint::Wsl {
                distro: "Ubuntu-24.04".to_string(),
            },
            transom_core::ContainerEngineKind::Docker,
        );
        assert_eq!(program, "wsl.exe");
        assert_eq!(prefix, vec!["-d", "Ubuntu-24.04", "-e", "docker"]);
    }

    #[test]
    fn test_validate_project_rules() {
        let remote_project = ComposeProject {
            name: "test".to_string(),
            working_dir: Some("/remote/dir/not/here".to_string()),
            config_files: vec!["compose.yml".to_string()],
            running: 1,
            total: 1,
        };

        // Down 在远端即使无本地目录也允许通过
        assert!(validate_project(&remote_project, ComposeAction::Down, true).is_ok());

        // Up 在远端且无本地目录时明确返回错误
        assert!(validate_project(&remote_project, ComposeAction::Up, true).is_err());
    }
}
