//! 容器镜像市场探索与镜像推送弹窗状态。
//!
//! 设计原理（Why）：
//! - 将镜像搜索与市场浏览（Hub）作为轻量模态弹窗独立管理，支持在多仓库源之间即时切换；
//! - 镜像推送（Push）弹窗自动根据用户选择的目标仓库补全 Tag 前缀，并挂载对应凭据。

use gpui_kit::component::input::InputState;
use gpui_kit::*;
use transom_core::{ContainerRequestId, ImageSearchResult};

/// 远端镜像探索与检索弹窗。
pub(crate) struct ContainerHubDialog {
    /// 搜索关键字输入框。
    pub(crate) search_input: Entity<InputState>,
    /// 当前选中的仓库源 ID（如 "docker-hub" 或自定义 Harbor 的 ID）。
    pub(crate) selected_registry_id: String,
    /// 检索结果列表。
    pub(crate) results: Vec<ImageSearchResult>,
    /// 是否正在向远端执行检索。
    pub(crate) searching: bool,
    /// 检索失败或无匹配时的错误说明。
    pub(crate) search_error: Option<String>,
    /// 绑定的在途请求 ID。
    pub(crate) search_request: Option<ContainerRequestId>,
    /// 当前正在查看详情的镜像（Some 时在弹窗内展示详情视图，None 时展示搜索列表）。
    pub(crate) selected_detail: Option<ImageSearchResult>,
    /// 记录各镜像当前选中的版本标签（Key 为镜像名，如 "mysql" -> "8.0"；未指定时默认 "latest"）。
    pub(crate) selected_tags: std::collections::HashMap<String, String>,
    /// 详情页中的自定义版本输入框。
    pub(crate) custom_tag_input: Option<Entity<InputState>>,
    /// 详情页复制 CLI 命令成功提示。
    pub(crate) copy_toast: bool,
    /// 输入框变更订阅。
    pub(crate) _subscription: Subscription,
}

/// 镜像推送到远端仓库弹窗。
pub(crate) struct ContainerPushDialog {
    /// 待推送的源镜像 ID。
    pub(crate) image_id: String,
    /// 待推送的源完整 Tag（如 `nginx:latest`）。
    pub(crate) source_tag: String,
    /// 当前选中的目标仓库源 ID。
    pub(crate) selected_registry_id: String,
    /// 目标命名空间与仓库名输入框（如 `library/nginx` 或 `team/app`）。
    pub(crate) target_repo: Entity<InputState>,
    /// 目标 Tag 输入框（如 `latest` 或 `v1.0.0`）。
    pub(crate) target_tag: Entity<InputState>,
    /// 发生错误时的提示文本。
    pub(crate) error: Option<String>,
    /// 输入框事件订阅。
    pub(crate) _subscriptions: Vec<Subscription>,
}
