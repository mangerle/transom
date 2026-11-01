//! 设置弹窗实体：持有弹窗的全部输入控件与临时状态。
//!
//! # 设计原理（Why）
//! - 设置弹窗是窗口级浮层，状态随打开/关闭生死，属于窗口级实体；把它从外壳
//!   状态里搬出来之后，二十多个输入框句柄有了明确归属，外壳只持有句柄；
//! - 面板/弹窗不持有外壳引用：设置值读 `Global`，需要外壳做事（落盘、应用主题、
//!   校验失败提示）时由调用方在实体 update 之外完成。
//!
//! # 代价与局限
//! 迁入阶段渲染仍由外壳组装（页面函数接收 `Context<AppShell>`），实体先接管状态；
//! 渲染搬迁完成后再让实体实现 `Render`。

use super::SettingsModalState;
use gpui_kit::EventEmitter;

/// 设置弹窗向外壳发出的意图。
///
/// 页面只改自身状态与 `Global` 设置；需要持久化、应用主题、开关弹窗等跨域动作时
/// 发事件，外壳订阅后执行。事件自带数据，外壳处理时不得回读弹窗。
#[derive(Clone)]
pub(crate) enum SettingsPanelEvent {
    /// 布局设置变化后落盘（停靠方向、活动栏显隐等）。
    PersistLayout,
    /// 保存并关闭弹窗。
    Save,
    /// 取消并关闭弹窗（还原外观预览）。
    Cancel,
    /// 恢复默认设置。
    Reset,
    /// 外观变化即时应用到当前窗口，取消时还原。
    PreviewAppearance,
    /// 终端配色随主题变化同步。
    SyncTerminalPalette,
    /// 关闭弹窗（不保存）。
    Close,
}

/// 设置弹窗实体。
pub(crate) struct SettingsView {
    /// 弹窗的输入控件与临时状态。
    pub(crate) state: SettingsModalState,
}

impl EventEmitter<SettingsPanelEvent> for SettingsView {}

impl SettingsView {
    /// 用已建好的状态创建弹窗实体。
    pub(crate) fn new(state: SettingsModalState) -> Self {
        Self { state }
    }

    /// 弹窗状态只读访问。
    pub(crate) fn state(&self) -> &SettingsModalState {
        &self.state
    }

    /// 弹窗状态可变访问。
    pub(crate) fn state_mut(&mut self) -> &mut SettingsModalState {
        &mut self.state
    }
}
