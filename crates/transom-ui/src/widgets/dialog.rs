//! 弹窗内容骨架与页脚动作容器。

use crate::theme::tokens::space;
use gpui_kit::base::{h_flex, v_flex};
use gpui_kit::component::dialog::{
    DialogContent, DialogDescription, DialogFooter, DialogHeader, DialogTitle,
};
use gpui_kit::*;

/// 构造弹窗内容：标题、说明、正文与可选页脚。
///
/// 表面、边框、阴影、宽度与超高滚动由组件库 Dialog 容器负责；这里只保留
/// 纵向结构，避免页面在容器里再套一层自绘外框。
pub(crate) fn dialog_body(
    id: impl Into<ElementId>,
    title: impl Into<String>,
    description: Option<String>,
    content: impl IntoElement,
    footer: Option<impl IntoElement>,
) -> AnyElement {
    let mut header = DialogHeader::new().child(DialogTitle::new().child(title.into()));
    if let Some(desc) = description {
        header = header.child(DialogDescription::new().child(desc));
    }

    let mut frame = v_flex()
        .id(id.into())
        .w_full()
        .gap(rems(space::LG))
        .child(header)
        .child(DialogContent::new().child(content));

    if let Some(footer) = footer {
        frame = frame.child(DialogFooter::new().child(footer));
    }
    frame.into_any_element()
}

/// 弹窗页脚动作条：右对齐、统一间距。
pub(crate) fn dialog_actions(actions: impl IntoIterator<Item = AnyElement>) -> AnyElement {
    h_flex()
        .w_full()
        .gap(rems(space::SM))
        .items_center()
        .justify_end()
        .children(actions)
        .into_any_element()
}
