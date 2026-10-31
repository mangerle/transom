//! 弹窗体渲染、焦点归属与弹窗级按键动作。

use super::stack::{cancel_modal, close_layer, commit_modal};
use super::*;

/// 弹窗打开后要把焦点交给谁。
///
/// 为什么需要：`open_dialog` 会把焦点移到弹窗容器上，站点原先在打开时写的
/// `input.focus(..)` 会被这一步覆盖，表现为一打开就打字没反应。这里由弹窗体
/// 在首帧按种类取回焦点，和容器抢焦点的顺序也就此固定下来。
pub(crate) enum ModalFocus {
    Input(Entity<InputState>),
    Textarea(Entity<TextareaState>),
    /// 面板自己持有的焦点句柄（如改键录入面板）。
    Handle(FocusHandle),
    /// 没有文本输入控件：焦点留在弹窗外壳，交给 Tab 键遍历内部控件。
    None,
}

/// 弹窗体实体：随外壳状态自绘。
///
/// 为什么必须是独立实体：弹窗层由组件库内部状态驱动重建，宿主 `cx.notify()`
/// 只会重绘外壳子树、不会重跑 `open_dialog` 的 builder；实体自己脏了才会重绘。
/// 实测依据见 `tests/dialog_key_probe.rs::宿主变脏时弹窗体独立重绘而弹窗层不重建`。
pub(super) struct ModalBody {
    pub(super) kind: ModalKind,
    pub(super) app: Entity<AppShell>,
    /// 首帧需要把焦点交给输入控件，之后不再干预用户的焦点移动。
    pub(super) pending_focus: bool,
    /// 保留订阅：外壳变更后让本实体脏掉重绘；丢弃即失联。
    pub(super) _observe: Subscription,
}

impl Render for ModalBody {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let p = Palette::from(cx);
        let app = self.app.clone();
        let kind = self.kind;
        if self.pending_focus {
            self.pending_focus = false;
            match kind.focus_target(app.read(cx)) {
                ModalFocus::Input(input) => {
                    input.update(cx, |state, cx| state.focus(window, cx));
                }
                ModalFocus::Textarea(textarea) => {
                    textarea.update(cx, |state, cx| state.focus(window, cx));
                }
                ModalFocus::Handle(handle) => handle.focus(window, cx),
                ModalFocus::None => {}
            }
        }
        let body = kind.render_body(app.read(cx), &app, window, cx, &p);
        let hint = app.read(cx).dialog_hint.clone();
        div()
            .id(ElementId::Name(
                format!("modal-body-{}", kind.spec(window).slug).into(),
            ))
            // 两路并存：焦点在输入框里时按键归输入框（容器那条键位不生效），
            // 焦点在弹窗外壳或按钮上时按键归容器动作；四种落点都指向同一条
            // 取消/提交逻辑。截住传播，避免焦点位于弹窗内部时容器同名动作
            // 再被执行一次。
            .on_action({
                let app = app.clone();
                move |_: &InputEscape, window, cx| {
                    if cancel_modal(&app, kind, window, cx) {
                        close_layer(&app, kind, cx);
                        window.close_dialog(cx);
                    }
                    cx.stop_propagation();
                }
            })
            .on_action({
                let app = app.clone();
                move |_: &InputEnter, window, cx| {
                    if commit_modal(&app, kind, window, cx) {
                        close_layer(&app, kind, cx);
                        window.close_dialog(cx);
                    }
                    cx.stop_propagation();
                }
            })
            .on_action({
                // 焦点落在弹窗内部控件上时，容器自己的 Cancel 处理还在更外层，
                // 这里先执行并截住，避免两条取消链各跑一次。
                let app = app.clone();
                move |_: &Cancel, window, cx| {
                    if cancel_modal(&app, kind, window, cx) {
                        close_layer(&app, kind, cx);
                        window.close_dialog(cx);
                    }
                    cx.stop_propagation();
                }
            })
            .on_action({
                let app = app.clone();
                move |_: &Confirm, window, cx| {
                    if commit_modal(&app, kind, window, cx) {
                        close_layer(&app, kind, cx);
                        window.close_dialog(cx);
                    }
                    cx.stop_propagation();
                }
            })
            .child(body)
            .children(hint.map(|text| {
                div()
                    .text_xs()
                    .text_color(Palette::from(cx).danger)
                    .child(text)
            }))
    }
}

/// 弹窗体内的事件回调适配器。
///
/// 为什么需要：弹窗体渲染时手上只有 `Entity<AppShell>` 与 `&mut App`，
/// 拿不到 `Context<AppShell>`，原先的 `cx.listener` 写法无处可用。事件回调
/// 发生在渲染之外，用实体 `update` 取回同样的借用对，即可原样复用各领域
/// 已有的状态变更方法，不必为弹窗另写一套逻辑。
pub(crate) fn on_event<A, F>(
    app: &Entity<AppShell>,
    handler: F,
) -> impl Fn(&A, &mut Window, &mut App) + 'static
where
    A: 'static,
    F: Fn(&mut AppShell, &A, &mut Window, &mut Context<AppShell>) + 'static,
{
    let app = app.clone();
    move |event, window, cx| {
        app.update(cx, |shell, cx| handler(shell, event, window, cx));
    }
}

/// 多行输入框的提交键位绑定：容器只绑了裸 `enter`。
///
/// 绑在 `"Dialog"` 上下文里，只在弹窗打开时生效，不会抢走终端与编辑器
/// 面板的 Ctrl+字母控制码；与 `keymap::bindings()` 一起注册。
pub(crate) const DIALOG_CONTEXT: &str = "Dialog";

pub(crate) fn dialog_bindings() -> Vec<KeyBinding> {
    vec![KeyBinding::new(
        "ctrl-enter",
        Confirm { secondary: true },
        Some(DIALOG_CONTEXT),
    )]
}
