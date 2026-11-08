#![cfg(windows)]

//! windows 相关的引擎候选发现。

use super::cli::{find_in_path, wslc_spec};
use super::{EngineCandidate, explicit_candidate};
use crate::transport::EngineTransport;
use std::collections::HashSet;
use std::path::Path;
use std::time::Duration;
use transom_core::{ContainerEndpoint, ContainerEngineKind};

/// WSL 发行版枚举上限；WSL 服务异常时不能拖住整个自动发现。
const WSL_LIST_TIMEOUT: Duration = Duration::from_secs(3);
/// 单发行版 Docker 客户端探测上限；只检查是否安装，不要求守护进程已启动。
const WSL_DOCKER_PROBE_TIMEOUT: Duration = Duration::from_secs(2);

pub(super) fn windows_candidates(kind: ContainerEngineKind) -> Vec<EngineCandidate> {
    let mut candidates = Vec::with_capacity(4);
    if kind == ContainerEngineKind::Docker {
        for pipe in [
            r"//./pipe/docker_engine",
            r"//./pipe/dockerDesktopLinuxEngine",
        ] {
            candidates.push(EngineCandidate {
                kind,
                endpoint: ContainerEndpoint::NamedPipe {
                    path: pipe.to_string(),
                },
                transport: Some(EngineTransport::NamedPipe {
                    path: pipe.to_string(),
                }),
                cli: None,
            });
        }
    }
    if kind == ContainerEngineKind::Docker {
        candidates.extend(wsl_candidates(kind));
        if find_in_path("wslc.exe").is_some() {
            candidates.push(EngineCandidate {
                kind,
                endpoint: ContainerEndpoint::Wslc,
                transport: None,
                cli: Some(wslc_spec()),
            });
        }
    }
    candidates
}

/// 枚举已安装的 WSL 发行版，并为其中安装了 Docker 客户端的发行版生成候选。
///
/// 设计原理（Why）：Docker Desktop 的 WSL 集成不会统一暴露到 Windows 固定命名管道，
/// 每个启用集成的发行版都有自己的守护进程上下文。先运行一次发行版列表，再逐项检查
/// Docker 客户端，可避免把没有 Docker 的发行版交给后续 REST 探测并制造无意义错误。
///
/// 代价与局限：WSL 首次启动发行版可能较慢，因此每个调用都设置独立上限；本函数只由
/// 自动发现路径在阻塞线程池中调用，不会阻塞 UI 或 Tokio 工作线程。
fn wsl_candidates(kind: ContainerEngineKind) -> Vec<EngineCandidate> {
    let Some(wsl) = find_in_path("wsl.exe").or_else(|| find_in_path("wsl")) else {
        return Vec::new();
    };
    let distros = list_wsl_distros(&wsl).unwrap_or_else(|error| {
        log::debug!("枚举 WSL 发行版失败，跳过 WSL 自动发现: {error}");
        Vec::new()
    });
    let mut candidates = Vec::with_capacity(distros.len());
    for distro in distros {
        if !distro_has_docker(&wsl, &distro) {
            log::debug!("WSL 发行版未检测到 Docker 客户端，跳过自动发现，发行版: {distro}");
            continue;
        }
        let endpoint = ContainerEndpoint::Wsl {
            distro: distro.clone(),
        };
        if let Some(candidate) = explicit_candidate(kind, &endpoint) {
            candidates.push(candidate);
        }
    }
    candidates
}

/// 读取已安装发行版列表，并解析 WSL 常见的 UTF-16LE 输出。
fn list_wsl_distros(wsl: &Path) -> Result<Vec<String>, String> {
    let mut command = std::process::Command::new(wsl);
    command.args(["--list", "--quiet"]);
    let output = crate::cli::run_grouped_output(&mut command, WSL_LIST_TIMEOUT)
        .map_err(|error| format!("wsl --list --quiet 执行失败: {error}"))?;
    if !output.status.success() {
        return Err(format!(
            "wsl --list --quiet 返回非零状态: {}，错误: {}",
            output.status,
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    Ok(parse_wsl_distro_list(&decode_wsl_output(&output.stdout)))
}

/// 检查发行版内是否存在 Docker 客户端；守护进程可用性留给统一的引擎探测阶段。
fn distro_has_docker(wsl: &Path, distro: &str) -> bool {
    let mut command = std::process::Command::new(wsl);
    command.args(["-d", distro, "-e", "docker", "--version"]);
    crate::cli::run_grouped_output(&mut command, WSL_DOCKER_PROBE_TIMEOUT)
        .map(|output| output.status.success())
        .unwrap_or_else(|error| {
            log::debug!("WSL 内 Docker 客户端探测失败，发行版: {distro}，原因: {error}");
            false
        })
}

/// WSL CLI 在管道模式下通常输出 UTF-16LE，而部分版本会输出 UTF-8。
fn decode_wsl_output(bytes: &[u8]) -> String {
    let bytes = bytes.strip_prefix(&[0xff, 0xfe]).unwrap_or(bytes);
    let odd_zeros = bytes
        .iter()
        .skip(1)
        .step_by(2)
        .filter(|byte| **byte == 0)
        .count();
    let looks_utf16 = !bytes.is_empty() && odd_zeros.saturating_mul(8) >= bytes.len();
    if !looks_utf16 {
        return String::from_utf8_lossy(bytes).into_owned();
    }
    let (units, _) = bytes.as_chunks::<2>();
    let units = units
        .iter()
        .map(|chunk| u16::from_le_bytes([chunk[0], chunk[1]]));
    char::decode_utf16(units)
        .map(|result| result.unwrap_or(char::REPLACEMENT_CHARACTER))
        .collect()
}

/// 解析发行版名称并按 Windows 的 ASCII 大小写不敏感语义去重。
fn parse_wsl_distro_list(text: &str) -> Vec<String> {
    let mut seen = HashSet::with_capacity(8);
    let mut distros = Vec::with_capacity(8);
    for line in text.lines() {
        let name = line.trim_matches(|character: char| {
            character == '\0' || character == '\u{feff}' || character.is_whitespace()
        });
        if name.is_empty() {
            continue;
        }
        let key = name.to_ascii_lowercase();
        if seen.insert(key) {
            distros.push(name.to_string());
        }
    }
    distros
}

#[cfg(test)]
mod tests {
    use super::{decode_wsl_output, parse_wsl_distro_list};

    #[test]
    fn decodes_utf16_distro_output() {
        let bytes: Vec<u8> = "Ubuntu-24.04\r\nDebian\r\n"
            .encode_utf16()
            .flat_map(u16::to_le_bytes)
            .collect();
        assert_eq!(
            parse_wsl_distro_list(&decode_wsl_output(&bytes)),
            ["Ubuntu-24.04", "Debian"]
        );
    }

    #[test]
    fn parses_utf8_output_and_deduplicates_names() {
        let text = "\u{feff}Ubuntu-24.04\r\n\r\nubuntu-24.04\r\nDebian\r\n";
        assert_eq!(parse_wsl_distro_list(text), ["Ubuntu-24.04", "Debian"]);
    }
}
