//! 设置实体向外壳发出的意图。
//!
//! 设计原理（Why）：设置弹窗拥有自己的输入控件、页面渲染与保存流程，但
//! "把外观下发到活窗格"、"切换 SFTP 停靠方向"、"重启监控采样"这类动作需要
//! 窗口级协调，不能由弹窗直接改别的领域状态。事件把跨域部分显式交回外壳，
//! 外壳在创建时订阅一次；事件自带全部数据，外壳处理时不得回读设置实体，
//! 避免实体回调期间的双重租借。
//!
//! 代价与局限：一次设置变更可能同时产生本地状态变更与一条事件，调用点需要
//! 明确区分"本域可完成"与"必须上交"的部分；事件不携带窗口句柄，需要窗口的
//! 动作由外壳排进渲染期请求队列。

use crate::monitor::{MonitorSectionCollapse, SampleIntervalMode};
use transom_core::{AppSettings, PanelSide, SessionCatalog, TabPosition, TriggerSettings};

/// 设置实体对外发出的意图。
pub(crate) enum SettingsEvent {
    /// 设置已成功落盘；外壳负责把新配置热应用到活窗格、快捷键、块视图与触发器。
    Saved(Box<AppSettings>),
    /// 弹窗被取消；外观预览已由实体回滚，外壳把回滚后的配置重新下发到活窗格。
    Cancelled,
    /// 外观即时预览发生变化；外壳把终端外观下发到活窗格。
    ProfilePreview,
    /// 会话目录被设置页修改（凭据模板）；外壳替换目录、刷新标签会话并落盘。
    CatalogChanged(Box<SessionCatalog>),
    /// SFTP 面板停靠方向变化；列宽状态归外壳，需要搬动分组。
    SetSftpPanelSide(PanelSide),
    /// 监控采样档位变化；外壳重启采样任务。
    MonitorIntervalChanged(SampleIntervalMode),
    /// 监控板块默认折叠状态变化；外壳同步到已打开的面板。
    MonitorSectionsChanged(MonitorSectionCollapse),
    /// 监控抽屉默认宽度变化；外壳同步到已打开的面板。
    MonitorDrawerWidthChanged(f32),
    /// 标签位置切换；外壳同步左侧标签槽位与会话树可见性。
    TabPositionChanged(TabPosition),
    /// 请求打开改键录制弹窗。
    OpenShortcutCapture(&'static str),
    /// 请求打开备份导入/导出选择器。
    OpenBackupImport,
    OpenBackupExport,
    /// 请求打开审计查看器。
    OpenAuditViewer,
    /// 云端同步动作。
    CloudSyncPush,
    CloudSyncPull,
    GitSyncNow,
    /// 触发器规则变化；外壳把新规则下发到活终端。
    ApplyTriggers(Box<TriggerSettings>),
    /// 状态栏提示。
    Status(String),
    /// 应用级错误提示。
    Error(String),
    /// 更新已安装，请求退出事件循环并在优雅停机后重启。
    RestartForUpdate,
}

/// 需要窗口句柄才能执行、必须排到渲染期处理的设置请求。
///
/// 事件订阅回调只有 `&mut App`，拿不到 `Window`；文件选择器、弹窗栈与
/// 云同步启动都依赖窗口，因此先入队，由外壳在渲染期统一消费。
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum SettingsRequest {
    OpenShortcutCapture(&'static str),
    OpenBackupImport,
    OpenBackupExport,
    OpenAuditViewer,
    CloudSyncPush,
    CloudSyncPull,
    GitSyncNow,
}
