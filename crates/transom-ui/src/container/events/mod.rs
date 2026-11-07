//! 容器事件分发：按事件类别交给各自处理函数。
//!
//! 分发器只做分类，具体状态更新在 connection / listing / file_stats /
//! operations 四个子模块，以及已有的 pull / result / system / terminal。

mod connection;
mod file_stats;
mod listing;
mod operations;
mod pull;
mod result;
mod system;
mod terminal;

use super::*;

impl AppShell {
    /// 容器事件分发：先按请求 ID 找到工作区，再更新对应字段。
    pub(crate) fn dispatch_container_event(
        &mut self,
        event: ContainerAppEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match event {
            e @ (ContainerAppEvent::Connected { .. }
            | ContainerAppEvent::ConnectFailed { .. }
            | ContainerAppEvent::EnginesDiscovered { .. }
            | ContainerAppEvent::TestFinished { .. }
            | ContainerAppEvent::Disconnected { .. }) => {
                self.handle_connection_event(e, window, cx);
            }
            e @ (ContainerAppEvent::ContainersListed { .. }
            | ContainerAppEvent::ContainerInspected { .. }
            | ContainerAppEvent::ImagesListed { .. }
            | ContainerAppEvent::ImageInspected { .. }
            | ContainerAppEvent::VolumesListed { .. }
            | ContainerAppEvent::VolumeInspected { .. }
            | ContainerAppEvent::NetworksListed { .. }
            | ContainerAppEvent::NetworkInspected { .. }
            | ContainerAppEvent::ComposeProjectsListed { .. }
            | ContainerAppEvent::PodsListed { .. }
            | ContainerAppEvent::QuadletsListed { .. }
            | ContainerAppEvent::ProcessesListed { .. }) => {
                self.handle_listing_event(e, window, cx);
            }
            e @ (ContainerAppEvent::ExecReady { .. }
            | ContainerAppEvent::LogsChunk { .. }
            | ContainerAppEvent::LogsClosed { .. }
            | ContainerAppEvent::StatsUpdated { .. }
            | ContainerAppEvent::StatsUnavailable { .. }
            | ContainerAppEvent::DirListed { .. }
            | ContainerAppEvent::FilePreview { .. }
            | ContainerAppEvent::TransferProgress { .. }
            | ContainerAppEvent::BuildProgress { .. }) => {
                self.handle_file_stats_event(e, window, cx);
            }
            e @ (ContainerAppEvent::VolumesPruned { .. }
            | ContainerAppEvent::ImagesPruned { .. }
            | ContainerAppEvent::NetworksPruned { .. }
            | ContainerAppEvent::ActionFinished { .. }
            | ContainerAppEvent::TransferFinished { .. }
            | ContainerAppEvent::Failed { .. }
            | ContainerAppEvent::ImagesSearched { .. }
            | ContainerAppEvent::ImageTagsListed { .. }
            | ContainerAppEvent::PullTaskProgress { .. }
            | ContainerAppEvent::PullTaskFinished { .. }
            | ContainerAppEvent::ContainerDiffed { .. }
            | ContainerAppEvent::SystemDfLoaded { .. }
            | ContainerAppEvent::SystemPruned { .. }
            | ContainerAppEvent::EngineEventReceived { .. }) => {
                self.handle_operations_event(e, window, cx);
            }
        }
        cx.notify();
    }
}
