//! 窗格与并排标签格共用的标题条。
//!
//! 设计原理（Why）：标签组内的窗格头与并排网格的格头此前各写一份布局，
//! 高度、状态点、图标尺寸、焦点配色与关闭按钮的 hover 反馈必须手工保持一致，
//! 改一处漏一处就出现两种"看起来像但不是"的标题条。这里只收敛视觉与关闭
//! 按钮的通用交互；激活语义（切窗格 / 切标签组）与右键菜单由调用方注入。

use crate::divider::{DividerEdge, DividerStyled};
use crate::palette::Palette;
use crate::theme::tokens::{size, typography};
use gpui_kit::base::h_flex;
use gpui_kit::component::Sizable as _;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::*;
use transom_core::DividerWidth;
use transom_ui_icons::AppIcon;
use transom_ui_icons::AppIconElement;
use transom_ui_icons::UiIcon;

/// 窗格头失败诊断按钮回调类型。
pub(super) type DiagnoseClickCb = Box<dyn Fn(&ClickEvent, &mut Window, &mut App)>;

/// 窗格头失败诊断信息：退出码、点击诊断回调与忽略该失败块回调。
pub(super) struct FailedDiagProps {
    pub(super) exit_code: i32,
    pub(super) on_diagnose: DiagnoseClickCb,
    pub(super) on_dismiss: DiagnoseClickCb,
}

/// 从窗格终端的 OSC-133 失败块构建一键 AI 诊断参数（自动跳过已忽略块）。
pub(super) fn build_failed_diag(
    tab_id: transom_core::TabId,
    state: &crate::app::AppState,
    cx: &mut Context<crate::app::AppShell>,
) -> Option<FailedDiagProps> {
    let tab = state.workspace.tab(tab_id)?;
    let dismissed = state.features.dismissed_failed_blocks.get(&tab_id).copied();
    let (block_id, exit_code, summary) = tab.terminal()?.read(cx).last_failed_block()?;
    if dismissed == Some(block_id) {
        return None;
    }
    let on_diagnose: DiagnoseClickCb = Box::new(cx.listener(move |this, _, window, cx| {
        cx.stop_propagation();
        this.activate_tab(tab_id, window, cx);
        this.state.features.ai_panel_open = true;
        let snippet = summary.clone();
        this.state.features.ai_panel.update(cx, |panel, cx| {
            panel.explain_terminal_output(snippet, cx);
        });
        cx.notify();
    }));
    let on_dismiss: DiagnoseClickCb = Box::new(cx.listener(move |this, _, _, cx| {
        cx.stop_propagation();
        this.state
            .features
            .dismissed_failed_blocks
            .insert(tab_id, block_id);
        cx.notify();
    }));
    Some(FailedDiagProps {
        exit_code,
        on_diagnose,
        on_dismiss,
    })
}

/// 标题条的渲染输入。
pub(super) struct PaneHeaderProps<'a> {
    /// 容器元素标识，必须跨帧稳定。
    pub(super) id: String,
    /// 关闭按钮的元素标识，与容器分开以免嵌套点击互相干扰。
    pub(super) close_id: String,
    /// 是否为当前焦点所在格。
    pub(super) active: bool,
    pub(super) dot_color: Hsla,
    pub(super) icon: AppIcon,
    pub(super) title: String,
    /// 组内窗格序号；并排格没有序号，传 `None`。
    pub(super) ordinal: Option<usize>,
    /// 最近一条 OSC-133 非零退出码及一键 AI 诊断回调。
    pub(super) failed_diag: Option<FailedDiagProps>,
    pub(super) palette: &'a Palette,
}

/// 构造标题条；点击容器与点击关闭按钮的行为由调用方提供。
pub(super) fn pane_header<F, G>(
    props: PaneHeaderProps<'_>,
    on_activate: F,
    on_close: G,
) -> Stateful<Div>
where
    F: Fn(&ClickEvent, &mut Window, &mut App) + 'static,
    G: Fn(&ClickEvent, &mut Window, &mut App) + 'static,
{
    let p = props.palette;
    // 焦点格只靠背景色区分太弱（激活色与标签栏底色只差几个色阶），用强调
    // 下划线把"焦点在哪一格"显式画出来：在分割线档位上加一档，同时换色。
    let underline_width = if props.active {
        match p.divider_width {
            DividerWidth::Hairline | DividerWidth::Unknown => DividerWidth::Medium,
            DividerWidth::Medium | DividerWidth::Thick => DividerWidth::Thick,
        }
    } else {
        p.divider_width
    };
    let mut header = h_flex()
        .id(ElementId::Name(props.id.into()))
        .h(rems(size::PANE_HEADER_HEIGHT))
        .flex_none()
        .px_2()
        .gap_2()
        .items_center()
        .border_color(if props.active {
            p.primary
        } else {
            p.divider(p.border)
        })
        .bg(if props.active {
            p.tab_active
        } else {
            p.tab_bar
        })
        .text_color(if props.active {
            p.tab_active_foreground
        } else {
            p.tab_foreground
        })
        .cursor_pointer()
        .on_click(on_activate)
        .child(
            div()
                .size(px(7.0))
                .rounded_full()
                .flex_none()
                .bg(props.dot_color),
        )
        .child(AppIconElement::new(props.icon).xsmall());
    if let Some(ordinal) = props.ordinal {
        header = header.child(
            div()
                .text_xs()
                .flex_none()
                .text_color(p.muted_foreground)
                .child(ordinal.to_string()),
        );
    }
    let close_id = props.close_id;
    let mut header_end = header.child(
        div()
            .flex_1()
            .text_xs()
            .overflow_hidden()
            .whitespace_nowrap()
            .truncate()
            .child(props.title),
    );
    if let Some(diag) = props.failed_diag {
        let code = diag.exit_code;
        let on_diag = diag.on_diagnose;
        let on_dismiss = diag.on_dismiss;
        let dismiss_tip = rust_i18n::t!("ai.dismiss_failed_diag").to_string();
        header_end = header_end.child(
            h_flex()
                .items_center()
                .gap(px(2.0))
                .on_mouse_down(MouseButton::Left, |_, window, cx| {
                    window.prevent_default();
                    cx.stop_propagation();
                })
                .child(
                    Button::new(SharedString::from(format!("{close_id}-diag")))
                        .ghost()
                        .xsmall()
                        .child(
                            div()
                                .text_size(rems(typography::CAPTION))
                                .text_color(p.warning)
                                .child(
                                    rust_i18n::t!("ai.diagnose_exit_btn", code = code).to_string(),
                                ),
                        )
                        .on_click(move |ev, w, cx| on_diag(ev, w, cx)),
                )
                .child(
                    Button::new(SharedString::from(format!("{close_id}-diag-dismiss")))
                        .ghost()
                        .xsmall()
                        .accessibility_label(dismiss_tip.clone())
                        .tooltip(dismiss_tip)
                        .child(AppIconElement::new(AppIcon::Ui(UiIcon::CLOSE)).xsmall())
                        .on_click(move |ev, w, cx| on_dismiss(ev, w, cx)),
                ),
        );
    }
    header_end
        .child(
            div()
                .id(ElementId::Name(close_id.into()))
                .size(px(18.0))
                .rounded(p.radius.sm)
                .flex()
                .items_center()
                .justify_center()
                .cursor_pointer()
                .hover(|style| {
                    style
                        .bg(p.muted_foreground.opacity(0.2))
                        .text_color(p.foreground)
                })
                .on_mouse_down(MouseButton::Left, |_, window, cx| {
                    window.prevent_default();
                    cx.stop_propagation();
                })
                .on_click(on_close)
                .child(AppIconElement::new(AppIcon::Ui(UiIcon::CLOSE)).xsmall()),
        )
        .divider_edge(DividerEdge::Bottom, underline_width)
}
