//! 终端右键菜单与上下文动作。

use super::TerminalView;
use gpui_kit::assets::IconName;
use gpui_kit::component::menu::{ContextMenuExt, PopupMenuItem};
use gpui_kit::*;
use transom_core::RightClickBehavior;

impl TerminalView {
    /// 为终端层挂载上下文菜单（或直接执行粘贴）。
    pub(super) fn wrap_context_menu(
        layer: Div,
        menu_entity: Entity<Self>,
        right_click: RightClickBehavior,
    ) -> AnyElement {
        // 只有菜单档位才挂 ContextMenuExt：Paste 档位右键必须直接粘贴，
        // 挂上菜单会让一次右键同时粘贴并弹出菜单。
        if right_click == RightClickBehavior::Paste {
            layer.into_any_element()
        } else {
            layer
                .context_menu(move |menu, _window, _cx| {
                    let mut menu = menu;
                    if right_click == RightClickBehavior::MenuWithPaste {
                        let app = menu_entity.clone();
                        menu = menu
                            .item(
                                PopupMenuItem::new("粘贴")
                                    .icon(IconName::ClipboardPaste)
                                    .on_click(move |_, _, cx| {
                                        app.update(cx, |view, cx| {
                                            view.paste_from_clipboard(cx);
                                        });
                                    }),
                            )
                            .item(PopupMenuItem::separator());
                    }
                    let app = menu_entity.clone();
                    menu.item(
                        PopupMenuItem::new("切换语义块视图 / 经典连续流")
                            .icon(IconName::ListTree)
                            .on_click(move |_, _, cx| {
                                app.update(cx, |view, cx| {
                                    view.toggle_block_view(cx);
                                });
                            }),
                    )
                })
                .into_any_element()
        }
    }
}
