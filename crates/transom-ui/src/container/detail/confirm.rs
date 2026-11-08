//! 危险操作确认条。

use super::*;
use crate::container::ContainerWorkspace;

/// 删除按钮：把待确认动作写回工作区实体，确认弹窗仍由外壳统一渲染。
///
/// 设计原理（Why）：删除确认只是「设置待确认项」，完全属于工作区实体状态；
/// 渲染函数持有实体句柄即可完成，不需要外壳上下文，也不会在点击回调里反查外壳。
pub(crate) fn delete_button(
    view: &Entity<ContainerWorkspace>,
    id: &'static str,
    confirm: ContainerConfirm,
) -> AnyElement {
    let view = view.clone();
    Button::new(id)
        .danger()
        .xsmall()
        .icon(IconName::Trash)
        .label(rust_i18n::t!("common.delete").to_string())
        .on_click(move |_, _, cx| {
            let confirm = confirm.clone();
            view.update(cx, |workspace, cx| {
                workspace.state.confirm = Some(confirm);
                cx.notify();
            });
        })
        .into_any_element()
}
