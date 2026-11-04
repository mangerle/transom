//! 容器资源动态更新状态管理：打开、取消与提交。
//!
//! 设计原理（Why）：通过居中模态弹窗支持用户独立或组合调整内存配额与 CPU 核数限制，
//! 提交后调用底层更新并在成功后自动刷新容器状态与详情。

use super::*;
use crate::app::AppShell;
use gpui_kit::component::input::InputState;
use transom_core::{ContainerUpdateSpec, TabId};

impl AppShell {
    /// 打开容器资源更新弹窗。
    pub(crate) fn open_container_update(
        &mut self,
        tab_id: TabId,
        name: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let mem_input = cx.new(|cx| {
            InputState::new(window, cx).placeholder(
                rust_i18n::t!("container.update_resources.memory_placeholder").to_string(),
            )
        });
        let cpu_input = cx.new(|cx| {
            InputState::new(window, cx).placeholder(
                rust_i18n::t!("container.update_resources.cpu_placeholder").to_string(),
            )
        });
        if let Some(workspace) = self.state.container_workspace_mut(tab_id) {
            workspace.update_container = Some((name, mem_input, cpu_input));
        }
        cx.notify();
    }

    /// 取消容器资源更新。
    pub(crate) fn cancel_container_update(&mut self, tab_id: TabId, cx: &mut Context<Self>) {
        if let Some(workspace) = self.state.container_workspace_mut(tab_id) {
            workspace.update_container = None;
        }
        cx.notify();
    }

    /// 提交容器资源更新。
    pub(crate) fn submit_container_update(&mut self, tab_id: TabId, cx: &mut Context<Self>) {
        let Some((name, mem_input, cpu_input)) = self
            .state
            .container_workspace(tab_id)
            .and_then(|workspace| workspace.update_container.clone())
        else {
            return;
        };

        let mem_text = mem_input.read(cx).text().to_string();
        let mem_text = mem_text.trim();
        let cpu_text = cpu_input.read(cx).text().to_string();
        let cpu_text = cpu_text.trim();

        let memory_bytes = if mem_text.is_empty() {
            None
        } else if let Ok(mb) = mem_text.parse::<u64>() {
            Some(mb.saturating_mul(1024 * 1024))
        } else {
            None
        };

        let nano_cpus = if cpu_text.is_empty() {
            None
        } else if let Ok(cores) = cpu_text.parse::<f64>() {
            if cores > 0.0 {
                Some((cores * 1_000_000_000.0) as u64)
            } else {
                None
            }
        } else {
            None
        };

        let spec = ContainerUpdateSpec {
            memory_bytes,
            nano_cpus,
            restart_policy: None,
        };

        self.cancel_container_update(tab_id, cx);
        self.update_container_resources(tab_id, name, spec, cx);
    }
}
