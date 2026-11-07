//! 内部完成事件的处理：连接接入、SFTP/转发提交与断开通知。

mod connected;
mod container;
mod disconnected;
mod forward;
mod sftp;

use connected::handle_connected;
use container::{
    handle_container_connected, handle_container_finished, handle_container_pull_finished,
};
use disconnected::handle_disconnected;
use forward::handle_forward_ready;
use sftp::handle_sftp_ready;

use crate::controller::Internal;
use crate::controller::state::{ControllerCtx, ControllerState};

/// 分发一条内部完成事件。
pub(super) async fn handle_internal(
    state: &mut ControllerState,
    ctx: &ControllerCtx,
    internal: Internal,
) {
    match internal {
        Internal::Connected { .. } => handle_connected(state, ctx, internal).await,
        Internal::Disconnected {
            tab_id,
            connection_id,
        } => handle_disconnected(state, ctx, tab_id, connection_id).await,
        Internal::SftpReady { .. } => handle_sftp_ready(state, ctx, internal).await,
        Internal::ForwardReady { .. } => handle_forward_ready(state, ctx, internal).await,
        Internal::ContainerConnected { .. } => {
            handle_container_connected(state, ctx, internal).await
        }
        Internal::ContainerTaskFinished { .. } => handle_container_finished(state, ctx, internal),
        Internal::ContainerPullTaskFinished { .. } => {
            handle_container_pull_finished(state, internal)
        }
    }
}
