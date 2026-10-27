//! 标签栏渲染模块入口。

mod badges;
mod item;
mod menu;

use crate::app::AppShell;
use crate::palette::Palette;
use crate::theme::tokens::size;
use gpui_kit::assets::IconName;
use gpui_kit::base::h_flex;
use gpui_kit::*;

impl AppShell {
    /// 渲染主工作区顶部标签栏。
    pub(super) fn render_tab_strip(&mut self, p: &Palette, cx: &mut Context<Self>) -> AnyElement {
        let mut tab_strip = h_flex()
            .h(rems(size::TAB_BAR_HEIGHT))
            .items_center()
            .bg(p.tab_bar)
            .border_b_1()
            .border_color(p.border)
            .overflow_hidden();
        let app_entity = cx.entity().clone();
        // 并排菜单在每个标签上都显示：条目不足两个时置灰，避免用户以为功能
        // 不存在；右键标签时由 `tile_tab` 取标签栏左边那一条作搭档，左边是
        // 并排合并条目则整组加入，不会把网格里的成员拆出来。
        let entries = self.strip_entries();
        let can_tile = entries.len() >= 2;

        for entry in &entries {
            if let Some(element) = item::render_tab_entry(self, entry, can_tile, p, &app_entity, cx)
            {
                tab_strip = tab_strip.child(element);
            }
        }

        tab_strip
            .child(
                div()
                    .h_full()
                    .px_1p5()
                    .flex()
                    .items_center()
                    .child(badges::tab_strip_button(
                        "tab-strip-quick-new",
                        IconName::Plus,
                        rust_i18n::t!("app.new_local").to_string(),
                        true,
                        p,
                        cx.listener(|this, _, window, cx| {
                            this.open_local_terminal(window, cx);
                        }),
                    )),
            )
            .into_any_element()
    }
}
