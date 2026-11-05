//! Docker Context 原生支持：读取系统 Docker 上下文、端点解析与动态切换。
//!
//! ### 设计原理（Why）
//! - **实现初衷**：现代 Docker 工作流中，开发者普遍使用 `docker context` 管理多套环境（如本机 socket、
//!   Orbstack、Colima、远程 SSH 服务器或云原生 Kubernetes 端点）。为了降低开发者在 Transom 中重复配置
//!   远端主机的摩擦成本，系统需要原生兼容 Docker Context，自动感知已有的上下文配置并支持切换。
//! - **核心优势**：采用“CLI 命令优先 + 磁盘配置回退”的双通道发现机制。一方面通过 `docker context ls --format '{{json .}}'`
//!   获取最准确的动态运行状态；另一方面在无 Docker CLI 或受限容器沙箱内，直接扫描 `~/.docker/contexts/meta/`
//!   与 `~/.docker/config.json` 获取静态元数据，并可将其无损转换为内部的 [`EngineCandidate`]。
//! - **代价与局限**：纯磁盘回退模式依赖 Docker 官方的上下文文件存储目录规范，若用户使用了定制的
//!   上下文凭证存储后端，静态回退可能缺失部分动态参数；因此本模块严格保证 CLI 优先。

use crate::discovery::{EngineCandidate, parse_endpoint};
use crate::error::ContainerError;
use directories::BaseDirs;
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::Path;
use std::process::Command;
use std::time::Duration;
use transom_core::ContainerEngineKind;

/// Docker Context 元数据信息。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DockerContextInfo {
    /// 上下文名称（如 `default`、`remote-vps`、`colima`）。
    #[serde(alias = "Name")]
    pub name: String,
    /// 是否为当前系统默认处于活动状态的上下文。
    #[serde(default, alias = "Current")]
    pub current: bool,
    /// 用户填写的描述说明。
    #[serde(default, alias = "Description")]
    pub description: String,
    /// 绑定的 Docker 守护进程端点（如 `unix:///var/run/docker.sock`、`ssh://user@host`、`tcp://...`）。
    #[serde(default, alias = "DockerEndpoint")]
    pub docker_endpoint: String,
    /// 探测或配置异常说明（若正常则为 None）。
    #[serde(default, alias = "Error")]
    pub error: Option<String>,
}

impl DockerContextInfo {
    /// 转换为系统标准引擎候选。
    pub fn to_candidate(&self) -> Option<EngineCandidate> {
        let endpoint = parse_endpoint(&self.docker_endpoint)?;
        crate::discovery::explicit_candidate(ContainerEngineKind::Docker, &endpoint)
    }
}

/// 获取系统配置的所有 Docker Context 上下文清单。
///
/// 优先调用本地 `docker context ls --format '{{json .}}'` 命令；若 CLI 不可用或执行失败，
/// 则安全降级为扫描本地 `~/.docker/contexts/` 目录。
///
/// # Errors
/// 当 CLI 失败且磁盘目录完全不存在或不可读时返回 [`ContainerError`]。
pub fn list_docker_contexts() -> Result<Vec<DockerContextInfo>, ContainerError> {
    if let Ok(contexts) = list_contexts_from_cli()
        && !contexts.is_empty()
    {
        return Ok(contexts);
    }
    log::debug!("未能通过 Docker CLI 获取上下文，尝试读取本地磁盘配置兜底");
    let contexts = list_contexts_from_disk();
    if !contexts.is_empty() {
        return Ok(contexts);
    }
    // 若均无配置，生成一个默认的本地上下文
    Ok(vec![default_context()])
}

/// 切换当前 Docker 上下文。
///
/// 优先调用 `docker context use <NAME>` 命令；若失败则尝试更新 `~/.docker/config.json`。
///
/// # Errors
/// 上下文切换失败时返回 [`ContainerError::Cli`]。
pub fn use_docker_context(name: &str) -> Result<(), ContainerError> {
    log::info!("正在切换 Docker 上下文为: {name}");
    let output = Command::new("docker")
        .args(["context", "use", name])
        .output();

    match output {
        Ok(out) if out.status.success() => {
            log::info!("成功切换 Docker 上下文为: {name}");
            Ok(())
        }
        Ok(out) => {
            let stderr = String::from_utf8_lossy(&out.stderr).trim().to_string();
            // 尝试磁盘回退更新
            if let Err(disk_err) = switch_context_in_config_file(name) {
                Err(ContainerError::Cli {
                    command: format!("docker context use {name}"),
                    message: format!("CLI 错误: {stderr}；配置文件更新失败: {disk_err}"),
                })
            } else {
                log::info!("通过更新 ~/.docker/config.json 成功切换当前上下文为: {name}");
                Ok(())
            }
        }
        Err(err) => {
            // CLI 未找到或调用失败，走纯文件更新
            switch_context_in_config_file(name).map_err(|disk_err| ContainerError::Cli {
                command: format!("docker context use {name}"),
                message: format!("执行 docker 失败: {err}；配置文件回退失败: {disk_err}"),
            })
        }
    }
}

/// 扫描系统中所有的 Docker Context 并转换为待探测引擎候选。
pub fn discover_context_candidates() -> Vec<EngineCandidate> {
    let contexts = match list_docker_contexts() {
        Ok(c) => c,
        Err(e) => {
            log::debug!("获取 Docker Context 候选失败: {e}");
            return Vec::new();
        }
    };

    let mut candidates = Vec::with_capacity(contexts.len());
    for ctx in contexts {
        if ctx.name == "default" && ctx.docker_endpoint.is_empty() {
            continue;
        }
        if let Some(candidate) = ctx.to_candidate() {
            candidates.push(candidate);
        }
    }
    candidates
}

/// 通过 Docker CLI 列出上下文。
fn list_contexts_from_cli() -> Result<Vec<DockerContextInfo>, String> {
    let mut child = Command::new("docker")
        .args(["context", "ls", "--format", "{{json .}}"])
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .map_err(|e| format!("启动 docker context 命令失败: {e}"))?;

    // 设置超时等待，避免僵尸进程或永久挂起
    let start = std::time::Instant::now();
    let timeout = Duration::from_secs(3);
    loop {
        if let Some(status) = child.try_wait().map_err(|e| e.to_string())? {
            if !status.success() {
                return Err(format!("docker context ls 返回非零状态: {status}"));
            }
            break;
        }
        if start.elapsed() > timeout {
            let _ = child.kill();
            return Err("读取 docker context ls 超时".to_string());
        }
        std::thread::sleep(Duration::from_millis(50));
    }

    let output = child
        .wait_with_output()
        .map_err(|e| format!("读取输出失败: {e}"))?;
    let stdout = String::from_utf8_lossy(&output.stdout);

    let mut list = Vec::new();
    for line in stdout.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        if let Some(info) = parse_context_json_line(line) {
            list.push(info);
        }
    }
    Ok(list)
}

/// 解析单行 Docker Context JSON 输出。
pub fn parse_context_json_line(line: &str) -> Option<DockerContextInfo> {
    serde_json::from_str::<DockerContextInfo>(line).ok()
}

/// 从本地文件系统 `~/.docker/` 扫描上下文。
fn list_contexts_from_disk() -> Vec<DockerContextInfo> {
    let Some(base_dirs) = BaseDirs::new() else {
        return Vec::new();
    };
    let docker_dir = base_dirs.home_dir().join(".docker");
    let current_context = read_current_context_from_config(&docker_dir.join("config.json"))
        .unwrap_or_else(|| "default".to_string());

    let mut list = Vec::new();
    let meta_dir = docker_dir.join("contexts").join("meta");
    if meta_dir.is_dir()
        && let Ok(entries) = fs::read_dir(&meta_dir)
    {
        for entry in entries.flatten() {
            let meta_file = entry.path().join("meta.json");
            if meta_file.is_file()
                && let Some(info) = parse_meta_json_file(&meta_file, &current_context)
            {
                list.push(info);
            }
        }
    }

    // 补充 default 上下文
    let has_default = list.iter().any(|c| c.name == "default");
    if !has_default {
        let mut def = default_context();
        def.current = current_context == "default";
        list.insert(0, def);
    }
    list
}

/// 读取 `~/.docker/config.json` 中的 `currentContext`。
fn read_current_context_from_config(config_path: &Path) -> Option<String> {
    let content = fs::read_to_string(config_path).ok()?;
    let value: serde_json::Value = serde_json::from_str(&content).ok()?;
    value
        .get("currentContext")
        .and_then(|v| v.as_str())
        .map(ToString::to_string)
}

/// 修改 `~/.docker/config.json` 中的 `currentContext`。
fn switch_context_in_config_file(new_context: &str) -> Result<(), String> {
    let Some(base_dirs) = BaseDirs::new() else {
        return Err("无法定位用户主目录".to_string());
    };
    let config_path = base_dirs.home_dir().join(".docker").join("config.json");
    let mut value: serde_json::Value = if config_path.exists() {
        let content =
            fs::read_to_string(&config_path).map_err(|e| format!("读取 config.json 失败: {e}"))?;
        serde_json::from_str(&content).unwrap_or(serde_json::json!({}))
    } else {
        serde_json::json!({})
    };

    value["currentContext"] = serde_json::Value::String(new_context.to_string());
    let formatted =
        serde_json::to_string_pretty(&value).map_err(|e| format!("序列化 JSON 失败: {e}"))?;

    if let Some(parent) = config_path.parent() {
        let _ = fs::create_dir_all(parent);
    }
    fs::write(&config_path, formatted).map_err(|e| format!("写入 config.json 失败: {e}"))?;
    Ok(())
}

/// 解析单个 meta.json 文件。
fn parse_meta_json_file(path: &Path, current_context: &str) -> Option<DockerContextInfo> {
    let content = fs::read_to_string(path).ok()?;
    let val: serde_json::Value = serde_json::from_str(&content).ok()?;

    let name = val.get("Name")?.as_str()?.to_string();
    let description = val
        .get("Metadata")
        .and_then(|m| m.get("Description"))
        .and_then(|d| d.as_str())
        .unwrap_or("")
        .to_string();

    let docker_endpoint = val
        .get("Endpoints")
        .and_then(|e| e.get("docker"))
        .and_then(|d| d.get("Host"))
        .and_then(|h| h.as_str())
        .unwrap_or("")
        .to_string();

    let current = name == current_context;
    Some(DockerContextInfo {
        name,
        current,
        description,
        docker_endpoint,
        error: None,
    })
}

/// 构造本地默认上下文。
fn default_context() -> DockerContextInfo {
    let default_endpoint = if cfg!(windows) {
        "npipe:////./pipe/docker_engine".to_string()
    } else {
        "unix:///var/run/docker.sock".to_string()
    };
    DockerContextInfo {
        name: "default".to_string(),
        current: true,
        description: "默认本机守护进程".to_string(),
        docker_endpoint: default_endpoint,
        error: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    use transom_core::ContainerEndpoint;

    #[test]
    fn test_parse_context_json_line() {
        let line = r#"{"Current":true,"Description":"Current DOCKER_HOST based configuration","DockerEndpoint":"unix:///var/run/docker.sock","Error":"","Name":"default"}"#;
        let parsed = parse_context_json_line(line).expect("必须成功解析 Context JSON");
        assert_eq!(parsed.name, "default");
        assert!(parsed.current);
        assert_eq!(parsed.docker_endpoint, "unix:///var/run/docker.sock");
        assert_eq!(parsed.error, Some("".to_string()));

        let candidate = parsed.to_candidate().expect("必须能转换成候选");
        assert_eq!(candidate.kind, ContainerEngineKind::Docker);
        match candidate.endpoint {
            ContainerEndpoint::Unix { path } => {
                assert_eq!(path, PathBuf::from("/var/run/docker.sock"));
            }
            _ => panic!("端点类型不符合预期"),
        }
    }

    #[test]
    fn test_ssh_context_candidate() {
        let ctx = DockerContextInfo {
            name: "remote-dev".to_string(),
            current: false,
            description: "远程开发机".to_string(),
            docker_endpoint: "ssh://root@192.168.1.100:2222".to_string(),
            error: None,
        };
        let candidate = ctx.to_candidate().expect("应成功转换为候选");
        match candidate.endpoint {
            ContainerEndpoint::Ssh {
                host, port, user, ..
            } => {
                assert_eq!(host, "192.168.1.100");
                assert_eq!(port, Some(2222));
                assert_eq!(user, Some("root".to_string()));
            }
            _ => panic!("应解析为 SSH 端点"),
        }
    }

    #[test]
    fn test_parse_meta_json_file() {
        let dir = tempfile::tempdir().unwrap();
        let meta_path = dir.path().join("meta.json");
        let content = r#"{
            "Name": "orbstack",
            "Metadata": { "Description": "OrbStack engine" },
            "Endpoints": {
                "docker": { "Host": "unix:///home/user/.orbstack/run/docker.sock" }
            }
        }"#;
        fs::write(&meta_path, content).unwrap();

        let parsed = parse_meta_json_file(&meta_path, "orbstack").expect("应成功解析 meta.json");
        assert_eq!(parsed.name, "orbstack");
        assert!(parsed.current);
        assert_eq!(parsed.description, "OrbStack engine");
        assert_eq!(
            parsed.docker_endpoint,
            "unix:///home/user/.orbstack/run/docker.sock"
        );
    }
}
