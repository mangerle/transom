//! L2 结构尺寸 token：以 rem 表达的结构件尺寸。
//!
//! # 单位与换算
//! 窗口 rem 基准 = 主题字号（由密度档位 [`transom_core::UiDensity`] 驱动，
//! 根插件每帧同步），因此 rem 表达的结构件在密度切换时等比缩放。各常量的
//! 文档注释以标准档（16px 基准）锚定原始像素，作为调整时的对照。
//!
//! # 使用规则
//! - 结构件（栏高、面板宽、弹窗、窗口按钮）必须从本模块取值，禁止就地写
//!   `px()`：密度切换时这些尺寸必须一起动，散落的 px 会立即失调。
//! - 布局算术与 resizable 面板（只接受 `Pixels`）用 [`rem_px`] 换算。
//! - 微装饰（状态点、分隔线、缩进、行内小元素）保留就地 `px()`：它们是像素
//!   语义的细节，随密度缩放会变粗发虚，见各使用处。
//! - 模块私有度量（状态栏数值槽宽、树缩进）留在各自模块内按 rem 表达：跨
//!   模块无复用，集中到这里反而割裂上下文。

use gpui_kit::*;

/// 把 rem 常量按当前密度基准换算为 px，供布局算术与 resizable 面板使用。
pub(crate) fn rem_px(rem: f32, rem_size: Pixels) -> Pixels {
    px(rem * rem_size.as_f32())
}

pub(crate) mod size {
    /// 顶栏高度（标准档 36px）。
    pub const TOP_BAR_HEIGHT: f32 = 2.25;
    /// 标签栏高度（标准档 32px）。
    pub const TAB_BAR_HEIGHT: f32 = 2.0;
    /// 状态栏高度（标准档 28px）；快捷命令卡片的锚点与它保持一致。
    pub const STATUS_BAR_HEIGHT: f32 = 1.75;
    /// 窗格标题条高度（标准档 28px）。
    pub const PANE_HEADER_HEIGHT: f32 = 1.75;
    /// 顶栏窗口按钮边长（标准档 30px）。
    pub const WINDOW_BUTTON_SIZE: f32 = 1.875;
    /// 活动栏宽度（标准档 48px）；状态栏按列分段时用它加侧栏宽定位分界线。
    pub const ACTIVITY_BAR_WIDTH: f32 = 3.0;
    /// 活动栏按钮边长（标准档 34px）。
    pub const ACTIVITY_BUTTON_SIZE: f32 = 2.125;
    /// 活动栏图标边长（标准档 18px）；与会话树行图标尺寸保持一致，让左侧
    /// 两条竖栏的视觉分量对齐（树侧常量为模块私有，调整时两处必须同步）。
    pub const ACTIVITY_ICON_SIZE: f32 = 1.125;
    /// 侧栏初始宽度（标准档 270px）；用户拖出的宽度存于 ResizableState。
    pub const SIDEBAR_WIDTH: f32 = 16.875;
    /// 转发面板固定宽度（标准档 340px）。
    pub const FORWARD_PANEL_WIDTH: f32 = 21.25;
    /// SFTP 面板初始宽度（标准档 760px，双栏模式）。
    pub const SFTP_PANEL_WIDTH: f32 = 47.5;
    /// SFTP 面板初始宽度（标准档 480px，仅本地模式）。
    pub const SFTP_LOCAL_PANEL_WIDTH: f32 = 30.0;
    /// SFTP 面板最小宽度（标准档 360px，双栏模式）。
    pub const SFTP_MIN_WIDTH: f32 = 22.5;
    /// SFTP 面板最小宽度（标准档 320px，仅本地模式）。
    pub const SFTP_LOCAL_MIN_WIDTH: f32 = 20.0;
    /// 标签条目最小宽度（标准档 100px）。
    pub const TAB_MIN_WIDTH: f32 = 6.25;
    /// 标签条目最大宽度（标准档 220px）。
    pub const TAB_MAX_WIDTH: f32 = 13.75;
    /// 通用弹窗最大宽度（标准档 760px）。
    pub const DIALOG_MAX_WIDTH: f32 = 47.5;
    /// 通用弹窗最大高度（标准档 640px）。
    pub const DIALOG_MAX_HEIGHT: f32 = 40.0;
    /// 设置弹窗宽度（标准档 880px）。
    pub const SETTINGS_DIALOG_WIDTH: f32 = 55.0;
    /// 设置弹窗高度（标准档 640px）。
    pub const SETTINGS_DIALOG_HEIGHT: f32 = 40.0;
}
