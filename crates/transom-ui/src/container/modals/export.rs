//! 容器配置反向导出查看居中模态弹窗。

use crate::container::{ContainerWorkspace, ContainerWorkspaceState};
use crate::palette::Palette;
use crate::widgets::{
    DialogConfirmKind, dialog_actions, dialog_body, dialog_cancel_button, dialog_confirm_button,
};
use gpui_kit::ClipboardItem;
use gpui_kit::base::{Selectable, h_flex, v_flex};
use gpui_kit::component::Sizable;
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::*;
use transom_ui_icons::AppIcon;
use transom_ui_icons::UiIcon;

/// 渲染容器配置反向导出模态弹窗主体。
///
/// 设计原理（Why）：导出内容与格式开关都归工作区实体，渲染只读状态快照；
/// 复制按钮在本帧读取文本后写入剪贴板，结果提示经实体事件交外壳弹出。
pub(crate) fn render_container_export_config_dialog_body(
    view: &Entity<ContainerWorkspace>,
    state: &ContainerWorkspaceState,
    p: &Palette,
) -> AnyElement {
    let Some((name, run_cmd, compose_yaml, is_compose)) = state.export_config.clone() else {
        return div().into_any_element();
    };

    let title = rust_i18n::t!("container.export.title", name = name).to_string();
    let display_content = if is_compose {
        compose_yaml.clone()
    } else {
        run_cmd.clone()
    };

    let mode_tabs = h_flex()
        .gap_1()
        .items_center()
        .child(
            Button::new("container-export-tab-run")
                .xsmall()
                .ghost()
                .selected(!is_compose)
                .label(rust_i18n::t!("container.export.run_tab").to_string())
                .on_click({
                    let view = view.clone();
                    move |_, _, cx| {
                        view.update(cx, |workspace, cx| {
                            workspace.toggle_container_export_mode(false, cx)
                        });
                    }
                }),
        )
        .child(
            Button::new("container-export-tab-compose")
                .xsmall()
                .ghost()
                .selected(is_compose)
                .label(rust_i18n::t!("container.export.compose_tab").to_string())
                .on_click({
                    let view = view.clone();
                    move |_, _, cx| {
                        view.update(cx, |workspace, cx| {
                            workspace.toggle_container_export_mode(true, cx)
                        });
                    }
                }),
        );

    let code_box = div()
        .id("container-export-modal-scroll")
        .w_full()
        .max_h(px(320.0))
        .overflow_y_scroll()
        .p_2p5()
        .rounded(p.radius.md)
        .bg(p.surface.hover)
        .border_1()
        .border_color(p.border)
        .text_xs()
        .font_family(".SystemUIFont")
        .text_color(p.foreground)
        .child(display_content.clone());

    let content = v_flex().gap_3().child(mode_tabs).child(code_box);

    let copy_btn = dialog_confirm_button(
        "copy-container-export",
        rust_i18n::t!("common.copy").to_string(),
        Some(AppIcon::Ui(UiIcon::COPY)),
        DialogConfirmKind::Primary,
        {
            let view = view.clone();
            let copy_text = display_content.clone();
            move |_, _, cx| {
                cx.write_to_clipboard(ClipboardItem::new_string(copy_text.clone()));
                view.update(cx, |_, cx| {
                    cx.emit(crate::container::ContainerWorkspaceEvent::ShowToast {
                        message: rust_i18n::t!("container.export.copied").to_string(),
                        success: true,
                    });
                });
            }
        },
    );

    let close_btn = dialog_cancel_button(
        "close-container-export",
        rust_i18n::t!("common.close").to_string(),
        {
            let view = view.clone();
            move |_, _, cx| {
                view.update(cx, |workspace, cx| {
                    workspace.cancel_container_export(cx);
                });
            }
        },
    );

    let footer = dialog_actions([close_btn.into_any_element(), copy_btn.into_any_element()]);

    dialog_body(
        "container-export-modal",
        title,
        None::<String>,
        content,
        Some(footer),
    )
}
