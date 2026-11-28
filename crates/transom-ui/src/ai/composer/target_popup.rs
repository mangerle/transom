//! AI 面板目标会话选择器弹层：支持跟随标签页、仅全局、锁定主会话与多机协同勾选。

use std::collections::HashSet;

use gpui_kit::base::{h_flex, v_flex};
use gpui_kit::component::scroll::ScrollableElement;
use gpui_kit::*;
use transom_core::{SessionId, TabId};
use transom_ui_icons::{AppIcon, AppIconElement, CustomIcon, DomainIcon, UiIcon};

use crate::app::ai::mention_menu::{AiMentionCandidate, AiMentionKind};
use crate::app::ai::state::{ActiveAiSessionInfo, AiTargetMode};
use crate::palette::Palette;
use crate::theme::tokens::{space, typography};

/// 目标会话选择器弹层渲染参数。
pub(in crate::app::ai) struct TargetSessionPopupParams<'a> {
    pub(in crate::app::ai) target_mode: AiTargetMode,
    pub(in crate::app::ai) workspace_active_session: Option<&'a ActiveAiSessionInfo>,
    pub(in crate::app::ai) active_session: Option<&'a ActiveAiSessionInfo>,
    pub(in crate::app::ai) connected_sessions: &'a [ActiveAiSessionInfo],
    pub(in crate::app::ai) extra_scope_tabs: &'a HashSet<TabId>,
    pub(in crate::app::ai) extra_scope_sessions: &'a HashSet<SessionId>,
    pub(in crate::app::ai) mention_candidates: &'a [AiMentionCandidate],
    pub(in crate::app::ai) p: &'a Palette,
}

/// 目标会话选择器弹层的单参数交互回调。
type PopupCallback<T> = Box<dyn Fn(T, &mut Window, &mut App)>;

/// 目标会话选择器弹层交互回调集合。
pub(in crate::app::ai) struct TargetSessionPopupActions {
    pub(in crate::app::ai) on_select_mode: PopupCallback<AiTargetMode>,
    pub(in crate::app::ai) on_toggle_coop: PopupCallback<TabId>,
    pub(in crate::app::ai) on_toggle_coop_session: PopupCallback<SessionId>,
    pub(in crate::app::ai) on_insert_mention: PopupCallback<String>,
    pub(in crate::app::ai) on_connect_and_pin: PopupCallback<SessionId>,
}

fn render_coop_toggle_btn(
    id: impl Into<ElementId>,
    selected: bool,
    p: &Palette,
    on_click: impl Fn(&mut Window, &mut App) + 'static,
) -> AnyElement {
    let coop_label = rust_i18n::t!("ai.target_coop_btn").to_string();
    let mut btn = h_flex()
        .id(id)
        .px(px(6.0))
        .py(px(2.0))
        .rounded(p.radius.sm)
        .border_1()
        .cursor_pointer()
        .text_size(rems(typography::CAPTION))
        .on_click(move |_, w, cx| {
            cx.stop_propagation();
            on_click(w, cx);
        })
        .child(coop_label);
    btn = if selected {
        btn.bg(p.tone.primary_subtle)
            .border_color(p.primary.opacity(0.5))
            .text_color(p.primary)
            .font_weight(FontWeight::MEDIUM)
    } else {
        btn.border_color(p.divider(p.border))
            .text_color(p.text.muted)
            .hover(|s| s.bg(p.surface.hover).text_color(p.foreground))
    };
    btn.into_any_element()
}

fn section_title(label: String, p: &Palette) -> AnyElement {
    div()
        .px(rems(space::SM))
        .pt(px(4.0))
        .pb(px(2.0))
        .text_size(rems(typography::CAPTION))
        .font_weight(FontWeight::SEMIBOLD)
        .text_color(p.text.muted)
        .child(label)
        .into_any_element()
}

fn render_mode_row(
    id: &'static str,
    icon: AppIcon,
    title: String,
    subtitle: String,
    active: bool,
    p: &Palette,
    on_click: impl Fn(&mut Window, &mut App) + 'static,
) -> AnyElement {
    let mut row = h_flex()
        .id(id)
        .w_full()
        .px(rems(space::SM))
        .py(rems(space::XS))
        .gap(rems(space::XS))
        .justify_between()
        .items_center()
        .rounded(p.radius.sm)
        .cursor_pointer()
        .hover(|s| s.bg(p.surface.hover))
        .on_click(move |_, w, cx| on_click(w, cx));
    if active {
        row = row.bg(p.surface.selected);
    }
    row = row.child(
        h_flex()
            .flex_1()
            .min_w_0()
            .gap(rems(space::XS))
            .items_start()
            .child(
                div()
                    .pt(px(2.0))
                    .flex_shrink_0()
                    .text_color(if active { p.primary } else { p.text.muted })
                    .child(AppIconElement::new(icon).small()),
            )
            .child(
                v_flex()
                    .flex_1()
                    .min_w_0()
                    .gap(px(1.0))
                    .child(
                        div()
                            .truncate()
                            .text_size(rems(typography::BODY))
                            .font_weight(if active {
                                FontWeight::SEMIBOLD
                            } else {
                                FontWeight::MEDIUM
                            })
                            .text_color(p.foreground)
                            .child(title),
                    )
                    .child(
                        div()
                            .truncate()
                            .text_size(rems(typography::CAPTION))
                            .text_color(p.text.muted)
                            .child(subtitle),
                    ),
            ),
    );
    if active {
        row = row.child(
            div()
                .flex_shrink_0()
                .text_color(p.success)
                .child(AppIconElement::new(AppIcon::Ui(UiIcon::GREEN_CHECKMARK)).small()),
        );
    }
    row.into_any_element()
}

/// 渲染输入卡片正上方的「目标会话选择器」浮层。
pub(in crate::app::ai) fn render_target_session_popup(
    params: TargetSessionPopupParams<'_>,
    actions: TargetSessionPopupActions,
) -> AnyElement {
    let p = params.p;
    let on_select_mode = std::rc::Rc::new(actions.on_select_mode);
    let on_toggle_coop = std::rc::Rc::new(actions.on_toggle_coop);
    let on_toggle_coop_session = std::rc::Rc::new(actions.on_toggle_coop_session);
    let on_insert_mention = std::rc::Rc::new(actions.on_insert_mention);
    let on_connect_and_pin = std::rc::Rc::new(actions.on_connect_and_pin);

    let follow_sub = params
        .workspace_active_session
        .map(|s| format!("{} · {}", s.host_name, s.login))
        .unwrap_or_else(|| rust_i18n::t!("ai.target_follow_tab_none").to_string());

    let pick_follow = on_select_mode.clone();
    let pick_global = on_select_mode.clone();

    let mut fixed_header = v_flex()
        .w_full()
        .gap(px(2.0))
        .child(section_title(
            rust_i18n::t!("ai.target_popup_auto_section").to_string(),
            p,
        ))
        .child(render_mode_row(
            "ai-target-follow",
            AppIcon::Ui(UiIcon::LAYOUT),
            rust_i18n::t!("ai.target_follow_tab").to_string(),
            follow_sub,
            params.target_mode == AiTargetMode::FollowActiveTab,
            p,
            move |w, cx| pick_follow(AiTargetMode::FollowActiveTab, w, cx),
        ))
        .child(render_mode_row(
            "ai-target-global",
            AppIcon::Domain(DomainIcon::TERMINAL),
            rust_i18n::t!("ai.target_global_only").to_string(),
            rust_i18n::t!("ai.target_global_only_desc").to_string(),
            params.target_mode == AiTargetMode::GlobalOnly,
            p,
            move |w, cx| pick_global(AiTargetMode::GlobalOnly, w, cx),
        ));

    let mut scroll_list = v_flex()
        .w_full()
        .max_h(px(220.0))
        .overflow_y_scrollbar()
        .gap(px(2.0));
    let mut has_scroll_items = false;

    if !params.connected_sessions.is_empty() {
        has_scroll_items = true;
        let primary_tab = params.active_session.map(|s| s.tab_id);
        scroll_list = scroll_list.child(section_title(
            rust_i18n::t!("ai.target_connected_section").to_string(),
            p,
        ));

        for (idx, sess) in params.connected_sessions.iter().enumerate() {
            let tid = sess.tab_id;
            let is_pinned = params.target_mode == AiTargetMode::PinnedTab(tid);
            let is_primary = primary_tab == Some(tid);
            let is_coop = params.extra_scope_tabs.contains(&tid);
            let pick_pin = on_select_mode.clone();
            let toggle_coop = on_toggle_coop.clone();

            let mut row = h_flex()
                .id(("ai-target-ssh", idx))
                .w_full()
                .px(rems(space::SM))
                .py(rems(space::XS))
                .gap(rems(space::XS))
                .justify_between()
                .items_center()
                .rounded(p.radius.sm)
                .cursor_pointer()
                .hover(|s| s.bg(p.surface.hover))
                .on_click(move |_, w, cx| pick_pin(AiTargetMode::PinnedTab(tid), w, cx));
            if is_pinned {
                row = row.bg(p.surface.selected);
            }
            row = row.child(
                h_flex()
                    .flex_1()
                    .min_w_0()
                    .gap(rems(space::XS))
                    .items_center()
                    .child(div().w(px(6.0)).h(px(6.0)).rounded_full().bg(p.success))
                    .child(
                        v_flex()
                            .flex_1()
                            .min_w_0()
                            .gap(px(1.0))
                            .child(
                                div()
                                    .truncate()
                                    .text_size(rems(typography::BODY))
                                    .font_weight(if is_pinned {
                                        FontWeight::SEMIBOLD
                                    } else {
                                        FontWeight::MEDIUM
                                    })
                                    .text_color(p.foreground)
                                    .child(sess.host_name.clone()),
                            )
                            .child(
                                div()
                                    .truncate()
                                    .text_size(rems(typography::CAPTION))
                                    .text_color(p.text.muted)
                                    .child(sess.login.clone()),
                            ),
                    ),
            );
            let mut trailing = h_flex().gap(px(4.0)).items_center().flex_shrink_0();
            if !is_primary {
                trailing = trailing.child(render_coop_toggle_btn(
                    ("ai-target-coop", idx),
                    is_coop,
                    p,
                    move |w, cx| toggle_coop(tid, w, cx),
                ));
            }
            if is_pinned {
                trailing = trailing.child(
                    div()
                        .text_color(p.success)
                        .child(AppIconElement::new(AppIcon::Ui(UiIcon::GREEN_CHECKMARK)).small()),
                );
            }
            row = row.child(trailing);
            scroll_list = scroll_list.child(row);
        }
    }

    // 优先展示本机（local）与容器引擎，再展示未连接的已保存 SSH 主机，避免 @local 被截断
    let mut other_targets: Vec<&AiMentionCandidate> = params
        .mention_candidates
        .iter()
        .filter(|c| {
            matches!(
                c.kind,
                AiMentionKind::Local | AiMentionKind::ContainerEngine
            )
        })
        .collect();
    other_targets.extend(
        params
            .mention_candidates
            .iter()
            .filter(|c| {
                matches!(
                    c.kind,
                    AiMentionKind::SshHost {
                        connected: false,
                        ..
                    }
                )
            })
            .take(6),
    );
    if !other_targets.is_empty() {
        if has_scroll_items {
            scroll_list = scroll_list.child(
                div()
                    .w_full()
                    .h(px(1.0))
                    .my(px(2.0))
                    .bg(p.divider(p.border)),
            );
        }
        has_scroll_items = true;
        scroll_list = scroll_list.child(section_title(
            rust_i18n::t!("ai.target_other_section").to_string(),
            p,
        ));
        for (idx, item) in other_targets.into_iter().enumerate() {
            let token = item.token.clone();
            let pick_mention = on_insert_mention.clone();
            let (icon, display_name) = match item.kind {
                AiMentionKind::Local => (
                    AppIcon::Domain(DomainIcon::TERMINAL),
                    format!("{} (@{})", item.detail, item.token),
                ),
                AiMentionKind::ContainerEngine => (
                    AppIcon::Custom(CustomIcon::DOCKER),
                    format!("Docker · {}", item.label),
                ),
                AiMentionKind::SshHost { .. } => {
                    (AppIcon::Ui(UiIcon::WEB_SERVER), format!("@{}", item.token))
                }
            };
            let mut trailing = h_flex().gap(px(4.0)).items_center().flex_shrink_0().child(
                div()
                    .max_w(px(130.0))
                    .truncate()
                    .text_size(rems(typography::CAPTION))
                    .text_color(p.text.muted)
                    .child(item.detail.clone()),
            );
            if let AiMentionKind::SshHost {
                connected: false,
                session_id,
            } = item.kind
            {
                let is_coop_saved = params.extra_scope_sessions.contains(&session_id);
                let toggle_saved = on_toggle_coop_session.clone();
                let connect_pin = on_connect_and_pin.clone();
                trailing = trailing
                    .child(render_coop_toggle_btn(
                        ("ai-target-saved-coop", idx),
                        is_coop_saved,
                        p,
                        move |w, cx| toggle_saved(session_id, w, cx),
                    ))
                    .child(
                        h_flex()
                            .id(("ai-target-connect-pin", idx))
                            .px(px(6.0))
                            .py(px(2.0))
                            .rounded(p.radius.sm)
                            .border_1()
                            .border_color(p.divider(p.border))
                            .cursor_pointer()
                            .text_size(rems(typography::CAPTION))
                            .text_color(p.primary)
                            .hover(|s| {
                                s.bg(p.tone.primary_subtle)
                                    .border_color(p.primary.opacity(0.5))
                            })
                            .on_click(move |_, w, cx| {
                                cx.stop_propagation();
                                connect_pin(session_id, w, cx);
                            })
                            .child(rust_i18n::t!("ai.target_connect_and_pin").to_string()),
                    );
            }
            scroll_list = scroll_list.child(
                h_flex()
                    .id(("ai-target-other", idx))
                    .w_full()
                    .px(rems(space::SM))
                    .py(rems(space::XS))
                    .gap(rems(space::XS))
                    .justify_between()
                    .items_center()
                    .rounded(p.radius.sm)
                    .cursor_pointer()
                    .hover(|s| s.bg(p.surface.hover))
                    .on_click(move |_, w, cx| pick_mention(token.clone(), w, cx))
                    .child(
                        h_flex()
                            .flex_1()
                            .min_w_0()
                            .gap(rems(space::XS))
                            .items_center()
                            .child(
                                div()
                                    .flex_shrink_0()
                                    .text_color(p.text.muted)
                                    .child(AppIconElement::new(icon).small()),
                            )
                            .child(
                                div()
                                    .truncate()
                                    .text_size(rems(typography::BODY))
                                    .text_color(p.foreground)
                                    .child(display_name),
                            ),
                    )
                    .child(trailing),
            );
        }
    }

    if has_scroll_items {
        fixed_header = fixed_header
            .child(
                div()
                    .w_full()
                    .h(px(1.0))
                    .my(px(2.0))
                    .bg(p.divider(p.border)),
            )
            .child(scroll_list);
    }

    v_flex()
        .w_full()
        .p(rems(space::XXS))
        .rounded(p.radius.md)
        .bg(p.surface.floating)
        .border_1()
        .border_color(p.border)
        .shadow_md()
        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
        .child(fixed_header)
        .into_any_element()
}
