//! 克隆弹窗界面渲染与交互。

use super::{CONTROL_COLUMN_WIDTH, GitCloneModalEvent, GitCloneModalView};
use crate::git::dir_browser::render_git_browser;
use crate::git::folder_field::{GitFolderFieldProps, render_git_folder_field};
use crate::palette::Palette;
use crate::session_appearance::{render_appearance_color_row, render_appearance_icon_row};
use gpui_kit::base::{h_flex, v_flex};
use gpui_kit::component::Sizable;
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::checkbox::Checkbox;
use gpui_kit::component::input::Input;
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::*;
use transom_ui_icons::AppIcon;
use transom_ui_icons::AppIconElement;
use transom_ui_icons::UiIcon;

impl Render for GitCloneModalView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let p = Palette::from(cx);
        let view = cx.entity().clone();
        let path_input = self.picker.read(cx).path.clone();
        let error = self.error.clone();

        let body = v_flex()
            .w_full()
            .gap_2()
            .child(crate::widgets::form_field(
                rust_i18n::t!("git.clone.url_label").to_string(),
                self.url.clone(),
                &p,
            ))
            .child(self.render_destination_section(path_input, &p, cx))
            .child(self.render_appearance_section(&view, &p, cx))
            .child(self.render_folder_and_shallow_row(&view, &p, cx))
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
                cx.listener(|_, _: &ClickEvent, _, cx| {
                    cx.emit(GitCloneModalEvent::Close);
                }),
            ))
            .child(crate::widgets::dialog_confirm_button(
                "git-clone-start",
                rust_i18n::t!("git.clone.start").to_string(),
                Some(AppIcon::Ui(UiIcon::DOWNLOAD)),
                crate::widgets::DialogConfirmKind::Primary,
                cx.listener(|this, _: &ClickEvent, _, cx| {
                    this.submit(cx);
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

impl GitCloneModalView {
    fn render_destination_section(
        &self,
        path_input: Entity<gpui_kit::component::input::InputState>,
        p: &Palette,
        cx: &mut Context<Self>,
    ) -> AnyElement {
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
                            .child(AppIconElement::new(AppIcon::Ui(UiIcon::FOLDER)).small())
                            .accessibility_label(rust_i18n::t!("git.clone.browse").to_string())
                            .tooltip(rust_i18n::t!("git.clone.browse").to_string())
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.tree_open = !this.tree_open;
                                cx.notify();
                            })),
                    ),
            )
            .when(self.tree_open, |el| {
                el.child(render_git_browser(&self.picker, 6, false, p, cx))
            })
            .into_any_element()
    }

    fn render_appearance_section(&self, view: &Entity<Self>, p: &Palette, cx: &App) -> AnyElement {
        v_flex()
            .w_full()
            .gap_2()
            .child(
                v_flex()
                    .w_full()
                    .gap_1()
                    .child(
                        div()
                            .text_xs()
                            .text_color(p.muted_foreground)
                            .child(rust_i18n::t!("form.icon").to_string()),
                    )
                    .child(render_appearance_icon_row(
                        "git-clone",
                        self.icon,
                        move |this: &mut Self, choice, cx| {
                            this.icon = choice;
                            cx.notify();
                        },
                        view,
                        cx,
                    )),
            )
            .child(
                v_flex()
                    .w_full()
                    .gap_1()
                    .child(
                        div()
                            .text_xs()
                            .text_color(p.muted_foreground)
                            .child(rust_i18n::t!("form.color").to_string()),
                    )
                    .child(render_appearance_color_row(
                        "git-clone",
                        self.color,
                        move |this: &mut Self, choice, cx| {
                            this.color = choice;
                            cx.notify();
                        },
                        p,
                        view,
                        cx,
                    )),
            )
            .into_any_element()
    }

    fn render_folder_and_shallow_row(
        &self,
        view: &Entity<Self>,
        p: &Palette,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        h_flex()
            .w_full()
            .gap_6()
            .items_start()
            .child(div().flex_1().min_w_0().child(render_git_folder_field(
                &self.folder,
                GitFolderFieldProps {
                    options: &self.folders,
                    fixed_width: Some(CONTROL_COLUMN_WIDTH),
                    palette: p,
                    entity: view,
                    access: |this| &mut this.folder,
                },
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
                                .checked(self.depth_enabled)
                                .on_click(cx.listener(|this, &checked, _, cx| {
                                    this.depth_enabled = checked;
                                    cx.notify();
                                })),
                        )
                        .child(
                            div()
                                .text_xs()
                                .child(rust_i18n::t!("git.clone.shallow_before").to_string()),
                        )
                        .child(div().w(px(56.0)).child(Input::new(&self.depth).w_full()))
                        .child(
                            div()
                                .text_xs()
                                .child(rust_i18n::t!("git.clone.shallow_after").to_string()),
                        ),
                ),
            )
            .into_any_element()
    }
}
