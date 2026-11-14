//! 密钥向导的外壳职责：打开入口、部署回执与实体事件消费。
//!
//! 向导状态、输入控件、公钥清单与全部动作都在 `KeyWizardView`；外壳只做
//! 跨域的三件事：打开入口、把部署回执交给实体展示、把实体上报的状态栏
//! 提示/错误落到 `AppState`。事件自带全部数据，这里禁止回读实体，避免实体
//! `emit` 期间形成双重租借。

use super::entity::KeyWizardEvent;
use crate::app::AppShell;
use crate::palette::localized_error;
use gpui_kit::*;
use transom_core::AppError;

/// 打开密钥生成与管理向导。
pub(crate) fn open_key_wizard(shell: &mut AppShell, window: &mut Window, cx: &mut Context<AppShell>) {
    let view = shell.state.modals.tools.key_wizard.clone();
    view.update(cx, |view, cx| view.open(window, cx));
    cx.notify();
}

/// 消费向导实体上报的状态栏提示与错误。
pub(crate) fn on_key_wizard_event(shell: &mut AppShell, event: &KeyWizardEvent, cx: &mut Context<AppShell>) {
    match event {
        KeyWizardEvent::Status(message) => shell.state.status = Some(message.clone()),
        KeyWizardEvent::Error(message) => shell.state.set_error(message.clone()),
    }
    cx.notify();
}

/// 部署回执：实体更新弹窗内文案，外壳写状态栏或错误通知。
///
/// 状态栏文案必须在 `error` 被移入实体之前取定；实体 `update` 期间外壳
/// 只持有自己的状态，不会形成对同一实体的双重租借。
pub(crate) fn handle_public_key_deployed(
    shell: &mut AppShell,
    key_name: String,
    error: Option<AppError>,
    cx: &mut Context<AppShell>,
) {
    match &error {
        Some(error) => shell.state.set_error(localized_error(error)),
        None => {
            shell.state.status = Some(
                rust_i18n::t!("keys.deployed_status", key = key_name.as_str()).to_string(),
            );
        }
    }
    let view = shell.state.modals.tools.key_wizard.clone();
    view.update(cx, |view, cx| view.on_deployed(key_name, error, cx));
    cx.notify();
}
