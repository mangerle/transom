//! 容器工作区列表刷新动作：单分类刷新与全量并发预热刷新。

use super::*;

/// 引擎事件连发时的合并窗口；把同一批变更收敛成一次列表请求。
const REFRESH_DEBOUNCE: std::time::Duration = std::time::Duration::from_millis(250);

impl ContainerWorkspace {
    /// 引擎事件触发的防抖刷新：窗口内只保留最后一次请求。
    ///
    /// 任务句柄挂在实体状态上：新事件到达时替换旧任务，旧计时器被丢弃，
    /// 同一批变更只会收敛成一次列表请求。
    pub(crate) fn schedule_refresh(&mut self, cx: &mut Context<Self>) {
        let task = cx.spawn(async move |this, cx| {
            cx.background_executor().timer(REFRESH_DEBOUNCE).await;
            let _ = this.update(cx, |workspace, cx| {
                workspace.refresh_section();
                cx.notify();
            });
        });
        self.state.refresh_task = Some(task);
    }

    /// 刷新当前选中分类的列表。
    pub(crate) fn refresh_section(&mut self) {
        let tab_id = self.tab_id;
        let section = self.state.section;
        self.state.busy = true;
        let request_id = ContainerRequestId::new();
        let command = match section {
            ContainerSection::Containers | ContainerSection::Compose => {
                ContainerCommand::ListContainers { tab_id, request_id }
            }
            ContainerSection::Images => ContainerCommand::ListImages { tab_id, request_id },
            ContainerSection::Volumes => ContainerCommand::ListVolumes { tab_id, request_id },
            ContainerSection::Networks => ContainerCommand::ListNetworks { tab_id, request_id },
        };
        self.send_tracked(request_id, command);
        // 卷分类要同时拉容器列表：「在用 / 空闲」与「仅未使用卷」都靠容器的挂载
        // 明细算，缺了这份数据会把在用卷说成空闲，用户据此删除就是事故。
        if section == ContainerSection::Volumes {
            let usage_request = ContainerRequestId::new();
            self.send_tracked(
                usage_request,
                ContainerCommand::ListContainers {
                    tab_id,
                    request_id: usage_request,
                },
            );
        }
    }

    /// 全量并发预热刷新容器工作区所有分类（容器、镜像、数据卷、网络）。
    ///
    /// 设计原理（Why）：
    /// - 实现初衷：初次建立连接或点击全局刷新时，一次性并发拉取所有分类数据，
    ///   避免顶栏分类标签（Tabs）中的数字显示为 0 且只有切换进去后才刷新；
    /// - 核心优势：标签数字进入即准确，且用户切换分类时实现秒开零等待；
    /// - 代价与局限：初次连接时并发执行 4 个轻量列表查询，但在普通 Docker/Podman
    ///   引擎上耗时通常在几十毫秒以内，开销极小。
    pub(crate) fn refresh_all(&mut self) {
        let tab_id = self.tab_id;
        self.state.busy = true;

        // 1. 容器与编排项目
        let req_containers = ContainerRequestId::new();
        self.send_tracked(
            req_containers,
            ContainerCommand::ListContainers {
                tab_id,
                request_id: req_containers,
            },
        );

        // 2. 镜像
        let req_images = ContainerRequestId::new();
        self.send_tracked(
            req_images,
            ContainerCommand::ListImages {
                tab_id,
                request_id: req_images,
            },
        );

        // 3. 数据卷
        let req_volumes = ContainerRequestId::new();
        self.send_tracked(
            req_volumes,
            ContainerCommand::ListVolumes {
                tab_id,
                request_id: req_volumes,
            },
        );

        // 4. 网络
        let req_networks = ContainerRequestId::new();
        self.send_tracked(
            req_networks,
            ContainerCommand::ListNetworks {
                tab_id,
                request_id: req_networks,
            },
        );
    }
}
