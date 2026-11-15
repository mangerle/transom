//! 主机批量执行、监控采样与公钥部署命令处理。

use super::super::host_exec::run_batch;
use super::super::lifecycle::{KeyDeployParams, deploy_key};
use super::super::samplers;
use super::super::state::{ControllerCtx, ControllerState};
use transom_core::HostCommand;

pub(super) async fn handle_host_command(
    state: &mut ControllerState,
    ctx: &ControllerCtx,
    command: HostCommand,
) {
    match command {
        HostCommand::SetStatsSampling { enabled } => {
            state
                .stats_sampling
                .store(enabled, std::sync::atomic::Ordering::Relaxed);
        }
        HostCommand::DeployPublicKey {
            tab_id,
            connection_id,
            key_name,
            public_key,
        } => {
            deploy_key(KeyDeployParams {
                sessions: &mut state.sessions,
                event_tx: &ctx.event_tx,
                tab_id,
                connection_id,
                key_name,
                public_key,
            })
            .await;
        }
        HostCommand::RunHostActions { actions } => {
            state.host_action_tasks.spawn(run_batch(
                actions,
                ctx.event_tx.clone(),
                ctx.action_permits.clone(),
            ));
        }
        HostCommand::SampleProcesses {
            tab_id,
            connection_id,
        } => {
            state
                .monitor_set
                .spawn(samplers::run_process_sample(samplers::ProcessTarget {
                    remote: state.current_remote(tab_id, connection_id),
                    tab_id,
                    connection_id,
                    event_tx: ctx.event_tx.clone(),
                    local: ctx.local_processes.clone(),
                }));
        }
        HostCommand::SampleProcessCount {
            tab_id,
            connection_id,
        } => {
            state
                .monitor_set
                .spawn(samplers::run_process_count(samplers::ProcessTarget {
                    remote: state.current_remote(tab_id, connection_id),
                    tab_id,
                    connection_id,
                    event_tx: ctx.event_tx.clone(),
                    local: ctx.local_processes.clone(),
                }));
        }
        HostCommand::KillProcess {
            tab_id,
            connection_id,
            pid,
            force,
        } => {
            state.monitor_set.spawn(samplers::run_process_kill(
                samplers::ProcessTarget {
                    remote: state.current_remote(tab_id, connection_id),
                    tab_id,
                    connection_id,
                    event_tx: ctx.event_tx.clone(),
                    local: ctx.local_processes.clone(),
                },
                pid,
                force,
            ));
        }
    }
}
