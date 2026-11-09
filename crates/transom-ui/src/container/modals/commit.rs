//! 容器提交为新镜像居中模态弹窗。

use crate::container::{ContainerWorkspace, ContainerWorkspaceState};
use crate::palette::Palette;
use crate::widgets::{
    DialogConfirmKind, dialog_actions, dialog_body, dialog_cancel_button, dialog_confirm_button,
};
use gpui_kit::assets::IconName;
use gpui_kit::base::v_flex;
use gpui_kit::component::input::Input;
use gpui_kit::*;

/// 渲染容器提交镜像模态弹窗主体。
///
/// 设计原理（Why）：三个输入控件与打开状态都归工作区实体，渲染只读状态快照；
/// 取消与确认回调直接 `update` 实体，不再经外壳转发。
pub(crate) fn render_container_commit_dialog_body(
    view: &Entity<ContainerWorkspace>,
    state: &ContainerWorkspaceState,
    p: &Palette,
) -> AnyElement {
    let Some(commit) = state.commit_container.clone() else {
        return div().into_any_element();
    };
    let repo_input = commit.repo;
    let tag_input = commit.tag;
    let comment_input = commit.comment;

    let content = v_flex()
        .gap_3()
        .child(
            v_flex()
                .gap_1()
                .child(
                    div()
                        .text_xs()
                        .font_weight(FontWeight::MEDIUM)
                        .text_color(p.muted_foreground)
                        .child(rust_i18n::t!("container.commit.repo_label").to_string()),
                )
                .child(Input::new(&repo_input).w_full()),
        )
        .child(
            v_flex()
                .gap_1()
                .child(
                    div()
                        .text_xs()
                        .font_weight(FontWeight::MEDIUM)
                        .text_color(p.muted_foreground)
                        .child(rust_i18n::t!("container.commit.tag_label").to_string()),
                )
                .child(Input::new(&tag_input).w_full()),
        )
        .child(
            v_flex()
                .gap_1()
                .child(
                    div()
                        .text_xs()
                        .font_weight(FontWeight::MEDIUM)
                        .text_color(p.muted_foreground)
                        .child(rust_i18n::t!("container.commit.comment_label").to_string()),
                )
                .child(Input::new(&comment_input).w_full()),
        );

    let cancel = dialog_cancel_button(
        "cancel-container-commit",
        rust_i18n::t!("common.cancel").to_string(),
        {
            let view = view.clone();
            move |_, _, cx| {
                view.update(cx, |workspace, cx| workspace.cancel_container_commit(cx));
            }
        },
    );

    let confirm = dialog_confirm_button(
        "confirm-container-commit",
        rust_i18n::t!("common.confirm").to_string(),
        Some(IconName::Layers),
        DialogConfirmKind::Primary,
        {
            let view = view.clone();
            move |_, _, cx| {
                view.update(cx, |workspace, cx| workspace.submit_container_commit(cx));
            }
        },
    );

    let footer = dialog_actions([cancel.into_any_element(), confirm.into_any_element()]);

    dialog_body(
        "container-commit-modal",
        rust_i18n::t!("container.commit.modal_title").to_string(),
        Some(rust_i18n::t!("container.commit.prompt").to_string()),
        content,
        Some(footer),
    )
}
