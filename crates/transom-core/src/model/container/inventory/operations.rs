//! 引擎级清理报告、容器生命周期、更新与构建参数。

use serde::{Deserialize, Serialize};

/// 一次清理（prune）的结果。
///
/// 设计原理（Why）：引擎的 prune 只告诉调用方删了哪些名字与回收了多少字节，
/// 具体删了几个必须回给用户看（否则「清理未使用卷」点下去毫无反馈，无法判断
/// 是本来就没得清还是没清成）。
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ContainerPruneReport {
    /// 被删除的条目名（卷名或镜像摘要）。
    #[serde(default)]
    pub deleted: Vec<String>,
    /// 引擎报告的回收字节数。
    #[serde(default)]
    pub reclaimed: u64,
    /// 系统级联合清理中失败但未阻断其它步骤的操作说明。
    ///
    /// 设计原理（Why）：联合清理包含容器、镜像、网络、卷与构建缓存五个独立步骤，
    /// 某一步失败不应让已经完成的删除结果丢失；把失败原因回传给界面，才能让用户
    /// 知道“回收了多少”之外还有哪些资源没清成。
    #[serde(default)]
    pub errors: Vec<String>,
}

/// 容器生命周期动作。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "action", rename_all = "snake_case")]
pub enum ContainerLifecycleAction {
    Start,
    Stop,
    Restart,
    Pause,
    Unpause,
    Kill {
        #[serde(default)]
        signal: Option<String>,
    },
    Remove {
        #[serde(default)]
        force: bool,
    },
}

impl ContainerLifecycleAction {
    /// 日志与 UI 使用的动作名。
    pub fn label(&self) -> &'static str {
        match self {
            Self::Start => "启动",
            Self::Stop => "停止",
            Self::Restart => "重启",
            Self::Pause => "暂停",
            Self::Unpause => "恢复",
            Self::Kill { .. } => "强制终止",
            Self::Remove { .. } => "删除",
        }
    }
}

/// 容器动态更新配置。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct ContainerUpdateSpec {
    /// 内存上限（字节）；None 表示不修改。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub memory_bytes: Option<u64>,
    /// CPU 配额（以 1e9 为 1 个核）；None 表示不修改。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub nano_cpus: Option<u64>,
    /// 重启策略；None 表示不修改。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub restart_policy: Option<String>,
}

/// 镜像构建参数。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct ContainerBuildSpec {
    /// 构建上下文所在的本地目录路径。
    pub context_path: String,
    /// 相对上下文路径的 Dockerfile 路径（默认为 "Dockerfile"）。
    #[serde(default)]
    pub dockerfile: Option<String>,
    /// 构建生成的标签列表（如 `["myapp:v1", "myapp:latest"]`）。
    #[serde(default)]
    pub tags: Vec<String>,
    /// 是否不使用缓存进行构建。
    #[serde(default)]
    pub no_cache: bool,
    /// 构建参数（ARG 参数名与值映射）。
    #[serde(default)]
    pub build_args: std::collections::HashMap<String, String>,
    /// 多阶段构建目标阶段名称。
    #[serde(default)]
    pub target: Option<String>,
    /// 目标平台架构（如 "linux/amd64", "linux/arm64"）。
    #[serde(default)]
    pub platform: Option<String>,
}
