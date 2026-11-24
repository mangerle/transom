//! RDP 经由 SSH 跳板机隧道转发 3389 端口的轻量封装。
//!
//! 设计原理（Why）：复用 `transom-net` 现有的多跳 SSH 跳板链与本地端口转发通道，
//! 在本机 `127.0.0.1:0` 绑定临时端口并转发至远端 RDP 目标，而 `transom-rdp` 保持
//! 原始目标主机名用于 TLS SNI、CredSSP 与 `known_hosts` 校验。

use std::sync::{Arc, OnceLock};
use tokio::runtime::Runtime;
use transom_core::{
    BindAddress, ForwardKind, ForwardRule, KnownHostsStore, OutboundProxy, SshHop, TargetAddress,
};
use transom_net::{ForwardTask, PasswordPrompter, SshRemote, SshRemoteConnectParams};

static TUNNEL_RUNTIME: OnceLock<Result<Runtime, String>> = OnceLock::new();

fn tunnel_runtime() -> Result<&'static Runtime, String> {
    match TUNNEL_RUNTIME.get_or_init(|| {
        tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .thread_name("transom-rdp-ssh-tunnel")
            .enable_all()
            .build()
            .map_err(|e| format!("初始化 RDP SSH 隧道运行时失败: {e}"))
    }) {
        Ok(rt) => Ok(rt),
        Err(msg) => Err(msg.clone()),
    }
}

/// 持有 SSH 隧道与本地端口转发任务生命周期的 RAII 守卫。
pub(crate) struct RdpTunnelGuard {
    _forward_task: ForwardTask,
    _remote: SshRemote,
}

/// 建立到跳板机链的 SSH 连接，并在本机 `127.0.0.1:0` 开启转发至 `target_host:target_port` 的隧道。
pub(crate) async fn open_ssh_tunnel_for_rdp(
    mut hops: Vec<SshHop>,
    proxy: OutboundProxy,
    known_hosts: Arc<dyn KnownHostsStore>,
    allow_sha1_publickey_auth: bool,
    target_host: String,
    target_port: u16,
) -> Result<(u16, RdpTunnelGuard), String> {
    let Some(last_hop) = hops.pop() else {
        return Err("SSH 隧道配置为空".to_string());
    };

    let rt = tunnel_runtime()?;
    let (tx, rx) = tokio::sync::oneshot::channel();
    rt.spawn(async move {
        let res = establish_tunnel_on_tokio(
            hops,
            last_hop,
            proxy,
            known_hosts,
            allow_sha1_publickey_auth,
            target_host,
            target_port,
        )
        .await;
        let _ = tx.send(res);
    });

    rx.await
        .map_err(|_| "RDP SSH 隧道建立任务异常中断".to_string())?
}

async fn establish_tunnel_on_tokio(
    hops: Vec<SshHop>,
    last_hop: SshHop,
    proxy: OutboundProxy,
    known_hosts: Arc<dyn KnownHostsStore>,
    allow_sha1_publickey_auth: bool,
    target_host: String,
    target_port: u16,
) -> Result<(u16, RdpTunnelGuard), String> {
    let prompter = Arc::new(PasswordPrompter::new(
        last_hop
            .secrets
            .password
            .as_ref()
            .map(|p| p.as_str())
            .unwrap_or(""),
    ));
    let remote = transom_net::connect_ssh_remote(SshRemoteConnectParams {
        connection: &last_hop.connection,
        jumps: &hops,
        credentials: last_hop.secrets,
        prompter,
        known_hosts,
        proxy,
        allow_sha1_publickey_auth,
    })
    .await
    .map_err(|e| format!("连接 RDP SSH 隧道跳板机失败: {e}"))?;

    let rule = ForwardRule {
        id: transom_core::ForwardId::new(),
        name: format!("rdp-tunnel-{target_host}:{target_port}"),
        kind: ForwardKind::Local {
            bind: BindAddress {
                host: "127.0.0.1".to_string(),
                port: 0,
            },
            target: TargetAddress {
                host: target_host,
                port: target_port,
            },
        },
        enabled: true,
    };

    let task = transom_net::start_forward(&remote, &rule)
        .await
        .map_err(|e| format!("启动 RDP SSH 端口转发隧道失败: {e}"))?;
    let local_port = task
        .bound_port
        .ok_or_else(|| "RDP SSH 隧道未分配本地监听端口".to_string())?;

    log::info!("已建立 RDP SSH 本地转发隧道: 127.0.0.1:{local_port}");
    Ok((
        local_port,
        RdpTunnelGuard {
            _forward_task: task,
            _remote: remote,
        },
    ))
}
