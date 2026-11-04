//! 侧边栏拖拽重排序落点遮罩、拖拽预览与行内操作按钮。

use super::{SidebarDrag, SidebarTarget};
use crate::app::AppShell;
use crate::app_assets::AppIcon;
use crate::palette::Palette;
use gpui_kit::assets::IconName;
use gpui_kit::base::h_flex;
use gpui_kit::component::{Icon, Sizable};
use gpui_kit::*;
use transom_core::DropSide;

/// 侧栏行快捷操作按钮配置属性。
pub(crate) struct RowActionButtonProps<F> {
    pub(crate) element_id: String,
    pub(crate) icon: IconName,
    pub(crate) tooltip: String,
    pub(crate) idle_color: Hsla,
    pub(crate) on_click: F,
}

/// 行内图标按钮：常驻但弱色，悬停时才加深并垫一层底色。
///
/// 设计原理（Why）：
/// - 实现初衷：入口要挂在每一行上才谈得上"快捷"，做成悬停才浮现等于把入口藏起来，
///   用户反而不知道该点哪里，所以图标必须始终可见。
/// - 核心优势：用 `muted_foreground` 弱色与 14px 小尺寸，让它明显轻于行首的
///   18px 协议图标，几百行的列表不会被图标压成噪声；hover 态再补足可读性。
///   悬浮样式与 `id` 落在同一个元素上，靠按钮自己的 hover 表达状态，不去读父行
///   （父行的 hover 只会改底色，拿不到布尔值）。
/// - 常态色由调用方给出：活动会话是实心强调底，弱色图标在上面会糊成一片，
///   与行首协议图标共用同一套 `is_active` 判断才不各自漂移。
/// - 代价与局限：图标占据固定布局宽度（不是绝对定位叠加），行名可用宽度比没有
///   按钮时少 20px；不这么做的话悬停时行名会跟着抖动，比少 20px 更难接受。
pub(crate) fn row_action_button(
    props: RowActionButtonProps<impl Fn(&ClickEvent, &mut Window, &mut App) + 'static>,
    _p: &Palette,
) -> AnyElement {
    let RowActionButtonProps {
        element_id,
        icon,
        tooltip,
        idle_color,
        on_click,
    } = props;
    crate::widgets::IconButton::new(ElementId::Name(element_id.into()), icon, tooltip)
        .size(crate::widgets::IconButtonSize::RowAction)
        .render()
        .text_color(idle_color)
        .on_click(on_click)
        .into_any_element()
}

/// 拖动时跟随鼠标的轻量预览卡片。
pub(crate) struct SidebarDragPreview {
    label: String,
    icon: AppIcon,
}

impl SidebarDragPreview {
    pub(crate) fn new(label: String, icon: impl Into<AppIcon>) -> Self {
        Self {
            label,
            icon: icon.into(),
        }
    }
}

impl Render for SidebarDragPreview {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let p = Palette::from(cx);
        h_flex()
            .px_3()
            .py_1p5()
            .gap_2()
            .items_center()
            .rounded(p.radius.md)
            .bg(p.popover)
            .border_1()
            .border_color(p.border)
            .shadow_md()
            .text_sm()
            .text_color(p.popover_foreground)
            .child(Icon::new(self.icon).xsmall().text_color(p.muted_foreground))
            .child(self.label.clone())
    }
}

/// 覆盖整行的排序落点：把行按份数等分，用户不需要瞄准行边缘。
///
/// 会话行二分：上半插到该行之前、下半之后。文件夹行三分并留出中间一块，
/// 中间块不注册任何监听，因此不产生命中框，拖放会继续落到行本身，
/// 即“放入该文件夹”。等分用 `flex_1` 而非百分比宽度，gpui 没有宽度百分比。
pub(super) fn reorder_overlay(
    app: &Entity<AppShell>,
    anchor: SidebarDrag,
    keep_middle: bool,
    p: &Palette,
) -> AnyElement {
    let overlay = div()
        .absolute()
        .inset_0()
        .flex()
        .flex_col()
        .child(reorder_half(app, anchor, DropSide::Before, true, p));
    let overlay = if keep_middle {
        overlay.child(div().flex_1())
    } else {
        overlay
    };
    overlay
        .child(reorder_half(app, anchor, DropSide::After, false, p))
        .into_any_element()
}

/// 半区落点本身：`top` 决定插入线画在上沿还是下沿。
///
/// 插入线用 accent_foreground：accent 已经是行底色，同色描边看不出。
fn reorder_half(
    app: &Entity<AppShell>,
    anchor: SidebarDrag,
    side: DropSide,
    top: bool,
    p: &Palette,
) -> AnyElement {
    let target = SidebarTarget::Relative { anchor, side };
    // `drag_over` 的闭包要求 `'static`，颜色先取成局部值再捕获。
    let line_color = p.sidebar_accent_foreground;
    let zone = div().flex_1().border_color(line_color);
    let zone = if top {
        zone.drag_over::<SidebarDrag>(move |style, _, _, _| {
            style.border_t_2().border_color(line_color)
        })
    } else {
        zone.drag_over::<SidebarDrag>(move |style, _, _, _| {
            style.border_b_2().border_color(line_color)
        })
    };
    let app = app.clone();
    zone.on_drop(move |drag: &SidebarDrag, window, cx| {
        let drag = *drag;
        app.update(cx, |this, cx| {
            this.handle_sidebar_drop(drag, target, cx);
            window.refresh();
        });
    })
    .into_any_element()
}
