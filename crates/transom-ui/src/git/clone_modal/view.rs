//! 克隆弹窗界面渲染与交互。

use super::CONTROL_COLUMN_WIDTH;
use crate::app::AppShell;
use crate::git::dir_browser::{GitHost, render_git_browser};
use crate::git::folder_field::render_git_folder_field;
use crate::palette::Palette;
use crate::session_appearance::{render_appearance_color_row, render_appearance_icon_row};
use gpui_kit::assets::IconName;
use gpui_kit::base::{h_flex, v_flex};
use gpui_kit::component::Sizable;
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::checkbox::Checkbox;
use gpui_kit::component::input::Input;
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::*;

impl AppShell {
    /// 渲染「克隆仓库」弹窗主体（接入 dialog_host）。
    pub(crate) fn render_git_clone_dialog_body(
        &self,
        app: &Entity<AppShell>,
        _window: &mut Window,
        cx: &App,
        p: &Palette,
    ) -> AnyElement {
        let Some(modal) = self.state.modals.git.clone.as_ref() else {
            return div().into_any_element();
        };
        let url = modal.url.clone();
        let depth = modal.depth.clone();
        let folder = modal.folder.clone();
        let error = modal.error.clone();
        let depth_enabled = modal.depth_enabled;
        let tree_open = modal.tree_open;
        let icon_choice = modal.icon;
        let color_choice = modal.color;
        let path_input = modal.picker.path.clone();
        let folders = self.git_folder_names();
        let target = GitHost::Clone;

        let body =
            v_flex()
                .w_full()
                .gap_2()
                .child(crate::widgets::form_field(
                    rust_i18n::t!("git.clone.url_label").to_string(),
                    url,
                    p,
                ))
                .child(
                    v_flex()
                        .gap_1()
                        .child(
                            div()
                                .text_xs()
                                .text_color(p.muted_foreground)
                                .child(rust_i18n::t!("git.clone.dest_label").to_string()),
                        )
                        .child(
                            h_flex()
                                .gap_1()
                                .items_center()
                                .child(
                                    div()
                                        .flex_1()
                                        .min_w_0()
                                        .child(Input::new(&path_input).w_full()),
                                )
                                .child(
                                    Button::new("git-clone-browse")
                                        .ghost()
                                        .small()
                                        .icon(IconName::FolderOpen)
                                        .tooltip(rust_i18n::t!("git.clone.browse").to_string())
                                        .on_click({
                                            let app = app.clone();
                                            move |_, _, cx| {
                                                app.update(cx, |this, cx| {
                                                    if let Some(modal) =
                                                        this.state.modals.git.clone.as_mut()
                                                    {
                                                        modal.tree_open = !modal.tree_open;
                                                    }
                                                    cx.notify();
                                                });
                                            }
                                        }),
                                ),
                        )
                        .when(tree_open, |el| {
                            el.child(render_git_browser(
                                &modal.picker,
                                target,
                                6,
                                false,
                                p,
                                app,
                                cx,
                            ))
                        }),
                )
                .child(
                    h_flex()
                        .w_full()
                        .gap_6()
                        .items_start()
                        .child(
                            v_flex()
                                .flex_1()
                                .min_w_0()
                                .gap_1()
                                .child(
                                    div()
                                        .text_xs()
                                        .text_color(p.muted_foreground)
                                        .child(rust_i18n::t!("form.icon").to_string()),
                                )
                                .child(div().w(px(CONTROL_COLUMN_WIDTH)).child(
                                    render_appearance_icon_row(
                                        "git-clone",
                                        IconName::GitBranch,
                                        icon_choice,
                                        move |this: &mut AppShell, choice, cx| {
                                            if let Some(modal) =
                                                this.state.modals.git.clone.as_mut()
                                            {
                                                modal.icon = choice;
                                            }
                                            cx.notify();
                                        },
                                        app,
                                        cx,
                                    ),
                                )),
                        )
                        .child(
                            v_flex()
                                .flex_1()
                                .min_w_0()
                                .gap_1()
                                .child(
                                    div()
                                        .text_xs()
                                        .text_color(p.muted_foreground)
                                        .child(rust_i18n::t!("form.color").to_string()),
                                )
                                .child(render_appearance_color_row(
                                    "git-clone",
                                    color_choice,
                                    move |this: &mut AppShell, choice, cx| {
                                        if let Some(modal) = this.state.modals.git.clone.as_mut() {
                                            modal.color = choice;
                                        }
                                        cx.notify();
                                    },
                                    p,
                                    app,
                                    cx,
                                )),
                        ),
                )
                .child(
                    h_flex()
                        .w_full()
                        .gap_6()
                        .items_start()
                        .child(div().flex_1().min_w_0().child(render_git_folder_field(
                            &folder,
                            target,
                            &folders,
                            Some(CONTROL_COLUMN_WIDTH),
                            p,
                            app,
                            cx,
                        )))
                        .child(
                            v_flex().flex_1().min_w_0().pt(px(20.0)).child(
                                h_flex()
                                    .gap_2()
                                    .items_center()
                                    .child(
                                        Checkbox::new("git-clone-shallow")
                                            .small()
                                            .checked(depth_enabled)
                                            .on_click({
                                                let app = app.clone();
                                                move |&checked, _, cx| {
                                                    app.update(cx, |this, cx| {
                                                        if let Some(modal) =
                                                            this.state.modals.git.clone.as_mut()
                                                        {
                                                            modal.depth_enabled = checked;
                                                        }
                                                        cx.notify();
                                                    });
                                                }
                                            }),
                                    )
                                    .child(div().text_xs().child(
                                        rust_i18n::t!("git.clone.shallow_before").to_string(),
                                    ))
                                    .child(div().w(px(56.0)).child(Input::new(&depth).w_full()))
                                    .child(div().text_xs().child(
                                        rust_i18n::t!("git.clone.shallow_after").to_string(),
                                    )),
                            ),
                        ),
                )
                .when(error.is_some(), |el| {
                    el.child(
                        div()
                            .text_xs()
                            .text_color(p.danger)
                            .child(error.unwrap_or_default()),
                    )
                });

        let footer = h_flex()
            .w_full()
            .gap_2()
            .justify_end()
            .child(crate::widgets::dialog_cancel_button(
                "git-clone-cancel",
                rust_i18n::t!("git.clone.cancel").to_string(),
                crate::dialog_host::on_event(app, |this, _: &ClickEvent, _, cx| {
                    this.close_git_clone_modal(cx);
                }),
            ))
            .child(crate::widgets::dialog_confirm_button(
                "git-clone-start",
                rust_i18n::t!("git.clone.start").to_string(),
                Some(IconName::Download),
                crate::widgets::DialogConfirmKind::Primary,
                crate::dialog_host::on_event(app, |this, _: &ClickEvent, window, cx| {
                    this.start_git_clone(window, cx);
                }),
            ));

        crate::widgets::dialog_body(
            "git-clone-dialog",
            rust_i18n::t!("git.clone.title").to_string(),
            None,
            body,
            Some(footer),
        )
    }
}
