#![cfg(windows)]

//! windows 相关的引擎候选发现。

use super::EngineCandidate;
use super::cli::{find_in_path, wslc_spec};
use crate::transport::EngineTransport;
use transom_core::{ContainerEndpoint, ContainerEngineKind};

#[cfg(windows)]
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
    // WSLC 与 WSL 发行版枚举依赖子进程，放在显式发现或用户选择时进行；
    // 这里只做低成本的可执行文件探测。
    if kind == ContainerEngineKind::Docker && find_in_path("wslc.exe").is_some() {
        candidates.push(EngineCandidate {
            kind,
            endpoint: ContainerEndpoint::Wslc,
            transport: None,
            cli: Some(wslc_spec()),
        });
    }
    candidates
}
