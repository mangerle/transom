//! 迁移前置取证：Dialog 容器与输入框组件的按键归属。
//!
//! 为什么要实测而不是照抄上游文档：容器在 `"Dialog"` 上下文绑定了
//! `escape → Cancel` 与 `enter → Confirm`，而输入框组件不注册任何键绑定、
//! 自己在元素层吃按键。两者谁先命中，决定本次迁移定下的口径
//! （Input 用 Enter 提交、Textarea 用 Enter 换行、Ctrl+Enter 提交）
//! 是能白拿，还是要宿主补绑。下面每条断言写的就是目标口径，
//! 跑出来的差异即待补的工作，同时留作迁移后的键位回归门禁。
//!
//! 为什么用普通 `#[test]` 手工起 `TestAppContext` 而不是上游的
//! `#[gpui::test]`：本仓库经 gpui-kit 门面消费 gpui，门面里
//! `pub use crate as gpui`，测试宏展开出的 `gpui::` 路径会回指门面自身，
//! 展开走成自引用递归（把 recursion_limit 抬到 1024 直接把 rustc 栈打爆）。
//!
//! 为什么这里不使用 `use gpui_kit::*`：门面的 glob 会把同名测试宏 `test`
//! 带进属性命名空间，`#[test]` 因此被解析成那个宏自身而不是内置测试属性，
//! 同样触发无限展开。用例需要的类型一律显式导入。

use gpui_kit::component::Root;
gpui_kit::actions!(probe_keys, [WorkspaceProbe]);
use gpui_kit::component::WindowExt;
use gpui_kit::component::dialog::Dialog;
use gpui_kit::component::dialog::DialogEntrance;
use gpui_kit::component::input::{Enter as InputEnter, Escape as InputEscape, Input, InputState};
use gpui_kit::component::input::{Textarea, TextareaState};
use gpui_kit::{
    App, AppContext, Context, Entity, FocusHandle, InteractiveElement, IntoElement, KeyBinding,
    ParentElement, Render, Size, Styled, Subscription, TestAppContext, VisualTestContext, Window,
    div, init, px,
};
use std::cell::{Cell, RefCell};
use std::rc::Rc;

/// 事件落点记录：一次按键命中了哪些处理器，按命中顺序记字符串。
type Log = Rc<RefCell<Vec<String>>>;

/// 探针弹窗用到的两个实体句柄：被观察方与弹窗体。
type ProbeEntities = (Entity<CountedHost>, Entity<ObservingBody>);

fn new_log() -> Log {
    Rc::new(RefCell::new(Vec::new()))
}

fn mark(log: &Log, name: &'static str) {
    log.borrow_mut().push(name.to_string());
}

/// 弹窗宿主：只提供一块画布，全部真实内容进 Dialog 容器。
struct ProbeHost;

impl Render for ProbeHost {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div().size_full().child("probe")
    }
}

/// 起一个装好组件层（含弹窗层与 Dialog 键绑定）的无头窗口。
fn probe_window(cx: &mut TestAppContext) -> &mut VisualTestContext {
    cx.update(|cx| {
        init(cx);
        cx.set_reduce_motion(true);
    });
    let (_, cx) = cx.add_window_view(|window, cx| {
        let view = cx.new(|_| ProbeHost);
        Root::new(view, window, cx)
    });
    cx.simulate_resize(Size {
        width: px(1000.),
        height: px(800.),
    });
    cx.update(|window, cx| window.draw(cx).clear(cx));
    cx
}

/// 首轮把焦点交给单行输入框，复现"弹窗一打开就能打字"的真实路径。
///
/// 容器打开时焦点落在弹窗外壳上，与焦点在输入框内是两条按键归属路径，
/// 所以两种位置都要能取消弹窗。
fn open_input_dialog(cx: &mut VisualTestContext, log: &Log) {
    let first = Rc::new(Cell::new(true));
    cx.update({
        let log = Rc::clone(log);
        move |window, cx| {
            let input = cx.new(|cx| InputState::new(window, cx).default_value("abc"));
            let builder = {
                let input = input.clone();
                let log = Rc::clone(&log);
                move |dialog: Dialog, window: &mut Window, cx: &mut App| {
                    if first.replace(false) {
                        input.update(cx, |state, cx| state.focus(window, cx));
                    }
                    let enter_log = Rc::clone(&log);
                    let escape_log = Rc::clone(&log);
                    let confirm_log = Rc::clone(&log);
                    let cancel_log = Rc::clone(&log);
                    dialog
                        .title("Probe")
                        .entrance(DialogEntrance::None)
                        .overlay_closable(false)
                        .child(
                            div()
                                .id("probe-body")
                                .on_action(move |_: &InputEnter, _, _| {
                                    mark(&enter_log, "input-enter")
                                })
                                .on_action(move |_: &InputEscape, _, _| {
                                    mark(&escape_log, "input-escape")
                                })
                                .child(Input::new(&input)),
                        )
                        .on_ok(move |_, _, _| {
                            mark(&confirm_log, "confirm");
                            true
                        })
                        .on_cancel(move |_, _, _| {
                            mark(&cancel_log, "cancel");
                            true
                        })
                }
            };
            window.open_dialog(cx, builder);
        }
    });
    paint(cx);
}

/// 打开一个带多行输入框的探针弹窗，返回其状态实体供读取正文。
fn open_textarea_dialog(cx: &mut VisualTestContext, log: &Log) -> Entity<TextareaState> {
    let first = Rc::new(Cell::new(true));
    let textarea = cx.update({
        let log = Rc::clone(log);
        move |window, cx| {
            let textarea = cx.new(|cx| TextareaState::new(window, cx).default_value("line"));
            let builder = {
                let textarea = textarea.clone();
                let log = Rc::clone(&log);
                move |dialog: Dialog, window: &mut Window, cx: &mut App| {
                    if first.replace(false) {
                        textarea.update(cx, |state, cx| state.focus(window, cx));
                    }
                    let confirm_log = Rc::clone(&log);
                    let cancel_log = Rc::clone(&log);
                    let enter_log = Rc::clone(&log);
                    dialog
                        .title("Probe")
                        .entrance(DialogEntrance::None)
                        .overlay_closable(false)
                        .child(
                            div()
                                .id("textarea-probe-body")
                                .on_action(move |_: &InputEnter, _, _| {
                                    mark(&enter_log, "input-enter")
                                })
                                .child(Textarea::new(&textarea)),
                        )
                        .on_ok(move |_, _, _| {
                            mark(&confirm_log, "confirm");
                            true
                        })
                        .on_cancel(move |_, _, _| {
                            mark(&cancel_log, "cancel");
                            true
                        })
                }
            };
            window.open_dialog(cx, builder);
            textarea
        }
    });
    paint(cx);
    textarea
}

/// 弹窗打开后需要两帧：一帧挂上层，一帧把内容画到位，按键才落到上下文栈里。
fn paint(cx: &mut VisualTestContext) {
    cx.run_until_parked();
    cx.update(|window, cx| {
        window.draw(cx).clear(cx);
    });
    cx.update(|window, cx| {
        window.draw(cx).clear(cx);
    });
}

fn dialog_open(cx: &mut VisualTestContext) -> bool {
    cx.update(|window, cx| window.has_active_dialog(cx))
}

fn hits(log: &Log) -> Vec<String> {
    log.borrow().clone()
}

#[test]
fn 焦点在单行输入框内时容器的_escape_不触发() {
    let mut app = TestAppContext::single();
    let cx = probe_window(&mut app);
    let log = new_log();
    open_input_dialog(cx, &log);
    log.borrow_mut().clear();

    cx.simulate_keystrokes("escape");
    cx.run_until_parked();

    // 实测：输入框把 Escape 吃成自己的 `Escape` 动作，"Dialog" 上下文里
    // 容器绑定的 Cancel 不会被触发，弹窗也不会关。因此凡是把焦点交给
    // 输入框的弹窗，取消必须由宿主显式接 `on_action(&Escape)`。
    assert_eq!(
        hits(&log),
        vec!["input-escape".to_string()],
        "焦点在输入框内时，容器 Cancel 不应白拿"
    );
    assert!(dialog_open(cx), "容器不会自行关闭焦点在输入框里的弹窗");
}

#[test]
fn 焦点在单行输入框内时容器的_enter_不触发() {
    let mut app = TestAppContext::single();
    let cx = probe_window(&mut app);
    let log = new_log();
    open_input_dialog(cx, &log);
    log.borrow_mut().clear();

    cx.simulate_keystrokes("enter");
    cx.run_until_parked();

    // 实测同上：Enter 归输入框的 `Enter` 动作，容器 Confirm 不触发。
    // 提交口径要靠宿主接 `on_action(&Enter)`，与鉴权口令框现有写法一致。
    assert_eq!(
        hits(&log),
        vec!["input-enter".to_string()],
        "焦点在输入框内时，容器 Confirm 不应白拿"
    );
    assert!(dialog_open(cx), "容器不会自行提交焦点在输入框里的弹窗");
}

#[test]
fn enter_在多行输入框内只换行不提交() {
    let mut app = TestAppContext::single();
    let cx = probe_window(&mut app);
    let log = new_log();
    let textarea = open_textarea_dialog(cx, &log);
    log.borrow_mut().clear();

    cx.simulate_keystrokes("enter");
    cx.run_until_parked();

    // 实测：多行框保留换行且不触发提交，正是要的口径；插入位置是光标处，
    // 而 `default_value` 之后光标在开头，所以换行落在行首。
    let value = cx.update(|_, app| textarea.read(app).value().to_string());
    assert_eq!(value, "\nline", "多行框里的 Enter 应当插入换行");
    assert_eq!(
        hits(&log),
        Vec::<String>::new(),
        "多行框里的 Enter 既不得提交，也不得派发输入框的 Enter 动作"
    );
    assert!(dialog_open(cx), "换行不该把弹窗关掉");
}

#[test]
fn ctrl_enter_默认不落到容器的_confirm() {
    let mut app = TestAppContext::single();
    let cx = probe_window(&mut app);
    let log = new_log();
    open_textarea_dialog(cx, &log);
    log.borrow_mut().clear();

    cx.simulate_keystrokes("ctrl-enter");
    cx.run_until_parked();

    // 实测：容器只绑了裸 enter，没有 ctrl-enter。多行框的提交口径
    // （Ctrl+Enter）要宿主在 "Dialog" 上下文里补这条键位。
    assert_eq!(
        hits(&log),
        Vec::<String>::new(),
        "宿主补绑之前，Ctrl+Enter 不应有任何落点"
    );
    assert!(dialog_open(cx), "未补绑时弹窗保持打开");
}

/// 打开一个只有静态正文、焦点留在弹窗外壳的探针弹窗。
///
/// 用于对照：容器自带的 `escape → Cancel`、`enter → Confirm` 只在没有任何
/// 输入控件抢占按键时才谈得上"白拿"。
fn open_bare_dialog(cx: &mut VisualTestContext, log: &Log) {
    cx.update({
        let log = Rc::clone(log);
        move |window, cx| {
            window.open_dialog(
                cx,
                move |dialog: Dialog, _window: &mut Window, _cx: &mut App| {
                    let confirm_log = Rc::clone(&log);
                    let cancel_log = Rc::clone(&log);
                    dialog
                        .title("Probe")
                        .entrance(DialogEntrance::None)
                        .overlay_closable(false)
                        .child("body")
                        .on_ok(move |_, _, _| {
                            mark(&confirm_log, "confirm");
                            true
                        })
                        .on_cancel(move |_, _, _| {
                            mark(&cancel_log, "cancel");
                            true
                        })
                },
            );
        }
    });
    paint(cx);
}

#[test]
fn 焦点在弹窗外壳时_escape_走容器_cancel() {
    let mut app = TestAppContext::single();
    let cx = probe_window(&mut app);
    let log = new_log();
    open_bare_dialog(cx, &log);

    cx.simulate_keystrokes("escape");
    cx.run_until_parked();

    assert_eq!(
        hits(&log),
        vec!["cancel".to_string()],
        "无输入控件抢占时，容器应当白拿 Esc"
    );
    assert!(!dialog_open(cx), "Esc 之后弹窗必须离开弹窗栈");
}

#[test]
fn 焦点在弹窗外壳时_enter_走容器_confirm() {
    let mut app = TestAppContext::single();
    let cx = probe_window(&mut app);
    let log = new_log();
    open_bare_dialog(cx, &log);

    cx.simulate_keystrokes("enter");
    cx.run_until_parked();

    assert_eq!(
        hits(&log),
        vec!["confirm".to_string()],
        "无输入控件抢占时，容器应当白拿 Enter"
    );
    assert!(!dialog_open(cx), "Enter 之后弹窗必须离开弹窗栈");
}

/// 被观察方：模拟 `AppShell`，只在自身 `cx.notify()` 时变脏。
struct CountedHost {
    value: usize,
}

impl Render for CountedHost {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div().size_full().child("host")
    }
}

/// 弹窗体：只读被观察方的状态，并订阅它的变更。
///
/// 这是迁移方案的关键前提：弹窗层由组件库内部状态驱动重建，宿主 notify
/// 不会让 `open_dialog` 的 builder 重跑；只有弹窗体自己是独立实体时，
/// 宿主状态变更才能把新内容画上屏。
struct ObservingBody {
    host: Entity<CountedHost>,
    renders: Rc<Cell<usize>>,
    _observe: Subscription,
}

impl Render for ObservingBody {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.renders.set(self.renders.get() + 1);
        let value = self.host.read(cx).value;
        div().id("observing-body").child(format!("value:{value}"))
    }
}

#[test]
fn 宿主变脏时弹窗体独立重绘而弹窗层不重建() {
    let mut app = TestAppContext::single();
    app.update(|cx| {
        init(cx);
        cx.set_reduce_motion(true);
    });
    let renders = Rc::new(Cell::new(0usize));
    let handles: Rc<RefCell<Option<ProbeEntities>>> = Rc::default();
    let (_root, cx) = app.add_window_view(|window, cx| {
        let host = cx.new(|_| CountedHost { value: 1 });
        let body = cx.new({
            let host = host.clone();
            let renders = Rc::clone(&renders);
            move |cx| {
                let observe = cx.observe(&host, move |_, _, cx| cx.notify());
                ObservingBody {
                    host: host.clone(),
                    renders: Rc::clone(&renders),
                    _observe: observe,
                }
            }
        });
        *handles.borrow_mut() = Some((host, body));
        let view = cx.new(|_| ProbeHost);
        Root::new(view, window, cx)
    });
    let (host, _body) = handles.borrow().clone().expect("弹窗实体应已建立");
    cx.simulate_resize(Size {
        width: px(1000.),
        height: px(800.),
    });
    let body_for_dialog = handles
        .borrow()
        .as_ref()
        .expect("弹窗实体应已建立")
        .1
        .clone();
    cx.update(move |window, cx| {
        window.open_dialog(cx, move |dialog: Dialog, _, _| {
            dialog
                .title("Probe")
                .entrance(DialogEntrance::None)
                .child(body_for_dialog.clone())
        });
        window.draw(cx).clear(cx);
    });
    cx.run_until_parked();
    let first = renders.get();
    assert!(first >= 1, "弹窗体至少要渲染一帧，实际 {first}");

    // 只让被观察方变脏：不碰弹窗栈、不重开弹窗。
    host.update(cx, |state, cx| {
        state.value = 42;
        cx.notify();
    });
    cx.update(|window, cx| {
        window.draw(cx).clear(cx);
    });
    cx.run_until_parked();

    assert!(
        renders.get() > first,
        "宿主变脏后弹窗体必须重绘，否则迁移方案不成立：首轮 {first}，现在 {}",
        renders.get()
    );
}

/// 带工作区键位上下文的外壳：用来验证弹窗打开时工作区快捷键还会不会命中。
struct WorkspaceHost {
    log: Log,
    focus: FocusHandle,
}

impl Render for WorkspaceHost {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        let log = Rc::clone(&self.log);
        div()
            .id("workspace-host")
            .size_full()
            .track_focus(&self.focus)
            .key_context("Workspace")
            .on_action(move |_: &WorkspaceProbe, _, _| mark(&log, "workspace-action"))
            .child("host")
    }
}

#[test]
fn 弹窗打开时工作区上下文的快捷键不再命中() {
    let mut app = TestAppContext::single();
    app.update(|cx| {
        init(cx);
        cx.set_reduce_motion(true);
        cx.bind_keys([KeyBinding::new("ctrl-b", WorkspaceProbe, Some("Workspace"))]);
    });
    let log = new_log();
    let host_focus = app.update(|cx| cx.focus_handle());
    let (_root, cx) = app.add_window_view({
        let log = Rc::clone(&log);
        let focus = host_focus.clone();
        move |window, cx| {
            window.focus(&focus, cx);
            let view = cx.new(|_| WorkspaceHost {
                log: log.clone(),
                focus,
            });
            Root::new(view, window, cx)
        }
    });
    cx.simulate_resize(Size {
        width: px(1000.),
        height: px(800.),
    });
    cx.update(|window, cx| window.draw(cx).clear(cx));
    cx.run_until_parked();

    // 基线：焦点在外壳、没有弹窗时，这条键位必须是通的。
    cx.simulate_keystrokes("ctrl-b");
    cx.run_until_parked();
    assert_eq!(
        hits(&log),
        vec!["workspace-action".to_string()],
        "基线不成立：键位绑定或焦点路径有问题"
    );

    log.borrow_mut().clear();
    let log2 = Rc::clone(&log);
    cx.update(move |window, cx| {
        window.open_dialog(cx, move |dialog: Dialog, _, _| {
            let cancel_log = Rc::clone(&log2);
            dialog
                .title("Probe")
                .entrance(DialogEntrance::None)
                .child("body")
                .on_cancel(move |_, _, _| {
                    mark(&cancel_log, "cancel");
                    true
                })
        });
    });
    cx.run_until_parked();
    cx.update(|window, cx| window.draw(cx).clear(cx));

    // 弹窗打开后焦点被容器接管：工作区上下文的键位不该再命中。
    // 命中就说明迁移前 "Workspace && !Overlay" 那条排除仍有必要。
    cx.simulate_keystrokes("ctrl-b");
    cx.run_until_parked();
    assert_eq!(
        hits(&log),
        Vec::<String>::new(),
        "弹窗打开时仍命中工作区快捷键，需要给弹窗补遮罩上下文排除"
    );
}
