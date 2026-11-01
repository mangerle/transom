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

/// 设置弹窗实体。
pub(crate) struct SettingsView {
    /// 弹窗的输入控件与临时状态。
    pub(crate) state: SettingsModalState,
}

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
