//! 弹窗栈影子记录、对账、压栈与取消/提交动作。

use super::body::ModalBody;
use super::*;

pub(super) fn cancel_modal(
    app: &Entity<AppShell>,
    kind: ModalKind,
    window: &mut Window,
    cx: &mut App,
) -> bool {
    let closed = app.update(cx, |shell, cx| kind.cancel(shell, window, cx));
    app.update(cx, |shell, cx| {
        if closed {
            shell.state.open_modals.drop(kind);
        }
        cx.notify();
    });
    closed
}

pub(super) fn commit_modal(
    app: &Entity<AppShell>,
    kind: ModalKind,
    window: &mut Window,
    cx: &mut App,
) -> bool {
    let closed = app.update(cx, |shell, cx| kind.commit(shell, window, cx));
    app.update(cx, |shell, cx| {
        if closed {
            shell.state.open_modals.drop(kind);
        }
        cx.notify();
    });
    closed
}

pub(super) fn close_layer(app: &Entity<AppShell>, kind: ModalKind, cx: &mut App) {
    app.update(cx, |shell, cx| {
        shell.state.open_modals.drop(kind);
        cx.notify();
    });
}

#[derive(Default)]
pub(crate) struct OpenModals {
    /// 每层记录三样：种类、弹窗体实体、打开时的文本基线。
    layers: Vec<ModalLayer>,
}

/// 影子栈的一层。
pub(crate) struct ModalLayer {
    kind: ModalKind,
    body: Entity<ModalBody>,
    /// 打开那一刻的文本项快照，脏判定与它比对。
    baseline: Vec<String>,
}

impl OpenModals {
    /// 摘掉某一层影子记录（容器已自行弹栈、或即将由调用方弹栈时调用）。
    pub(super) fn drop(&mut self, kind: ModalKind) {
        if let Some(index) = self.layers.iter().position(|layer| layer.kind == kind) {
            self.layers.remove(index);
        }
    }

    /// 取某一层的文本基线；该层不存在时返回空切片（此时脏判定恒为假）。
    pub(super) fn baseline(&self, kind: ModalKind) -> &[String] {
        self.layers
            .iter()
            .find(|layer| layer.kind == kind)
            .map_or(&[][..], |layer| layer.baseline.as_slice())
    }
}

impl AppShell {
    /// 当前应当打开的弹窗栈，按层序由下到上。
    fn desired_modal_stack(&self) -> Vec<ModalKind> {
        ModalKind::ORDER
            .iter()
            .copied()
            .filter(|kind| kind.is_open(self))
            .collect()
    }

    /// 把容器弹窗栈对账到期望栈。
    ///
    /// 为什么放在渲染里而不是各个开关点：状态变更入口分散在菜单、快捷键、
    /// 拖放与异步回调等二十多处，逐个补开关必然漏；渲染是唯一的汇聚点。
    /// 为什么要 `defer`：`open_dialog` 会移交焦点并通知组件层，在渲染过程中
    /// 改这些状态会踩借用与重入，延到绘制之后再执行。
    pub(crate) fn reconcile_dialogs(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let desired = self.desired_modal_stack();
        let actual: Vec<ModalKind> = self
            .state
            .open_modals
            .layers
            .iter()
            .map(|layer| layer.kind)
            .collect();
        if desired == actual {
            return;
        }

        // 公共前缀原地保留；一旦中间层被移除或需要在中间插入，只能整栈
        // 关掉再按期望顺序重开——这是容器栈的真实代价。
        let common = desired
            .iter()
            .zip(actual.iter())
            .take_while(|(want, have)| want == have)
            .count();
        let extra = actual.len() - common;
        let missing = desired[common..].to_vec();

        let app = cx.entity().clone();
        let deferred_app = app.clone();
        let deferred_missing = missing.clone();
        window.defer(cx, move |window, cx| {
            for _ in 0..extra {
                window.close_dialog(cx);
            }
            for kind in deferred_missing {
                deferred_app.update(cx, |shell, cx| shell.push_dialog(kind, window, cx));
            }
        });
        // 影子栈先按期望落定：延后执行的那批开关完成后即与之一致。
        self.state.open_modals.layers.truncate(common);
        for kind in missing {
            // 基线在开弹窗这一刻取：此后任何文本变化都算未提交内容。
            let baseline = kind.field_values(self, cx);
            let body_app = app.clone();
            let body = cx.new(move |cx| {
                let observe = cx.observe(&body_app, move |_, _, cx| cx.notify());
                ModalBody {
                    kind,
                    app: body_app,
                    pending_focus: true,
                    _observe: observe,
                }
            });
            self.state.open_modals.layers.push(ModalLayer {
                kind,
                body,
                baseline,
            });
        }
    }

    /// 压入一层弹窗：外壳只负责容器属性，内容交给弹窗体。
    fn push_dialog(&mut self, kind: ModalKind, window: &mut Window, cx: &mut Context<Self>) {
        let Some(body) = self
            .state
            .open_modals
            .layers
            .iter()
            .find(|layer| layer.kind == kind)
            .map(|layer| layer.body.clone())
        else {
            log::error!("弹窗对账失序：种类 {:?} 缺少弹窗体实体，跳过这一层", kind);
            return;
        };
        let app = cx.entity().clone();
        let width = kind.spec(window).width;
        window.open_dialog(cx, move |dialog, _window, _cx| {
            let cancel_app = app.clone();
            let commit_app = app.clone();

            dialog
                .w(width)
                .max_w(width)
                // 右上角关闭件保留：用户要一个看得见的出口。遮罩点击关闭与
                // 入场动画按拍板关掉——对本应用的高危确认框，点外面就关是误操作源。
                .close_button(true)
                // 容器遮罩按 `window_paddings` 内缩定位：客户端装饰下实测内缩
                // 20px，会把压暗层上沿留在自绘顶栏中部。压暗改由宿主画满窗面，
                // 见 `AppShell::dialog_dim`。
                .overlay(false)
                .overlay_closable(false)
                .entrance(DialogEntrance::None)
                .child(body.clone())
                .on_cancel(move |_, window, cx| {
                    let closed = cancel_modal(&cancel_app, kind, window, cx);
                    if closed {
                        close_layer(&cancel_app, kind, cx);
                    }
                    closed
                })
                .on_ok(move |_, window, cx| {
                    let closed = commit_modal(&commit_app, kind, window, cx);
                    if closed {
                        close_layer(&commit_app, kind, cx);
                    }
                    closed
                })
        });
    }
}

impl AppShell {
    /// 弹窗栈的压暗层：只负责视觉压暗与命中阻断，不承载弹窗本体。
    ///
    /// 为什么要在宿主这一侧画：弹窗层由组件库在窗口根上渲染，其遮罩按窗口
    /// 内缩定位，客户端装饰下实测四周各留 20px；而应用顶栏从窗面 y=0 起画，
    /// 于是压暗层上沿正好卡在顶栏中间。宿主元素铺满整个窗面，层序在编辑面板
    /// 之上、弹窗层之下，与迁移前的自绘遮罩一致。
    pub(crate) fn dialog_dim(&self, p: &Palette) -> Option<AnyElement> {
        if self.state.open_modals.layers.is_empty() {
            return None;
        }
        Some(
            div()
                .absolute()
                .top_0()
                .left_0()
                .size_full()
                .bg(p.overlay)
                // 与迁移前一致：阻断下层命中测试，否则点弹窗会同时触发终端与
                // 文件列表的鼠标处理并抢走焦点。点遮罩本身不做关闭动作（已拍板）。
                .occlude()
                .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                .on_mouse_down(MouseButton::Right, |_, _, cx| cx.stop_propagation())
                .into_any_element(),
        )
    }
}

impl AppShell {
    /// 关闭一个 SFTP 弹窗：清状态并把焦点还给活动面板。
    ///
    /// 七个 SFTP 弹窗的取消动作只差"清哪个字段"，用闭包参数而不是七个
    /// 同形方法，避免复制七遍同样的焦点与重绘逻辑。
    pub(crate) fn close_sftp_modal<F>(
        &mut self,
        clear: F,
        window: &mut Window,
        cx: &mut Context<AppShell>,
    ) -> bool
    where
        F: FnOnce(&mut crate::sftp::SftpModals),
    {
        clear(&mut self.state.sftp_modals);
        self.focus_active_sftp_panel(window, cx);
        cx.notify();
        true
    }
}
