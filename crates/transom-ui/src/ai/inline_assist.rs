//! 终端内联 AI 唤起浮条（Ctrl/Cmd+I）：不切侧栏直接在终端底部生成并插入命令。

use gpui_kit::base::{h_flex, v_flex};
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::input::{Input, InputState};
use gpui_kit::component::{Disableable as _, Sizable as _};
use gpui_kit::*;
use transom_ui_icons::{AppIcon, AppIconElement, CustomIcon, UiIcon};

use crate::palette::Palette;
use crate::theme::tokens::{space, typography};

/// 终端内联命令生成浮条的独立状态（归属工作区前端特征状态）。
pub(crate) struct InlineAssistState {
    pub(crate) open: bool,
    pub(crate) loading: bool,
    pub(crate) active_request_id: Option<String>,
    pub(crate) generated_command: Option<String>,
    pub(crate) explanation: String,
    pub(crate) error: Option<String>,
    pub(crate) input: Entity<InputState>,
    pub(crate) _sub: Option<Subscription>,
}

impl InlineAssistState {
    pub(crate) fn new(window: &mut Window, cx: &mut App) -> Self {
        let input = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder(rust_i18n::t!("ai.inline_placeholder").to_string())
        });
        Self {
            open: false,
            loading: false,
            active_request_id: None,
            generated_command: None,
            explanation: String::new(),
            error: None,
            input,
            _sub: None,
        }
    }
}

/// 在活动终端底部渲染轻量内联 AI 命令生成浮条。
pub(crate) fn render_inline_assist_overlay(
    state: &InlineAssistState,
    p: &Palette,
    on_submit: impl Fn(&mut Window, &mut App) + 'static,
    on_insert: impl Fn(String, &mut Window, &mut App) + 'static,
    on_close: impl Fn(&mut Window, &mut App) + 'static,
) -> AnyElement {
    if !state.open {
        return div().into_any_element();
    }
    let on_close_btn = std::rc::Rc::new(on_close);
    let on_close_esc = on_close_btn.clone();
    let on_submit_rc = std::rc::Rc::new(on_submit);
    let on_submit_key = on_submit_rc.clone();

    let mut card = v_flex()
        .id("terminal-inline-ai-assist")
        .w(px(520.0))
        .max_w(relative(0.88))
        .p(rems(space::SM))
        .gap(rems(space::XS))
        .rounded(p.radius.md)
        .bg(p.surface.floating)
        .border_1()
        .border_color(p.primary.opacity(0.5))
        .shadow_md()
        .on_key_down(move |ev, w, cx| {
            if ev.keystroke.key == "escape" {
                on_close_esc(w, cx);
            } else if ev.keystroke.key == "enter" && !ev.keystroke.modifiers.shift {
                on_submit_key(w, cx);
            }
        })
        .child(
            h_flex()
                .justify_between()
                .items_center()
                .child(
                    h_flex()
                        .gap(px(6.0))
                        .items_center()
                        .child(div().text_color(p.primary).child(
                            AppIconElement::new(AppIcon::Custom(CustomIcon::AI_SPARKLE)).small(),
                        ))
                        .child(
                            div()
                                .text_size(rems(typography::CAPTION))
                                .font_weight(FontWeight::SEMIBOLD)
                                .text_color(p.foreground)
                                .child(rust_i18n::t!("ai.inline_title").to_string()),
                        ),
                )
                .child(
                    Button::new("inline-ai-close")
                        .ghost()
                        .xsmall()
                        .child(AppIconElement::new(AppIcon::Ui(UiIcon::CLOSE)).small())
                        .on_click(move |_, w, cx| on_close_btn(w, cx)),
                ),
        )
        .child(
            h_flex()
                .w_full()
                .gap(rems(space::XS))
                .items_center()
                .child(div().flex_1().child(Input::new(&state.input)))
                .child(
                    Button::new("inline-ai-run")
                        .primary()
                        .small()
                        .disabled(state.loading)
                        .label(if state.loading {
                            rust_i18n::t!("ai.inline_generating").to_string()
                        } else {
                            rust_i18n::t!("ai.inline_generate").to_string()
                        })
                        .on_click(move |_, w, cx| on_submit_rc(w, cx)),
                ),
        );

    if let Some(err) = &state.error {
        card = card.child(
            div()
                .text_size(rems(typography::CAPTION))
                .text_color(p.danger)
                .child(err.clone()),
        );
    }

    if let Some(cmd) = &state.generated_command {
        let cmd_to_insert = cmd.clone();
        card = card.child(
            v_flex()
                .w_full()
                .p(rems(space::XS))
                .gap(px(4.0))
                .rounded(p.radius.sm)
                .bg(p.background)
                .border_1()
                .border_color(p.divider(p.border))
                .child(
                    div()
                        .font_family("monospace")
                        .text_size(rems(typography::BODY))
                        .text_color(p.foreground)
                        .child(cmd.clone()),
                )
                .child(
                    h_flex()
                        .justify_between()
                        .items_center()
                        .gap(rems(space::XS))
                        .child(
                            div()
                                .flex_1()
                                .text_size(rems(typography::CAPTION))
                                .text_color(p.text.muted)
                                .child(state.explanation.clone()),
                        )
                        .child(
                            Button::new("inline-ai-insert")
                                .primary()
                                .xsmall()
                                .label(rust_i18n::t!("ai.inline_insert").to_string())
                                .on_click(move |_, w, cx| on_insert(cmd_to_insert.clone(), w, cx)),
                        ),
                ),
        );
    }

    deferred(
        div()
            .absolute()
            .bottom(px(28.0))
            .left(px(0.0))
            .right(px(0.0))
            .flex()
            .justify_center()
            .child(card),
    )
    .with_priority(2)
    .into_any_element()
}

/// 绑定内联输入框回车事件，并提供给 AppShell 的工作区快捷调用入口。
pub(crate) fn open_inline_assist(
    state: &mut crate::app::AppState,
    window: &mut Window,
    cx: &mut App,
) {
    state.features.inline_assist.open = true;
    state.features.inline_assist.error = None;
    let focus = state.features.inline_assist.input.read(cx).focus_handle(cx);
    window.focus(&focus, cx);
}

pub(crate) fn submit_inline_assist(state: &mut crate::app::AppState, cx: &App) {
    let prompt = state
        .features
        .inline_assist
        .input
        .read(cx)
        .value()
        .trim()
        .to_owned();
    if prompt.is_empty() || state.features.inline_assist.loading {
        return;
    }
    let req_id = uuid::Uuid::new_v4().to_string();
    state.features.inline_assist.loading = true;
    state.features.inline_assist.active_request_id = Some(req_id.clone());
    state.features.inline_assist.error = None;
    let (host_label, platform) = state
        .workspace
        .active_tab
        .and_then(|tid| state.workspace.tab(tid))
        .and_then(|tab| tab.session())
        .map(|s| {
            let plat = match s.connection {
                transom_core::Connection::Local(_) => std::env::consts::OS.to_owned(),
                _ => "linux".to_owned(),
            };
            (s.name.clone(), plat)
        })
        .unwrap_or_else(|| ("local".to_owned(), std::env::consts::OS.to_owned()));
    let _ = state
        .runtime
        .services
        .command_tx
        .try_send(transom_core::AppCommand::Ai(Box::new(
            transom_core::AiCommand::InlineCommandAssist {
                request_id: req_id,
                prompt,
                host_label,
                platform,
            },
        )));
}

pub(crate) fn apply_inline_result(
    state: &mut crate::app::AppState,
    request_id: &str,
    command: Option<String>,
    explanation: String,
    error: Option<String>,
) {
    if state.features.inline_assist.active_request_id.as_deref() != Some(request_id) {
        return;
    }
    state.features.inline_assist.loading = false;
    state.features.inline_assist.generated_command = command;
    state.features.inline_assist.explanation = explanation;
    state.features.inline_assist.error = error;
}

pub(crate) fn render_workspace_inline_assist(
    state: &crate::app::AppState,
    p: &Palette,
    app: &Entity<crate::app::AppShell>,
) -> AnyElement {
    let app_sub = app.clone();
    let app_ins = app.clone();
    let app_cls = app.clone();
    render_inline_assist_overlay(
        &state.features.inline_assist,
        p,
        move |_, cx| {
            app_sub.update(cx, |this, cx| {
                submit_inline_assist(&mut this.state, cx);
                cx.notify();
            });
        },
        move |cmd, window, cx| {
            app_ins.update(cx, |this, cx| {
                if let Some(term) = this.state.workspace.active_pane_terminal() {
                    term.update(cx, |v, cx| {
                        let _ = v.send_text(&cmd, cx);
                        window.focus(v.focus_handle(), cx);
                    });
                }
                this.state.features.inline_assist.open = false;
                cx.notify();
            });
        },
        move |_, cx| {
            app_cls.update(cx, |this, cx| {
                this.state.features.inline_assist.open = false;
                cx.notify();
            });
        },
    )
}
