//! 容器资源动态更新状态管理：打开、取消与提交。
//!
//! 设计原理（Why）：通过居中模态弹窗支持用户独立或组合调整内存配额与 CPU 核数限制，
//! 提交后调用底层更新并在成功后自动刷新容器状态与详情。
//!
//! 动作归属（Why）：解析输入、校验与投递命令都只依赖本标签页状态，
//! 全部落在 [`ContainerWorkspace`] 上；外壳不再中转。

use super::*;
use gpui_kit::component::input::InputState;
use transom_core::ContainerUpdateSpec;

impl ContainerWorkspace {
    /// 打开容器资源更新弹窗。
    pub(crate) fn open_container_update(
        &mut self,
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
        self.state.update_container = Some((name, mem_input, cpu_input));
        cx.notify();
    }

    /// 取消容器资源更新。
    pub(crate) fn cancel_container_update(&mut self, cx: &mut Context<Self>) {
        self.state.update_container = None;
        cx.notify();
    }

    /// 提交容器资源更新。
    ///
    /// 空白输入表示不限制对应资源；无法解析的值同样按「不限制」处理，
    /// 与迁移前的宽松口径一致。解析完成后关闭弹窗并投递更新命令。
    pub(crate) fn submit_container_update(&mut self, cx: &mut Context<Self>) {
        let Some((name, mem_input, cpu_input)) = self.state.update_container.clone() else {
            return;
        };

        let mem_text = mem_input.read(cx).text().to_string();
        let mem_text = mem_text.trim();
        let cpu_text = cpu_input.read(cx).text().to_string();
        let cpu_text = cpu_text.trim();

        let memory_bytes = if mem_text.is_empty() {
            None
        } else {
            mem_text
                .parse::<u64>()
                .ok()
                .map(|mb| mb.saturating_mul(1024 * 1024))
        };

        let nano_cpus = if cpu_text.is_empty() {
            None
        } else {
            cpu_text.parse::<f64>().ok().and_then(|cores| {
                if cores > 0.0 {
                    Some((cores * 1_000_000_000.0) as u64)
                } else {
                    None
                }
            })
        };

        let spec = ContainerUpdateSpec {
            memory_bytes,
            nano_cpus,
            restart_policy: None,
        };

        self.cancel_container_update(cx);
        self.update_container_resources(name, spec, cx);
    }

    /// 投递容器资源更新命令并标记忙碌。
    fn update_container_resources(
        &mut self,
        container: String,
        spec: ContainerUpdateSpec,
        cx: &mut Context<Self>,
    ) {
        let request_id = ContainerRequestId::new();
        self.state.busy = true;
        self.state.notice = Some(
            rust_i18n::t!(
                "container.action.running",
                action = rust_i18n::t!("container.action.update")
            )
            .to_string(),
        );
        self.send_tracked(
            request_id,
            ContainerCommand::UpdateContainer {
                tab_id: self.tab_id,
                request_id,
                container,
                spec,
            },
        );
        cx.notify();
    }
}
