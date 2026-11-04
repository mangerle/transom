//! 容器镜像构建（Build）弹窗状态契约。
//!
//! 设计原理（Why）：
//! - 将从 Dockerfile 构建镜像作为专用的向导弹窗，包含上下文目录、Dockerfile 相对路径、标签配置与免缓存开关；
//! - 内部维护日志行缓冲区（logs），以便将后台推送的流式构建分层输出实时渲染给用户；
//! - 维护 building 状态与在途请求 ID，支持在构建过程中显示加载状态与取消控制。

use gpui_kit::component::input::InputState;
use gpui_kit::*;
use transom_core::ContainerRequestId;

/// 从 Dockerfile 构建镜像弹窗状态。
pub(crate) struct ContainerBuildDialog {
    /// 构建上下文所在的本地目录路径输入框。
    pub(crate) context_path: Entity<InputState>,
    /// Dockerfile 相对路径输入框（默认为 "Dockerfile"）。
    pub(crate) dockerfile: Entity<InputState>,
    /// 构建生成的镜像标签输入框（如 "myapp:latest"）。
    pub(crate) tag: Entity<InputState>,
    /// 是否开启免缓存构建 (--no-cache)。
    pub(crate) no_cache: bool,
    /// 是否正在执行构建。
    pub(crate) building: bool,
    /// 当前在途的构建请求 ID。
    pub(crate) request_id: Option<ContainerRequestId>,
    /// 流式接收到的构建输出日志行。
    pub(crate) logs: Vec<String>,
    /// 构建失败时的错误信息。
    pub(crate) error: Option<String>,
    /// 构建是否已成功完成。
    pub(crate) success: bool,
    /// 输入框变更与回车提交订阅。
    pub(crate) _subscriptions: Vec<Subscription>,
}
