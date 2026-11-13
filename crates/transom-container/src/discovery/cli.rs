//! cli 相关的引擎候选发现。

use super::{CliSpec, EngineCandidate};
use std::path::PathBuf;
use transom_core::{ContainerEndpoint, ContainerEngineKind};

/// CLI 兜底候选：只有可执行文件存在时才加入，避免无意义的探测。
pub(super) fn cli_candidate(kind: ContainerEngineKind) -> Option<EngineCandidate> {
    let program = kind.cli_program();
    let path = find_in_path(program)?;
    Some(EngineCandidate {
        kind,
        endpoint: ContainerEndpoint::Cli {
            program: path.to_string_lossy().to_string(),
            args: Vec::new(),
        },
        transport: None,
        cli: Some(CliSpec {
            program: path.to_string_lossy().to_string(),
            prefix: Vec::new(),
        }),
    })
}

/// 在 PATH 中查找可执行文件。
pub fn find_in_path(program: &str) -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    for directory in std::env::split_paths(&path) {
        let candidate = directory.join(program);
        if is_executable(&candidate) {
            return Some(candidate);
        }
        #[cfg(windows)]
        {
            for suffix in ["exe", "cmd", "bat"] {
                let candidate = directory.join(format!("{program}.{suffix}"));
                if candidate.is_file() {
                    return Some(candidate);
                }
            }
        }
    }
    None
}

fn is_executable(path: &std::path::Path) -> bool {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        path.is_file()
            && path
                .metadata()
                .map(|metadata| metadata.permissions().mode() & 0o111 != 0)
                .unwrap_or(false)
    }
    #[cfg(not(unix))]
    {
        path.is_file()
    }
}

/// WSLC 的 CLI 形态：优先 `wslc.exe`，部分发行版把别名装成 `container.exe`。
pub(super) fn wslc_spec() -> CliSpec {
    let program = find_in_path("wslc")
        .or_else(|| find_in_path("container"))
        .map(|path| path.to_string_lossy().to_string())
        .unwrap_or_else(|| "wslc.exe".to_string());
    CliSpec {
        program,
        prefix: Vec::new(),
    }
}
