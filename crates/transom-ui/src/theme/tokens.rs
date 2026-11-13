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
    /// 活动栏宽度（标准档 48px）；状态栏按列分段时用它加侧栏宽定位分界线。
    pub const ACTIVITY_BAR_WIDTH: f32 = 3.0;
    /// 活动栏按钮边长（标准档 34px）。
    pub const ACTIVITY_BUTTON_SIZE: f32 = 2.125;
    /// 活动栏图标边长（标准档 18px）；与会话树行图标尺寸保持一致，让左侧
    /// 两条竖栏的视觉分量对齐（树侧常量为模块私有，调整时两处必须同步）。
    pub const ACTIVITY_ICON_SIZE: f32 = 1.125;
    /// 侧栏最小宽度（标准档 180px）。
    pub const SIDEBAR_MIN_WIDTH: f32 = 11.25;
    /// 侧栏初始宽度（标准档 270px）；用户拖出的宽度存于 ResizableState。
    pub const SIDEBAR_WIDTH: f32 = 16.875;
    /// 侧栏最大宽度（标准档 500px）。
    pub const SIDEBAR_MAX_WIDTH: f32 = 31.25;
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
    /// 设置弹窗宽度（标准档 896px）。
    ///
    /// 收窄自 1088px：设置行是"左标题 + 右控件"的两列结构，面板越宽，两列之间
    /// 的空白越显眼，输入类控件也被拉得越长；896px 下内容区仍有 644px，够放
    /// 快捷键行的标签、按键与操作按钮。
    pub const SETTINGS_DIALOG_WIDTH: f32 = 56.0;
    /// 设置弹窗高度（标准档 736px）。
    pub const SETTINGS_DIALOG_HEIGHT: f32 = 46.0;
    /// 侧栏树行槽位高度（标准档 38px）。
    pub const TREE_ROW_HEIGHT: f32 = 2.375;
    /// 侧栏树内层高亮条高度（标准档 34px）。
    pub const TREE_BAND_HEIGHT: f32 = 2.125;
    /// 侧栏树行内主图标边长（标准档 18px）；与活动栏图标一致。
    pub const TREE_ROW_ICON: f32 = 1.125;
    /// 侧栏树折叠箭头边长（标准档 14px）。
    pub const TREE_CHEVRON: f32 = 0.875;
    /// 侧栏当前行左侧强调条宽度（标准档 2px）。
    pub const SIDEBAR_SELECTION_RAIL_WIDTH: f32 = 0.125;
    /// 标准文件列表行高（标准档 28px）。
    pub const FILE_ROW_HEIGHT: f32 = 1.75;
    /// 紧凑文件树行高（标准档 24px，用于变更文件树等紧凑清单）。
    pub const COMPACT_TREE_ROW_HEIGHT: f32 = 1.5;
}
/// 间距档位：以 rem 表达，供组件层替换 `gap_2` / `py_2` 这类魔法数字。
///
/// 现有页面仍大量使用 gpui 的 `gap_2()` 等快捷方法；新组件与新页面必须从
/// 这里取值，逐步迁移，避免一次性改动全部布局导致视觉回归。
pub(crate) mod space {
    /// 2px：行内极紧间距。
    pub const XXS: f32 = 0.125;
    /// 4px：图标与文字、紧凑控件间距。
    pub const XS: f32 = 0.25;
    /// 6px：带图标按钮内部图标与文案的标准间距。
    pub const COMPACT: f32 = 0.375;
    /// 8px：表单行内、按钮组默认间距。
    pub const SM: f32 = 0.5;
    /// 12px：卡片内区块间距。
    pub const MD: f32 = 0.75;
    /// 16px：页面级区块间距。
    pub const LG: f32 = 1.0;
    /// 24px：大区块或空状态留白。
    pub const XL: f32 = 1.5;
}

/// 字号档位：以 rem 表达，组件层统一用 `text_size(rems(...))`。
pub(crate) mod typography {
    /// 12px：说明、辅助信息与表头。
    pub const CAPTION: f32 = 0.75;
    /// 14px：正文与设置行标题。
    pub const BODY: f32 = 0.875;
    /// 16px：区块标题。
    pub const TITLE: f32 = 1.0;
}

/// 控件尺寸档位：像素语义的点击目标，随密度等比缩放。
pub(crate) mod control {
    /// 16px：常规工具图标。
    pub const ICON: f32 = 1.0;
    /// 14px：紧凑工具栏图标。
    pub const COMPACT_ICON: f32 = 0.875;
    /// 26px：侧栏工具按钮。
    pub const ICON_BUTTON: f32 = 1.625;
    /// 22px：标签栏紧凑按钮。
    pub const COMPACT_ICON_BUTTON: f32 = 1.375;
    /// 20px：列表行内动作按钮。
    pub const ROW_ACTION: f32 = 1.25;
}

/// 弹窗宽度档位：以 rem 表达，所有弹窗都随密度缩放。
pub(crate) mod dialog {
    pub const AUTH: f32 = 27.5;
    pub const CLIPBOARD: f32 = 32.5;
    pub const CONFIRM: f32 = 28.75;
    pub const INPUT: f32 = 26.25;
    pub const CHMOD: f32 = 32.5;
    pub const FILTER: f32 = 35.0;
    pub const SEARCH: f32 = 35.0;
    pub const EDITOR: f32 = 26.25;
    pub const SESSION_FORM: f32 = 47.5;
    pub const FOLDER_FORM: f32 = 30.0;
    pub const BATCH_EDIT: f32 = 35.0;
    pub const SNIPPET_FORM: f32 = 26.25;
    pub const KNOWN_HOSTS: f32 = 36.25;
    pub const BACKUP: f32 = 32.5;
    pub const AUDIT: f32 = 56.25;
    pub const CONTAINER_RUN: f32 = 54.0;
    pub const REPLAY: f32 = 35.0;
    pub const GIT_FORM: f32 = 41.25;
    pub const GIT_CLONE: f32 = 45.0;
    pub const GIT_PUSH: f32 = 48.0;
    pub const GIT_CONFLICT: f32 = 62.0;
}
