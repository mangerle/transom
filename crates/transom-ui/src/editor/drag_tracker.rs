//! 编辑面板拖拽的窗口级事件追踪器。
//!
//! 拖拽中指针会离开面板乃至窗口边界，而普通元素监听带"指针命中"门槛，
//! 收不到这些事件——曾经依赖根元素监听收尾，结果指针在窗口外松开、或抬起
//! 落在浮层上时 mouse_up 丢失，拖拽位残留，面板此后一直跟随鼠标。
//!
//! 窗口级监听（`window.on_mouse_event`）不受命中门槛限制，任何到达窗口的
//! move/up 都能收到，但它只能在元素 paint 阶段注册。因此本元素不绘制任何
//! 内容，仅作为挂载点存在于根容器中，模式与 gpui-component 的
//! `DockResizeTracker` 一致。Linux 的 X11/Wayland 在按下期间有隐式抓取，
//! 释放事件即使落在窗口外也会投递到本窗口，收尾由此闭环。

use crate::app::AppShell;
use gpui_kit::*;

/// 无视觉输出的 paint 挂载点：每帧注册窗口级的拖拽位移与收尾监听。
pub(crate) struct EditorDragTracker {
    shell: Entity<AppShell>,
}

impl EditorDragTracker {
    pub(crate) fn new(shell: Entity<AppShell>) -> Self {
        Self { shell }
    }
}

impl IntoElement for EditorDragTracker {
    type Element = Self;

    fn into_element(self) -> Self::Element {
        self
    }
}

impl Element for EditorDragTracker {
    type RequestLayoutState = ();
    type PrepaintState = ();

    fn id(&self) -> Option<ElementId> {
        None
    }

    fn source_location(&self) -> Option<&'static std::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, Self::RequestLayoutState) {
        (window.request_layout(Style::default(), None, cx), ())
    }

    fn prepaint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        _: Bounds<Pixels>,
        _: &mut Self::RequestLayoutState,
        _: &mut Window,
        _: &mut App,
    ) -> Self::PrepaintState {
    }

    fn paint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        _: Bounds<Pixels>,
        _: &mut Self::RequestLayoutState,
        _: &mut Self::PrepaintState,
        window: &mut Window,
        _: &mut App,
    ) {
        let shell = self.shell.clone();
        window.on_mouse_event(move |event: &MouseMoveEvent, phase, window, cx| {
            if !phase.bubble() {
                return;
            }
            shell.update(cx, |this, cx| {
                if this.editor_drag_in_progress() {
                    this.drag_editor_panels(event.position, window.viewport_size(), cx);
                }
            });
        });
        let shell = self.shell.clone();
        window.on_mouse_event(move |_: &MouseUpEvent, phase, _, cx| {
            if !phase.bubble() {
                return;
            }
            shell.update(cx, |this, cx| {
                if this.editor_drag_in_progress() {
                    this.end_editor_panel_drag(cx);
                }
            });
        });
    }
}

impl AppShell {
    /// 是否有面板处于拖拽中；窗口级监听以此作廉价闸门，
    /// 避免每次鼠标移动都推进无意义的重绘。
    pub(crate) fn editor_drag_in_progress(&self) -> bool {
        self.editor_panels
            .iter()
            .any(|item| item.drag_grab.is_some())
    }
}
