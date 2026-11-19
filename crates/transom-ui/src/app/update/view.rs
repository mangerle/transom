//! 应用更新弹窗视图。

use super::snapshot::UpdateSnapshot;
use super::{UpdatePrimaryAction, UpdateView};
use crate::palette::Palette;
use crate::theme::tokens::{size, space, typography};
use crate::widgets::{
    DialogConfirmKind, dialog_actions, dialog_body, dialog_cancel_button, dialog_confirm_button,
};
use gpui_kit::base::{TextView, h_flex, v_flex};
use gpui_kit::component::Disableable;
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::progress::Progress;
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::*;
use transom_ui_icons::{AppIcon, UiIcon};

impl UpdateView {
    /// 渲染更新弹窗：版本摘要、Markdown 更新日志与单一主动作。
    pub(crate) fn render_body(&self, view: &Entity<Self>, p: &Palette) -> AnyElement {
        let snapshot = self.snapshot();
        let close = dialog_cancel_button(
            "app-update-close",
            rust_i18n::t!("common.close").to_string(),
            {
                let view = view.clone();
                move |_, _, cx| {
                    view.update(cx, |this, cx| this.close(cx));
                }
            },
        );
        let primary = render_primary_button(&snapshot, view);
        let footer = dialog_actions([close.into_any_element(), primary]);

        let description = rust_i18n::t!("settings.update_desc").to_string();
        dialog_body(
            "app-update-dialog",
            rust_i18n::t!("settings.update_title").to_string(),
            Some(description),
            render_content(&snapshot, p),
            Some(footer),
        )
    }
}

/// 版本摘要、状态、进度和更新日志。
fn render_content(snapshot: &UpdateSnapshot, p: &Palette) -> AnyElement {
    let status_color = if snapshot.failed {
        p.danger
    } else {
        p.foreground
    };
    let mut content = v_flex()
        .w_full()
        .gap(rems(space::MD))
        .child(
            h_flex()
                .w_full()
                .items_center()
                .justify_between()
                .gap(rems(space::SM))
                .child(version_meta(
                    rust_i18n::t!("settings.update_current_version").to_string(),
                    format!("v{}", snapshot.current_version),
                    p,
                ))
                .when_some(snapshot.target_version.clone(), |row, version| {
                    row.child(version_meta(
                        rust_i18n::t!("settings.update_latest_version").to_string(),
                        format!("v{version}"),
                        p,
                    ))
                }),
        )
        .child(
            div()
                .text_size(rems(typography::BODY))
                .font_weight(FontWeight::MEDIUM)
                .text_color(status_color)
                .child(snapshot.status.clone()),
        );

    if let Some(published) = snapshot.published.as_ref() {
        content = content.child(
            div()
                .text_size(rems(typography::CAPTION))
                .text_color(p.muted_foreground)
                .child(
                    rust_i18n::t!("settings.update_published", date = published.as_str())
                        .to_string(),
                ),
        );
    }
    if let Some(detail) = snapshot.detail.as_ref() {
        content = content.child(
            div()
                .text_size(rems(typography::CAPTION))
                .text_color(p.muted_foreground)
                .child(detail.clone()),
        );
    }
    if snapshot.indeterminate {
        content = content.child(Progress::new("app-update-progress").loading(true).w_full());
    } else if let Some(percent) = snapshot.progress {
        content = content.child(Progress::new("app-update-progress").value(percent).w_full());
    }

    content
        .child(render_release_notes(snapshot, p))
        .into_any_element()
}

/// 当前版本与新版本的一列元数据。
fn version_meta(label: String, value: String, p: &Palette) -> AnyElement {
    v_flex()
        .gap(rems(space::XXS))
        .child(
            div()
                .text_size(rems(typography::CAPTION))
                .text_color(p.muted_foreground)
                .child(label),
        )
        .child(
            div()
                .text_size(rems(typography::BODY))
                .font_weight(FontWeight::MEDIUM)
                .child(value),
        )
        .into_any_element()
}

/// 更新日志区：Markdown 保持可选中、可滚动，并限制固定可视高度。
fn render_release_notes(snapshot: &UpdateSnapshot, p: &Palette) -> AnyElement {
    let Some(notes) = snapshot.notes.as_ref() else {
        return div()
            .text_size(rems(typography::CAPTION))
            .text_color(p.muted_foreground)
            .child(rust_i18n::t!("settings.update_notes_empty").to_string())
            .into_any_element();
    };

    v_flex()
        .w_full()
        .gap(rems(space::XS))
        .child(
            div()
                .text_size(rems(typography::CAPTION))
                .font_weight(FontWeight::SEMIBOLD)
                .child(rust_i18n::t!("settings.update_notes_title").to_string()),
        )
        .child(
            div()
                .id("app-update-notes-viewport")
                .w_full()
                .h(rems(size::UPDATE_NOTES_HEIGHT))
                .overflow_hidden()
                .child(
                    TextView::markdown("app-update-notes", notes.clone())
                        .selectable(true)
                        .scrollable(true),
                ),
        )
        .into_any_element()
}

/// 根据更新阶段生成唯一主按钮；`Enter` 与点击都走同一动作。
fn render_primary_button(snapshot: &UpdateSnapshot, view: &Entity<UpdateView>) -> AnyElement {
    if snapshot.primary == UpdatePrimaryAction::Busy {
        return Button::new("app-update-busy")
            .primary()
            .label(rust_i18n::t!("settings.update_busy_action").to_string())
            .disabled(true)
            .into_any_element();
    }

    let (id, label_key, icon) = match snapshot.primary {
        UpdatePrimaryAction::Check => (
            "app-update-check",
            "settings.update_check_action",
            AppIcon::Ui(UiIcon::REFRESH),
        ),
        UpdatePrimaryAction::Download => (
            "app-update-download",
            "settings.update_download_action",
            AppIcon::Ui(UiIcon::DOWNLOAD),
        ),
        UpdatePrimaryAction::Install => (
            "app-update-install",
            "settings.update_install_action",
            AppIcon::Ui(UiIcon::DOWNLOAD),
        ),
        UpdatePrimaryAction::Restart => (
            "app-update-restart",
            "settings.update_restart_action",
            AppIcon::Ui(UiIcon::UPDATE),
        ),
        UpdatePrimaryAction::Busy => return div().into_any_element(),
    };
    let button_view = view.clone();
    dialog_confirm_button(
        id,
        rust_i18n::t!(label_key).to_string(),
        Some(icon),
        DialogConfirmKind::Primary,
        move |_, _, cx| {
            button_view.update(cx, |this, cx| this.perform_primary_action_for_dialog(cx));
        },
    )
    .into_any_element()
}
