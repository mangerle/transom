//! Compose 项目聚合与项目级动作。

use super::{ContainerState, ContainerSummary};
use serde::{Deserialize, Serialize};

/// 编排项目命令。
///
/// 设计原理（Why）：引擎没有 Compose 的 REST 接口，这两个动作只能由 CLI 完成；
/// 界面把它做成枚举而不是字符串，避免把任意参数拼进子进程。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ComposeAction {
    /// 按 compose 文件重新起（幂等：已存在的容器由 CLI 自己判断）。
    Up,
    /// 停止并删除该项目创建的容器与网络。
    Down,
    /// 重启项目下所有容器。
    Restart,
    /// 构建项目下定义的镜像。
    Build,
    /// 拉取项目下引用的最新镜像。
    Pull,
}

impl ComposeAction {
    /// 界面与日志用的人类可读名。
    pub fn label(self) -> &'static str {
        match self {
            Self::Up => "启动",
            Self::Down => "停止并删除",
            Self::Restart => "重启",
            Self::Build => "构建",
            Self::Pull => "拉取",
        }
    }

    /// 子命令名。
    pub fn subcommand(self) -> &'static str {
        match self {
            Self::Up => "up",
            Self::Down => "down",
            Self::Restart => "restart",
            Self::Build => "build",
            Self::Pull => "pull",
        }
    }

    /// 子命令及其参数。
    pub fn args(self) -> &'static [&'static str] {
        match self {
            Self::Up => &["up", "-d"],
            Self::Down => &["down"],
            Self::Restart => &["restart"],
            Self::Build => &["build"],
            Self::Pull => &["pull"],
        }
    }

    /// `down` 会删容器与网络，属于破坏性动作。
    pub fn is_dangerous(self) -> bool {
        matches!(self, Self::Down)
    }
}

/// Compose 项目（按容器标签聚合）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct ComposeProject {
    pub name: String,
    pub running: usize,
    pub total: usize,
    #[serde(default)]
    pub working_dir: Option<String>,
    #[serde(default)]
    pub config_files: Vec<String>,
}

impl ComposeProject {
    /// 项目状态摘要：`3/5 运行中`。
    pub fn state_label(&self) -> String {
        format!("{}/{} 运行中", self.running, self.total)
    }
}

/// 按容器标签聚合 Compose 项目。
///
/// 设计原理（Why）：Docker Engine 没有 Compose 专有 API，项目信息只存在于
/// 容器标签中；聚合放在领域层，HTTP 与 CLI 后端共用一套口径。
/// 代价：如果项目内所有容器都被删除，项目本身就不再可见。
pub fn compose_projects(containers: &[ContainerSummary]) -> Vec<ComposeProject> {
    let mut projects: std::collections::BTreeMap<String, ComposeProject> =
        std::collections::BTreeMap::new();
    for container in containers {
        let Some(name) = container.compose_project.as_deref() else {
            continue;
        };
        let project = projects
            .entry(name.to_string())
            .or_insert_with(|| ComposeProject {
                name: name.to_string(),
                running: 0,
                total: 0,
                working_dir: container.compose_working_dir.clone(),
                config_files: container.compose_config_files.clone(),
            });
        project.total += 1;
        if container.state == ContainerState::Running {
            project.running += 1;
        }
        if project.working_dir.is_none() {
            project.working_dir = container.compose_working_dir.clone();
        }
        if project.config_files.is_empty() {
            project.config_files = container.compose_config_files.clone();
        }
    }
    projects.into_values().collect()
}
