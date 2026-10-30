//! SFTP 内置文本编辑器、Linux 文件权限 (Chmod) 与文件属性模态对话框视图渲染。

use super::{
    SftpChmodModalState, SftpFilterModalState, SftpInputModalState, SftpPropertiesModalState,
    SftpSyncModalState, format_size, format_unix_time,
};
use crate::app::AppShell;
use crate::palette::Palette;
use gpui_kit::assets::IconName;
use gpui_kit::base::{Selectable, h_flex, v_flex};
use gpui_kit::component::Sizable;
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::checkbox::Checkbox;
use gpui_kit::component::input::{EditorState, Input, InputState, Textarea};
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::*;
use std::path::PathBuf;
use transom_core::{
    ConnectionId, RemoteDeleteTarget, TabId, format_octal_permissions, format_rwx_permissions,
};

/// 编辑器"跳转到行"弹窗的状态。
pub(crate) struct EditorGotoLineModalState {
    pub(crate) editor: Entity<EditorState>,
    pub(crate) input: Entity<InputState>,
    pub(crate) error: Option<String>,
    pub(crate) _enter_sub: Subscription,
}

/// 待用户确认的破坏性操作。
pub(crate) enum SftpPendingDelete {
    Local {
        tab_id: TabId,
        paths: Vec<PathBuf>,
    },
    Remote {
        tab_id: TabId,
        /// 目标连接；`None` 表示标签页主远端，`Some` 表示并列远端列。
        connection_id: Option<ConnectionId>,
        targets: Vec<RemoteDeleteTarget>,
    },
}

/// 删除确认弹窗：破坏性操作必须经过一次显式确认，且文案说明不可恢复。
pub(crate) struct SftpConfirmModalState {
    pub(crate) action: SftpPendingDelete,
    pub(crate) title: String,
    pub(crate) description: String,
}

/// SFTP 五类模态弹窗与编辑器跳转行的聚合状态。
///
/// 这些弹窗都依附于 SFTP 双栏或内置编辑器，统一由本模块管理显隐；
/// AppShell 只持有聚合对象，不再平铺五个互不相关的 `Option` 字段。
pub(crate) struct SftpModals {
    pub(crate) chmod: Option<SftpChmodModalState>,
    pub(crate) input: Option<SftpInputModalState>,
    pub(crate) properties: Option<SftpPropertiesModalState>,
    pub(crate) filter: Option<SftpFilterModalState>,
    pub(crate) sync: Option<SftpSyncModalState>,
    pub(crate) goto_line: Option<EditorGotoLineModalState>,
    pub(crate) confirm: Option<SftpConfirmModalState>,
    pub(crate) search: Option<super::SftpSearchModalState>,
}

impl AppShell {
    pub(crate) fn render_sftp_confirm_modal(
        &mut self,
        _window: &mut Window,
        cx: &mut Context<Self>,
        p: &Palette,
    ) -> AnyElement {
        let Some(modal) = self.sftp_modals.confirm.as_ref() else {
            return div().into_any_element();
        };
        let title = modal.title.clone();
        let description = modal.description.clone();

        let content = div()
            .text_sm()
            .text_color(p.muted_foreground)
            .child(description);
        let footer = h_flex()
            .gap_2()
            .child(
                Button::new("cancel-sftp-delete")
                    .ghost()
                    .label(rust_i18n::t!("common.cancel").to_string())
                    .on_click(cx.listener(|this, _, window, cx| {
                        this.sftp_modals.confirm = None;
                        this.focus_active_sftp_panel(window, cx);
                        cx.notify();
                    })),
            )
            .child(
                Button::new("confirm-sftp-delete")
                    .danger()
                    .label(rust_i18n::t!("common.confirm_delete").to_string())
                    .on_click(cx.listener(|this, _, window, cx| {
                        this.sftp_confirm_delete(cx);
                        this.focus_active_sftp_panel(window, cx);
                    })),
            );

        let dialog = Self::dialog_frame(
            "sftp-confirm-modal",
            title,
            None::<String>,
            content,
            Some(footer),
            px(460.0),
            p,
        );
        Self::overlay(dialog, p)
    }

    pub(crate) fn render_sftp_filter_modal(
        &mut self,
        _window: &mut Window,
        cx: &mut Context<Self>,
        p: &Palette,
    ) -> AnyElement {
        let Some(modal) = self.sftp_modals.filter.as_ref() else {
            return div().into_any_element();
        };
        let textarea = modal.textarea.clone();
        let error = modal.error.clone();

        let content = v_flex()
            .gap_3()
            .child(Textarea::new(&textarea).h(px(220.0)).w_full())
            .child(
                div()
                    .text_xs()
                    .text_color(p.muted_foreground)
                    .child(rust_i18n::t!("sftp.filter_rules_hint").to_string()),
            )
            .when_some(error, |this, err| {
                this.child(div().text_xs().text_color(p.danger).child(err))
            });

        let footer = h_flex()
            .gap_2()
            .child(
                Button::new("cancel-sftp-filter")
                    .ghost()
                    .label(rust_i18n::t!("common.cancel").to_string())
                    .on_click(cx.listener(|this, _, window, cx| {
                        this.sftp_modals.filter = None;
                        this.focus_active_sftp_panel(window, cx);
                        cx.notify();
                    })),
            )
            .child(
                Button::new("save-sftp-filter")
                    .primary()
                    .icon(IconName::Check)
                    .label(rust_i18n::t!("sftp.modals.filter_save").to_string())
                    .on_click(cx.listener(|this, _, window, cx| {
                        this.save_sftp_filter_modal(window, cx);
                    })),
            );

        let dialog = Self::dialog_frame(
            "sftp-filter-modal",
            rust_i18n::t!("sftp.filter_rules").to_string(),
            Some(rust_i18n::t!("sftp.modals.filter_scope_hint").to_string()),
            content,
            Some(footer),
            px(560.0),
            p,
        );

        Self::overlay(dialog, p)
    }

    pub(crate) fn render_sftp_chmod_modal(
        &mut self,
        _window: &mut Window,
        cx: &mut Context<Self>,
        p: &Palette,
    ) -> AnyElement {
        let Some(modal) = self.sftp_modals.chmod.as_ref() else {
            return div().into_any_element();
        };
        let file_name = modal.file_name.clone();
        let remote_path = modal.remote_path.clone();
        let perms = modal.permissions & 0o777;
        let octal_input = modal.octal_input.clone();
        let error = modal.error.clone();

        let groups = [
            (
                rust_i18n::t!("sftp.chmod.group_owner").to_string(),
                (0o400, "chmod-ur"),
                (0o200, "chmod-uw"),
                (0o100, "chmod-ux"),
            ),
            (
                rust_i18n::t!("sftp.chmod.group_group").to_string(),
                (0o040, "chmod-gr"),
                (0o020, "chmod-gw"),
                (0o010, "chmod-gx"),
            ),
            (
                rust_i18n::t!("sftp.chmod.group_others").to_string(),
                (0o004, "chmod-or"),
                (0o002, "chmod-ow"),
                (0o001, "chmod-ox"),
            ),
        ];

        let mut grid_rows: Vec<AnyElement> = Vec::with_capacity(3);
        for (group_title, (r_mask, r_id), (w_mask, w_id), (x_mask, x_id)) in groups {
            grid_rows.push(
                h_flex()
                    .py_2()
                    .px_3()
                    .rounded(p.radius.md)
                    .border_1()
                    .border_color(p.border)
                    .bg(p.muted.opacity(0.4))
                    .items_center()
                    .justify_between()
                    .child(
                        div()
                            .w(px(140.0))
                            .text_sm()
                            .font_weight(FontWeight::SEMIBOLD)
                            .child(group_title),
                    )
                    .child(
                        h_flex()
                            .gap_4()
                            .items_center()
                            .child(
                                Checkbox::new(r_id)
                                    .label(rust_i18n::t!("sftp.chmod.read").to_string())
                                    .checked(perms & r_mask != 0)
                                    .on_change(cx.listener(move |this, &checked, window, cx| {
                                        this.sftp_toggle_chmod_bit(r_mask, checked, window, cx);
                                    })),
                            )
                            .child(
                                Checkbox::new(w_id)
                                    .label(rust_i18n::t!("sftp.chmod.write").to_string())
                                    .checked(perms & w_mask != 0)
                                    .on_change(cx.listener(move |this, &checked, window, cx| {
                                        this.sftp_toggle_chmod_bit(w_mask, checked, window, cx);
                                    })),
                            )
                            .child(
                                Checkbox::new(x_id)
                                    .label(rust_i18n::t!("sftp.chmod.execute").to_string())
                                    .checked(perms & x_mask != 0)
                                    .on_change(cx.listener(move |this, &checked, window, cx| {
                                        this.sftp_toggle_chmod_bit(x_mask, checked, window, cx);
                                    })),
                            ),
                    )
                    .into_any_element(),
            );
        }

        let rwx_str = format_rwx_permissions(perms);
        let presets = [
            (0o755, "755"),
            (0o644, "644"),
            (0o700, "700"),
            (0o600, "600"),
        ];
        let mut preset_btns = h_flex().gap_1p5().items_center();
        for (mode_val, label_str) in presets {
            preset_btns = preset_btns.child(
                Button::new(format!("chmod-preset-{label_str}"))
                    .ghost()
                    .xsmall()
                    .label(label_str)
                    .selected(perms == mode_val)
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.sftp_set_chmod_preset(mode_val, window, cx);
                    })),
            );
        }

        let content = v_flex()
            .gap_3()
            .children(grid_rows)
            .child(
                h_flex()
                    .gap_3()
                    .items_center()
                    .justify_between()
                    .pt_1()
                    .child(
                        h_flex()
                            .gap_2()
                            .items_center()
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(p.muted_foreground)
                                    .child(rust_i18n::t!("sftp.chmod.octal_label").to_string()),
                            )
                            .child(div().w(px(90.0)).child(Input::new(&octal_input).w_full()))
                            .child(
                                div()
                                    .text_xs()
                                    .px_2()
                                    .py_1()
                                    .rounded(p.radius.sm)
                                    .bg(p.muted)
                                    .text_color(p.muted_foreground)
                                    .child(rwx_str),
                            ),
                    )
                    .child(preset_btns),
            )
            .when_some(error, |this, err| {
                this.child(div().text_xs().text_color(p.danger).child(err))
            });

        let footer = h_flex()
            .gap_2()
            .child(
                Button::new("cancel-sftp-chmod")
                    .ghost()
                    .label(rust_i18n::t!("common.cancel").to_string())
                    .on_click(cx.listener(|this, _, window, cx| {
                        this.sftp_modals.chmod = None;
                        this.focus_active_sftp_panel(window, cx);
                        cx.notify();
                    })),
            )
            .child(
                Button::new("save-sftp-chmod")
                    .primary()
                    .icon(IconName::Check)
                    .label(rust_i18n::t!("sftp.chmod.apply").to_string())
                    .on_click(cx.listener(|this, _, window, cx| {
                        this.save_sftp_chmod_modal(window, cx);
                    })),
            );

        let dialog = Self::dialog_frame(
            "sftp-chmod-modal",
            rust_i18n::t!("sftp.chmod.title", name = file_name.as_str()).to_string(),
            Some(rust_i18n::t!("sftp.chmod.remote_path", path = remote_path.as_str()).to_string()),
            content,
            Some(footer),
            px(520.0),
            p,
        );

        Self::overlay(dialog, p)
    }

    pub(crate) fn render_sftp_input_modal(
        &mut self,
        _window: &mut Window,
        cx: &mut Context<Self>,
        p: &Palette,
    ) -> AnyElement {
        let Some(modal) = self.sftp_modals.input.as_ref() else {
            return div().into_any_element();
        };
        let title = modal.title.clone();
        let prompt = modal.prompt.clone();
        let input = modal.input.clone();
        let error = modal.error.clone();

        let content = v_flex()
            .gap_3()
            .child(Input::new(&input).w_full())
            .when_some(error, |this, err| {
                this.child(div().text_xs().text_color(p.danger).child(err))
            });

        let footer = h_flex()
            .gap_2()
            .child(
                Button::new("cancel-sftp-input")
                    .ghost()
                    .label(rust_i18n::t!("common.cancel").to_string())
                    .on_click(cx.listener(|this, _, window, cx| {
                        this.sftp_modals.input = None;
                        this.focus_active_sftp_panel(window, cx);
                        cx.notify();
                    })),
            )
            .child(
                Button::new("confirm-sftp-input")
                    .primary()
                    .icon(IconName::Check)
                    .label(rust_i18n::t!("common.confirm").to_string())
                    .on_click(cx.listener(|this, _, window, cx| {
                        this.commit_sftp_input_modal(window, cx);
                    })),
            );

        let dialog = Self::dialog_frame(
            "sftp-input-modal",
            title,
            Some(prompt),
            content,
            Some(footer),
            px(420.0),
            p,
        );

        Self::overlay(dialog, p)
    }

    pub(crate) fn render_sftp_properties_modal(
        &mut self,
        _window: &mut Window,
        cx: &mut Context<Self>,
        p: &Palette,
    ) -> AnyElement {
        let Some(modal) = self.sftp_modals.properties.as_ref() else {
            return div().into_any_element();
        };
        let scope = modal.scope.clone();
        let properties = modal.properties.clone();
        let name = properties.name.clone();
        let path = properties.path.clone();

        let mut rows: Vec<AnyElement> = Vec::with_capacity(9);
        rows.push(property_row(
            rust_i18n::t!("sftp.properties.label_name").as_ref(),
            name.clone(),
            p,
        ));
        rows.push(property_row(
            rust_i18n::t!("sftp.properties.label_kind").as_ref(),
            properties.kind.clone(),
            p,
        ));
        if let Some(size) = properties.size {
            rows.push(property_row(
                rust_i18n::t!("sftp.properties.label_size").as_ref(),
                rust_i18n::t!(
                    "sftp.properties.size_value",
                    size = format_size(size),
                    bytes = size
                )
                .to_string(),
                p,
            ));
        }
        if let Some(permissions) = properties.permissions {
            rows.push(property_row(
                rust_i18n::t!("sftp.properties.label_permissions").as_ref(),
                format!(
                    "{} ({})",
                    format_rwx_permissions(permissions),
                    format_octal_permissions(permissions)
                ),
                p,
            ));
        }
        if let Some(owner) = properties.owner.clone() {
            rows.push(property_row(
                rust_i18n::t!("sftp.properties.label_owner").as_ref(),
                owner,
                p,
            ));
        }
        if let Some(group) = properties.group.clone() {
            rows.push(property_row(
                rust_i18n::t!("sftp.properties.label_group").as_ref(),
                group,
                p,
            ));
        }
        if let Some(modified) = properties.modified {
            rows.push(property_row(
                rust_i18n::t!("sftp.properties.label_modified").as_ref(),
                format_unix_time(modified),
                p,
            ));
        }
        if let Some(accessed) = properties.accessed {
            rows.push(property_row(
                rust_i18n::t!("sftp.properties.label_accessed").as_ref(),
                format_unix_time(accessed),
                p,
            ));
        }
        if let Some(link_target) = properties.link_target.clone() {
            rows.push(property_row(
                rust_i18n::t!("sftp.properties.label_link_target").as_ref(),
                link_target,
                p,
            ));
        }

        let content = v_flex().gap_1().children(rows);

        let footer = h_flex().gap_2().child(
            Button::new("close-sftp-properties")
                .ghost()
                .icon(IconName::Close)
                .label(rust_i18n::t!("app.close").to_string())
                .on_click(cx.listener(|this, _, window, cx| {
                    this.sftp_modals.properties = None;
                    this.focus_active_sftp_panel(window, cx);
                    cx.notify();
                })),
        );

        let scope_label = if scope == "本地" {
            rust_i18n::t!("sftp.local_title").to_string()
        } else {
            rust_i18n::t!("sftp.modals.scope_remote").to_string()
        };

        let dialog = Self::dialog_frame(
            "sftp-properties-modal",
            rust_i18n::t!("sftp.properties.title", name = name.as_str()).to_string(),
            Some(
                rust_i18n::t!(
                    "sftp.properties.subtitle",
                    scope = scope_label,
                    path = path.as_str()
                )
                .to_string(),
            ),
            content,
            Some(footer),
            px(520.0),
            p,
        );

        Self::overlay(dialog, p)
    }
}

/// 渲染属性弹窗中的单行「标签 - 值」条目
fn property_row(label: &str, value: String, p: &Palette) -> AnyElement {
    h_flex()
        .gap_3()
        .py_1p5()
        .px_3()
        .rounded(p.radius.md)
        .bg(p.muted.opacity(0.3))
        .items_start()
        .child(
            div()
                .w(px(110.0))
                .text_xs()
                .text_color(p.muted_foreground)
                .child(label.to_string()),
        )
        .child(div().flex_1().text_sm().child(value))
        .into_any_element()
}
