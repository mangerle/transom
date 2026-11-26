//! 端口转发面板与公网监听确认弹窗视图渲染。

use super::ForwardView;
use crate::palette::Palette;
use gpui_kit::base::v_flex;
use gpui_kit::*;
use transom_ui_icons::AppIcon;
use transom_ui_icons::DomainIcon;
use transom_ui_icons::UiIcon;

impl ForwardView {
    pub(crate) fn render_forward_panel(
        &mut self,
        _window: &mut Window,
        cx: &mut Context<Self>,
        p: &Palette,
    ) -> AnyElement {
        // 规则与状态由外壳注入：渲染期不反查外壳。
        let tab_id = self.tab_id();
        let forwards = self.rules().to_vec();
        let statuses = self.statuses().clone();

        let header = self.render_forward_header(cx, p);
        let rules_list = self.render_forward_rules_list(tab_id, &forwards, &statuses, cx, p);
        let add_form = self.render_forward_add_form(tab_id, cx, p);

        v_flex()
            .size_full()
            .overflow_hidden()
            .bg(p.background)
            .text_color(p.foreground)
            .child(header)
            .child(rules_list)
            .child(add_form)
            .into_any_element()
    }

    /// 端口转发面板顶栏标题与关闭按钮。
    ///
    /// 高度取标签栏 token 而不是靠内边距撑开：该面板与主区标签条、监控抽屉
    /// 并排，三条横线必须在同一水平线上。
    fn render_forward_header(&self, cx: &mut Context<Self>, p: &Palette) -> AnyElement {
        let close = crate::widgets::IconButton::new(
            "close-forward",
            AppIcon::Ui(UiIcon::CLOSE),
            rust_i18n::t!("forward.close_panel").to_string(),
        )
        .size(crate::widgets::IconButtonSize::Compact)
        .render()
        .on_click(cx.listener(|_this, _, _, cx| {
            // 句柄由窗格持有，回收动作回交外壳；实体只发请求。
            cx.emit(super::ForwardPanelEvent::ClosePanel);
            cx.notify();
        }));
        crate::widgets::PanelHeader::new(
            format!("forward-panel-header-{}", self.tab_id()),
            rust_i18n::t!("forward.title").to_string(),
            p,
        )
        .action(close)
        .render(p)
    }

    /// 端口转发规则列表渲染。
    fn render_forward_rules_list(
        &self,
        tab_id: transom_core::TabId,
        rules: &[transom_core::ForwardRule],
        statuses: &std::collections::HashMap<transom_core::ForwardId, transom_core::ForwardStatus>,
        cx: &mut Context<Self>,
        p: &Palette,
    ) -> AnyElement {
        let mut rule_rows: Vec<AnyElement> = Vec::new();
        if rules.is_empty() {
            rule_rows.push(
                div()
                    .py_8()
                    .px_3()
                    .child(crate::widgets::empty_state(
                        AppIcon::Domain(DomainIcon::FORWARD),
                        rust_i18n::t!("forward.empty").to_string(),
                        rust_i18n::t!("forward.empty_description").to_string(),
                        p,
                    ))
                    .into_any_element(),
            );
        }
        for rule in rules {
            rule_rows.push(self.render_forward_rule_row(rule, statuses, tab_id, cx, p));
        }

        v_flex()
            .id(ElementId::Name("forward-rules".into()))
            .w_full()
            .flex_1()
            .overflow_y_scroll()
            .p_2()
            .gap_2()
            .children(rule_rows)
            .into_any_element()
    }
}
