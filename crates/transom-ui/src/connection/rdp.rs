//! RDP 远程桌面会话的异步凭据拉取、SSH 跳板隧道建立与 TLS TOFU 指纹校验逻辑。

use super::secrets::collect_jump_hops;
use crate::app::{AppShell, AppState, TabStatus, WorkspaceTab, app_settings, spawn_blocking_local};
use crate::rdp::{RdpView, RdpViewEvent, tunnel};
use gpui_kit::{AppContext, Context, Entity};
use std::sync::Arc;
use transom_core::{
    AppError, ErrorCode, HostKeyInfo, HostKeyStatus, KnownHostsStore, OutboundProxy, RdpConnection,
    SecretKey, SecretKind, SecretOwner, SecretStore, Session, TabId,
};
use transom_rdp::{SessionConfig, TlsCertVerifier, TlsCertificateInfo, TlsVerifyOutcome};

/// 打开 RDP 远程桌面会话并将其作为独立图形标签挂入工作区。
///
/// 设计原理（Why）：钥匙串读取可能涉及平台 IPC 阻塞，严禁在 UI 线程同步等待；
/// 此处先挂载处于 `Connecting` 状态的 `RdpView`，再经 `spawn_blocking_local` 异步
/// 拉取凭据并启动连接；重连路径复用同一异步启动器。
pub(super) fn open_rdp_session(
    app_state: &mut AppState,
    tab_id: TabId,
    session: Session,
    rdp: RdpConnection,
    cx: &mut Context<AppShell>,
) {
    let view = cx.new(|cx| RdpView::new_pending(!rdp.audio, rdp.dynamic_resolution, cx));
    let secret_store = app_state.runtime.services.secret_store.clone();
    let known_hosts = app_state.runtime.services.known_hosts.clone();
    let secret_owner = session.secret_owner();
    let jump_chain = match app_state.session.catalog.proxy_jump_chain(&session) {
        Ok(chain) => chain,
        Err(err) => {
            log::warn!("解析 RDP SSH 隧道跳板机链失败: {err}");
            Vec::new()
        }
    };
    let outbound_proxy = session.effective_proxy(&app_settings(cx).proxy);
    let allow_sha1 = app_settings(cx).allow_sha1_publickey_auth;

    let tunnel_params = RdpTunnelLaunchParams {
        jump_chain,
        outbound_proxy,
        allow_sha1,
    };

    launch_rdp_session_async(
        view.clone(),
        secret_store.clone(),
        known_hosts.clone(),
        secret_owner.clone(),
        rdp.clone(),
        tunnel_params.clone(),
        cx,
    );

    let subscription = cx.subscribe(&view, move |shell, view, event: &RdpViewEvent, cx| {
        match event {
            RdpViewEvent::Reconnect => {
                launch_rdp_session_async(
                    view,
                    secret_store.clone(),
                    known_hosts.clone(),
                    secret_owner.clone(),
                    rdp.clone(),
                    tunnel_params.clone(),
                    cx,
                );
            }
            RdpViewEvent::TrustAndReconnect {
                host,
                port,
                fingerprint,
            } => {
                let info = HostKeyInfo::new("rdp-tls-sha256", fingerprint.clone(), None);
                if let Err(err) = known_hosts.record(host, *port, &info) {
                    log::warn!("更新 RDP known_hosts 指纹失败: {err}");
                }
                launch_rdp_session_async(
                    view,
                    secret_store.clone(),
                    known_hosts.clone(),
                    secret_owner.clone(),
                    rdp.clone(),
                    tunnel_params.clone(),
                    cx,
                );
            }
            RdpViewEvent::FullscreenChanged(active) => {
                shell.state.ui.immersive = *active;
                if *active {
                    shell.state.ui.modals.close_transient();
                }
                cx.notify();
                return;
            }
            _ => {}
        }

        let Some(tab) = shell
            .state
            .workspace
            .tabs
            .iter_mut()
            .find(|t| t.id == tab_id)
        else {
            return;
        };
        tab.status = match event {
            RdpViewEvent::Connected => TabStatus::Connected,
            RdpViewEvent::Disconnected => TabStatus::Disconnected,
            RdpViewEvent::Reconnect | RdpViewEvent::TrustAndReconnect { .. } => {
                TabStatus::Connecting
            }
            RdpViewEvent::Failed(detail) => TabStatus::Failed(AppError::with_detail(
                ErrorCode::ConnectionFailed,
                detail.clone(),
            )),
            RdpViewEvent::FullscreenChanged(_) => return,
        };
        cx.notify();
    });

    let tab = WorkspaceTab::new_rdp(tab_id, session, view, subscription);
    app_state.workspace.create_group_for_tab(&tab);
    app_state.workspace.tabs.push(tab);
    app_state.workspace.active_tab = Some(tab_id);
    app_state.present_active_group();
    cx.notify();
}

#[derive(Clone)]
struct RdpTunnelLaunchParams {
    jump_chain: Vec<Session>,
    outbound_proxy: OutboundProxy,
    allow_sha1: bool,
}

/// 在专用阻塞线程池读取钥匙串凭据，按需建立 SSH 转发隧道后回到 UI 线程启动 RDP 会话。
fn launch_rdp_session_async(
    view: Entity<RdpView>,
    secret_store: Arc<dyn SecretStore>,
    known_hosts: Arc<dyn KnownHostsStore>,
    secret_owner: SecretOwner,
    rdp: RdpConnection,
    tunnel_params: RdpTunnelLaunchParams,
    cx: &mut Context<AppShell>,
) {
    cx.spawn(async move |_, cx| {
        let jump_chain = tunnel_params.jump_chain;
        let prep_result = spawn_blocking_local(move || {
            let key = SecretKey::new(secret_owner, SecretKind::Password);
            let password = secret_store
                .get(&key)
                .map(|secret| secret.map(|s| s.as_str().to_string()).unwrap_or_default())
                .map_err(|err| rust_i18n::t!("rdp.password_read_failed", err = err).to_string())?;
            let (hops, hop_err) = collect_jump_hops(secret_store.as_ref(), &jump_chain);
            if let Some(err) = hop_err {
                return Err(rust_i18n::t!("rdp.jump_credentials_failed", err = err).to_string());
            }
            Ok((password, hops))
        })
        .await
        .unwrap_or_else(|| Err(rust_i18n::t!("rdp.password_task_cancelled").to_string()));

        let (password, hops) = match prep_result {
            Ok(v) => v,
            Err(detail) => {
                log::error!("{detail}");
                view.update(cx, |rdp_view, cx| {
                    rdp_view.fail_with_error(detail, cx);
                });
                return;
            }
        };

        let (connect_endpoint, tunnel_guard) = if !hops.is_empty() {
            match tunnel::open_ssh_tunnel_for_rdp(
                hops,
                tunnel_params.outbound_proxy,
                known_hosts.clone(),
                tunnel_params.allow_sha1,
                rdp.host.clone(),
                rdp.port,
            )
            .await
            {
                Ok((local_port, guard)) => {
                    (Some(("127.0.0.1".to_string(), local_port)), Some(guard))
                }
                Err(detail) => {
                    log::error!("{detail}");
                    view.update(cx, |rdp_view, cx| {
                        rdp_view.fail_with_error(detail, cx);
                    });
                    return;
                }
            }
        } else {
            (None, None)
        };

        let port = rdp.port;
        view.update(cx, |rdp_view, cx| {
            let initial_size = rdp_view.current_viewport_size().unwrap_or((1280, 800));
            let mut config = SessionConfig::new(&rdp, password, initial_size);
            config.scale_factor_percent = rdp_view.current_scale_factor_percent();
            config.connect_endpoint = connect_endpoint;
            config.cert_verifier = Some(build_rdp_cert_verifier(known_hosts, port));
            rdp_view.start_session(config, tunnel_guard, cx);
        });
    })
    .detach();
}

/// 基于 `KnownHostsStore` 构造 RDP TLS 公钥 TOFU 指纹校验回调。
fn build_rdp_cert_verifier(known_hosts: Arc<dyn KnownHostsStore>, port: u16) -> TlsCertVerifier {
    Arc::new(move |cert: &TlsCertificateInfo| {
        let info = HostKeyInfo::new("rdp-tls-sha256", cert.sha256_fingerprint.clone(), None);
        match known_hosts.check(&cert.host, port, &info) {
            Ok(HostKeyStatus::Match) => TlsVerifyOutcome::Trusted,
            Ok(HostKeyStatus::Unknown) => {
                if let Err(err) = known_hosts.record(&cert.host, port, &info) {
                    log::warn!("记录 RDP TLS 首次信任指纹失败，忽略继续: {err}");
                } else {
                    log::info!(
                        "首次信任 RDP 主机 TLS 公钥指纹并已持久化，主机: {}:{port}，指纹: {}",
                        cert.host,
                        cert.sha256_fingerprint
                    );
                }
                TlsVerifyOutcome::Trusted
            }
            Ok(HostKeyStatus::Mismatch {
                previous_fingerprint,
            }) => TlsVerifyOutcome::Mismatch {
                previous_fingerprint,
            },
            Err(err) => {
                log::warn!("检查 RDP known_hosts 失败，降级放行: {err}");
                TlsVerifyOutcome::Trusted
            }
        }
    })
}
