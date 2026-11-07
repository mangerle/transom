//! 提交页底部表单：修正选项、已选统计、多行消息输入与提交/推送动作。
//!
//! 设计原理（Why）：
//! - 将提交输入表单与改动树分层解耦，保持 `commit_page.rs` 紧凑且单文件不超 500 行；
//! - 输入框自适应父面板高度，配合外部 `v_resizable` 实现上下自由拖拽伸缩。

use crate::app::AppShell;
use crate::palette::Palette;
use crate::widgets::menu_item;
use gpui_kit::assets::IconName;
use gpui_kit::base::{h_flex, v_flex};
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::input::{Textarea, TextareaState};
use gpui_kit::component::menu::{DropdownMenu, PopupMenuItem};
use gpui_kit::component::{Disableable, Sizable};
use gpui_kit::*;
use transom_core::TabId;

/// 提交输入表单属性包。
pub(crate) struct CommitBoxProps<'a> {
    pub(crate) tab_id: TabId,
    pub(crate) amend_toggle: AnyElement,
    pub(crate) amend_target: AnyElement,
    pub(crate) checked_count: usize,
    pub(crate) message: &'a Entity<TextareaState>,
    pub(crate) submitting: bool,
}

impl AppShell {
    /// 渲染提交说明与动作面板（可作为上下切分的下半部分）。
    pub(crate) fn render_commit_box(
        &mut self,
        props: CommitBoxProps<'_>,
        p: &Palette,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let tab_id = props.tab_id;
        let submitting = props.submitting;

        let history: Vec<String> = self
            .git_state_of_tab(tab_id)
            .map(|s| {
                s.log
                    .entries
                    .iter()
                    .map(|e| e.subject.clone())
                    .filter(|subj| !subj.trim().is_empty())
                    .fold(Vec::new(), |mut acc, item| {
                        if !acc.contains(&item) && acc.len() < 10 {
                            acc.push(item);
                        }
                        acc
                    })
            })
            .unwrap_or_default();

        let recent_btn = Button::new("git-commit-recent-messages")
            .ghost()
            .xsmall()
            .icon(IconName::Clock)
            .tooltip(rust_i18n::t!("git.commit.recent_messages").to_string())
            .dropdown_menu({
                let message_input = props.message.clone();
                move |mut menu, _window, _cx| {
                    if history.is_empty() {
                        menu = menu.item(
                            PopupMenuItem::new(
                                rust_i18n::t!("git.commit.no_recent_messages").to_string(),
                            )
                            .disabled(true),
                        );
                    } else {
                        for text in &history {
                            let text_clone = text.clone();
                            let input = message_input.clone();
                            menu = menu.item(menu_item(
                                text.clone(),
                                IconName::Clock,
                                move |_event, window, cx| {
                                    input.update(cx, |input, cx| {
                                        input.set_value(&text_clone, window, cx);
                                    });
                                },
                            ));
                        }
                    }
                    menu
                }
            });

        v_flex()
            .w_full()
            .h_full()
            .p_2()
            .gap_2()
            .border_color(p.divider(p.border))
            .border_t_1()
            .child(
                h_flex()
                    .w_full()
                    .gap_2()
                    .items_center()
                    .child(props.amend_toggle)
                    .child(props.amend_target)
                    .child(div().flex_1())
                    .child(recent_btn)
                    .child(
                        div().text_xs().text_color(p.primary).child(
                            rust_i18n::t!("git.commit.checked_count", count = props.checked_count)
                                .to_string(),
                        ),
                    ),
            )
            .child(
                div()
                    .w_full()
                    .flex_1()
                    .min_h(px(80.0))
                    .child(Textarea::new(props.message).size_full()),
            )
            .child(
                h_flex()
                    .w_full()
                    .gap_2()
                    .items_center()
                    .child(
                        Button::new("git-commit-submit")
                            .primary()
                            .small()
                            .label(rust_i18n::t!("git.commit.submit").to_string())
                            .disabled(submitting)
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.submit_git_commit(tab_id, false, cx);
                            })),
                    )
                    .child(
                        Button::new("git-commit-and-push")
                            .primary()
                            .small()
                            .icon(IconName::ArrowUpFromLine)
                            .label(rust_i18n::t!("git.commit.and_push").to_string())
                            .disabled(submitting)
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.submit_git_commit(tab_id, true, cx);
                            })),
                    )
                    .child(div().flex_1())
                    .child(
                        Button::new("git-commit-settings")
                            .ghost()
                            .small()
                            .icon(IconName::Settings)
                            .tooltip(rust_i18n::t!("git.commit.settings").to_string())
                            .on_click(cx.listener(move |this, _, window, cx| {
                                this.open_git_settings(window, cx);
                            })),
                    ),
            )
            .into_any_element()
    }
}
