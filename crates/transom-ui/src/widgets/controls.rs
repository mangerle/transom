//! 通用设置控件：数字步进器、下拉选择与路径值。
//!
//! 设计原理（Why）：这些控件原本挂在 `AppShell`/`SettingsView` 的固有方法上，
//! 与实体生命周期无关，却让会话表单等调用方必须知道“哪个实体提供了下拉”。
//! 抽成只接收数据与回调的组件后，设置页、会话表单与后续任何表单共用同一份
//! 焦点、菜单与宽度行为；字号、间距全部取设计令牌，密度切换一致。
//!
//! 代价与局限：回调仍通过 `Entity<T>` 在点击时取回可写上下文，调用方要自行
//! 保证实体在菜单关闭前仍存活（应用内所有调用点都满足）。

use crate::palette::Palette;
use crate::theme::tokens::typography;
use gpui_kit::component::Sizable;
use gpui_kit::component::button::Button;
use gpui_kit::component::input::{InputState, NumberInput};
use gpui_kit::component::menu::{DropdownMenu, PopupMenuItem};
use gpui_kit::component::tooltip::Tooltip;
use gpui_kit::*;

/// 路径型值的定宽上限（rem，标准档 192px）：够看清目录层级，又不把控件挤出面板。
const PATH_VALUE_MAX_WIDTH: f32 = 12.0;

/// 数字步进器：复用组件库 `NumberInput` 的加减按钮与键盘步进。
pub(crate) fn number_stepper(input: &Entity<InputState>, width: Pixels) -> AnyElement {
    div()
        .w(width)
        .child(NumberInput::new(input).small())
        .into_any_element()
}

/// 下拉选择器：触发按钮展示当前项，菜单项用 `checked` 标出选中态。
pub(crate) fn dropdown<E, V, F>(
    id: impl Into<ElementId>,
    current_value: V,
    options: Vec<(V, String)>,
    on_change: F,
    entity: Entity<E>,
) -> AnyElement
where
    E: 'static,
    V: Clone + PartialEq + 'static,
    F: Fn(&mut E, V, &mut Window, &mut Context<E>) + 'static + Clone,
{
    let current_label = options
        .iter()
        .find(|(val, _)| val == &current_value)
        .map(|(_, label)| label.clone())
        .unwrap_or_default();

    Button::new(id)
        .small()
        .outline()
        .dropdown_caret(true)
        .label(current_label)
        .dropdown_menu_with_anchor(Anchor::TopRight, {
            let options = options.clone();
            let current_value = current_value.clone();
            move |mut menu, _window, _cx| {
                for (val, label) in &options {
                    let is_selected = val == &current_value;
                    let val = val.clone();
                    let entity = entity.clone();
                    let on_change = on_change.clone();
                    menu = menu.item(
                        PopupMenuItem::new(label.clone())
                            .checked(is_selected)
                            .on_click(move |_, window, cx| {
                                let val = val.clone();
                                let on_change = on_change.clone();
                                entity.update(cx, |this, cx| {
                                    on_change(this, val, window, cx);
                                });
                            }),
                    );
                }
                menu
            }
        })
        .into_any_element()
}

/// 路径型只读值：定宽截断，完整路径挂在悬停 tooltip 上。
///
/// 为什么定宽：配置目录、下载目录、审计目录都可能是任意长路径，按内容排版
/// 会把整行撑爆；截断后用户看不出“到底写到哪个目录”，所以全文挂 tooltip。
pub(crate) fn path_value(element_key: &'static str, label: String, p: &Palette) -> AnyElement {
    let tip = label.clone();
    div()
        .id(ElementId::Name(
            format!("setting-path-{element_key}").into(),
        ))
        .max_w(rems(PATH_VALUE_MAX_WIDTH))
        .truncate()
        .text_size(rems(typography::CAPTION))
        .text_color(p.muted_foreground)
        .tooltip(move |_window, cx| Tooltip::new(tip.clone()).build(_window, cx))
        .child(label)
        .into_any_element()
}
