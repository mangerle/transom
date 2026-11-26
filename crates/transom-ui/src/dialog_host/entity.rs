//! 通用实体弹窗的渲染分发。

use super::ModalKind;
use crate::app::AppShell;
use crate::palette::Palette;
use gpui_kit::*;

impl ModalKind {
    /// 渲染由独立实体直接持有状态的通用弹窗。
    pub(super) fn render_entity_modal_body(
        self,
        app: &Entity<AppShell>,
        window: &mut Window,
        cx: &mut App,
        p: &Palette,
    ) -> Option<AnyElement> {
        let body = match self {
            ModalKind::Settings => {
                let settings = app.read(cx).state.ui.modals.settings.clone();
                settings.update(cx, |view, cx| view.render_body(&settings, window, cx, p))
            }
            ModalKind::Update => {
                let update = app.read(cx).state.features.update.clone();
                update.update(cx, |view, _cx| view.render_body(&update, p))
            }
            ModalKind::KnownHosts => {
                let view = app.read(cx).state.ui.modals.connection.known_hosts.clone();
                view.update(cx, |view, cx| view.render_body(cx, p))
            }
            ModalKind::BackupPrompt => {
                let view = app.read(cx).state.ui.modals.tools.backup_prompt.clone();
                view.update(cx, |view, cx| view.render_body(cx, p))
            }
            ModalKind::AuditViewer => {
                let view = app.read(cx).state.ui.modals.tools.audit_viewer.clone();
                view.update(cx, |view, cx| view.render_body(cx, p))
            }
            ModalKind::KeyWizard => {
                let (view, targets) = {
                    let app_state = &app.read(cx).state;
                    (
                        app_state.ui.modals.tools.key_wizard.clone(),
                        app_state.key_targets(),
                    )
                };
                view.update(cx, |view, cx| view.render_body(&targets, cx, p))
            }
            ModalKind::Replay => {
                let view = app.read(cx).state.ui.modals.tools.replay.clone();
                view.update(cx, |view, cx| view.render_body(cx, p))
            }
            ModalKind::SessionForm => {
                let view = app.read(cx).state.ui.modals.session.form.clone();
                view.update(cx, |view, cx| view.render_body(window, cx, p))
            }
            _ => return None,
        };
        Some(body)
    }
}
