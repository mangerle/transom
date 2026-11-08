//! 容器重命名输入条：打开、取消与提交。

use super::*;

impl AppShell {
    /// Esc 关闭容器侧的浮层：先关表单，再退全屏。
    ///
    /// 为什么挂在浮层自己的按键回调上而不是全局键位：终端里 Esc 是 vim 等程序的
    /// 正常输入，全局绑一个「关浮层」的键会在没有浮层时也吃掉这一次按键。浮层
    /// 存在时它的元素在焦点路径上，按键自然只到这里；全屏视图里正处于终端页签时
    /// 直接放行，Esc 仍然只交给容器内的程序。
    pub(crate) fn on_container_overlay_key(
        &mut self,
        event: &KeyDownEvent,
        fullscreen_tab: Option<TabId>,
        cx: &mut Context<Self>,
    ) {
        if !event.keystroke.key.eq_ignore_ascii_case("escape") {
            return;
        }
        let Some(tab) = fullscreen_tab.filter(|tab| self.state.containers.fullscreen == Some(*tab))
        else {
            return;
        };
        // 终端页签里 Esc 是 vim 等程序的正常输入，这时不能把整个工作区收掉；
        // 其余页签（列表/日志/统计/文件）Esc 的语义就是「关掉眼前的全屏」。
        let in_terminal = self
            .state
            .container_workspace(tab)
            .is_some_and(|workspace| {
                workspace.read(cx).state.detail_tab == ContainerDetailTab::Terminal
            });
        if in_terminal {
            return;
        }
        self.state.containers.fullscreen = None;
        cx.notify();
    }

    /// 打开容器改名条。
    pub(crate) fn open_container_rename(
        &mut self,
        tab_id: TabId,
        id: String,
        current: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let input = cx.new(|cx| InputState::new(window, cx).default_value(current));
        self.with_container_mut(tab_id, cx, |workspace| {
            workspace.rename_container = Some((id, input));
        });
        cx.notify();
    }

    /// 取消改名。
    pub(crate) fn cancel_container_rename(&mut self, tab_id: TabId, cx: &mut Context<Self>) {
        self.with_container_mut(tab_id, cx, |workspace| {
            workspace.rename_container = None;
        });
        cx.notify();
    }

    /// 提交改名：名称留空或与原名相同都不下发，避免无意义的请求与 409。
    pub(crate) fn submit_container_rename(
        &mut self,
        tab_id: TabId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some((id, input)) = self
            .with_container(tab_id, cx, |workspace| workspace.rename_container.clone())
            .flatten()
        else {
            return;
        };
        let name = input.read(cx).value().trim().to_string();
        let same = self
            .with_container(tab_id, cx, |workspace| {
                workspace
                    .containers
                    .iter()
                    .any(|item| item.id == id && item.name == name)
            })
            .unwrap_or(false);
        if name.is_empty() || same {
            self.show_toast(
                rust_i18n::t!("container.rename.invalid", name = name).to_string(),
                false,
                window,
                cx,
            );
            return;
        }
        let request_id = ContainerRequestId::new();
        self.with_container_mut(tab_id, cx, |workspace| workspace.busy = true);
        self.send_tracked(
            tab_id,
            request_id,
            ContainerCommand::RenameContainer {
                tab_id,
                request_id,
                container: id,
                name,
            },
        );
        cx.notify();
    }
}
