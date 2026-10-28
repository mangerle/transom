use crate::client::SshRemote;
use crate::timeout::SOCKS5_HANDSHAKE_TIMEOUT;
use russh::Channel;
use russh::client::Msg;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream, UdpSocket};
use tokio::sync::watch;
use tokio::task::{JoinHandle, JoinSet};
use transom_core::{
    AppError, BindAddress, ErrorCode, ForwardKind, ForwardRule, SocksAuth, TargetAddress,
};

struct RemoteForwardInfo {
    remote: SshRemote,
    host: String,
    port: u16,
}

pub struct ForwardTask {
    pub bound_port: Option<u16>,
    stop: watch::Sender<bool>,
    join: Option<JoinHandle<()>>,
    remote_forward: Option<RemoteForwardInfo>,
}

impl ForwardTask {
    pub async fn stop(mut self) {
        let _ = self.stop.send(true);
        if let Some(join) = self.join.take() {
            let _ = join.await;
        }
        if let Some(info) = self.remote_forward.take() {
            info.remote.unregister_remote_target(info.port);
            let _ = info
                .remote
                .cancel_tcpip_forward(&info.host, info.port)
                .await;
        }
    }
}

impl Drop for ForwardTask {
    fn drop(&mut self) {
        let _ = self.stop.send(true);
        if let Some(info) = self.remote_forward.take() {
            info.remote.unregister_remote_target(info.port);
        }
    }
}

pub async fn start_forward(
    remote: &SshRemote,
    rule: &ForwardRule,
) -> Result<ForwardTask, AppError> {
    match &rule.kind {
        ForwardKind::Local { bind, target } => start_local(remote, bind, target).await,
        ForwardKind::Dynamic { bind, socks } => {
            if !matches!(socks.auth, SocksAuth::None) {
                return Err(AppError::with_detail(
                    ErrorCode::ForwardFailed,
                    "动态转发暂不支持 SOCKS 认证",
                ));
            }
            start_dynamic(remote, bind).await
        }
        ForwardKind::Remote { bind, target } => start_remote(remote, bind, target).await,
        _ => Err(AppError::with_detail(
            ErrorCode::ForwardFailed,
            "不支持的转发模式",
        )),
    }
}

async fn start_local(
    remote: &SshRemote,
    bind: &BindAddress,
    target: &TargetAddress,
) -> Result<ForwardTask, AppError> {
    let listener = bind_listener(bind).await?;
    let bound_port = listener.local_addr().ok().map(|address| address.port());
    let (stop, mut stop_rx) = watch::channel(false);
    let remote = remote.clone();
    let target_host = target.host.clone();
    let target_port = target.port;

    let join = tokio::spawn(async move {
        let mut connections = JoinSet::new();
        loop {
            tokio::select! {
                _ = stop_rx.changed() => break,
                // 及时回收已结束的连接任务，避免 JoinSet 随历史连接数增长。
                _ = connections.join_next(), if !connections.is_empty() => {}
                accepted = listener.accept() => {
                    let Ok((tcp, _)) = accepted else {
                        continue;
                    };
                    connections.spawn(relay_local_connection(
                        remote.clone(),
                        tcp,
                        target_host.clone(),
                        target_port,
                    ));
                }
            }
        }
        // 停止转发必须同时断开在途连接：abort 取消 copy_bidirectional 后，
        // 本地套接字与 SSH 通道随 future 一起释放，客户端立即看到连接关闭。
        abort_connections(&mut connections).await;
    });

    Ok(ForwardTask {
        bound_port,
        stop,
        join: Some(join),
        remote_forward: None,
    })
}

async fn start_dynamic(remote: &SshRemote, bind: &BindAddress) -> Result<ForwardTask, AppError> {
    let listener = bind_listener(bind).await?;
    let bound_port = listener.local_addr().ok().map(|address| address.port());
    let (stop, mut stop_rx) = watch::channel(false);
    let remote = remote.clone();

    let join = tokio::spawn(async move {
        let mut connections = JoinSet::new();
        loop {
            tokio::select! {
                _ = stop_rx.changed() => break,
                _ = connections.join_next(), if !connections.is_empty() => {}
                accepted = listener.accept() => {
                    let Ok((tcp, _)) = accepted else {
                        continue;
                    };
                    let remote = remote.clone();
                    connections.spawn(async move {
                        if let Err(error) = run_socks5(remote, tcp).await {
                            log::debug!("SOCKS5 转发连接处理失败，原因: {error}");
                        }
                    });
                }
            }
        }
        // 与本地转发一致：停止时中止协商与中继任务，关闭全部在途连接。
        abort_connections(&mut connections).await;
    });

    Ok(ForwardTask {
        bound_port,
        stop,
        join: Some(join),
        remote_forward: None,
    })
}

async fn start_remote(
    remote: &SshRemote,
    bind: &BindAddress,
    target: &TargetAddress,
) -> Result<ForwardTask, AppError> {
    let actual_port = remote.tcpip_forward(&bind.host, bind.port).await?;
    let actual_port = actual_port.min(u16::MAX as u32) as u16;
    remote.register_remote_target(actual_port, target.host.clone(), target.port);
    let (stop, _stop_rx) = watch::channel(false);

    Ok(ForwardTask {
        bound_port: Some(actual_port),
        stop,
        join: None,
        remote_forward: Some(RemoteForwardInfo {
            remote: remote.clone(),
            host: bind.host.clone(),
            port: actual_port,
        }),
    })
}

async fn bind_listener(bind: &BindAddress) -> Result<TcpListener, AppError> {
    TcpListener::bind((bind.host.as_str(), bind.port))
        .await
        .map_err(forward_error)
}

fn forward_error(error: std::io::Error) -> AppError {
    AppError::with_detail(ErrorCode::ForwardFailed, error.to_string())
}

/// 本地转发单条连接：建立远端通道后双向复制，任一方向结束即关闭两端。
async fn relay_local_connection(remote: SshRemote, mut tcp: TcpStream, host: String, port: u16) {
    let channel = match remote.open_direct_tcpip(&host, port).await {
        Ok(channel) => channel,
        Err(error) => {
            log::debug!("本地转发建立远端通道失败，目标: {host}:{port}，原因: {error}");
            return;
        }
    };
    let mut stream = channel.into_stream();
    let _ = tokio::io::copy_bidirectional(&mut tcp, &mut stream).await;
}

/// SOCKS5 协商完成后的转发形态。
enum Socks5Route {
    /// CONNECT：已建立到目标地址的 SSH 通道。
    Connect(Channel<Msg>),
    /// UDP ASSOCIATE：保留本地 UDP 套接字，交给中继循环。
    Udp {
        inbound: UdpSocket,
        outbound: UdpSocket,
    },
}

/// SOCKS5 入口：先限时完成协商，再进入不设总时限的数据中继。
async fn run_socks5(remote: SshRemote, mut client: TcpStream) -> Result<(), AppError> {
    let route = tokio::time::timeout(
        SOCKS5_HANDSHAKE_TIMEOUT,
        socks5_negotiate(&remote, &mut client),
    )
    .await
    .map_err(|_| {
        AppError::with_detail(
            ErrorCode::Timeout,
            format!(
                "SOCKS5 握手超时，等待上限: {} 秒",
                SOCKS5_HANDSHAKE_TIMEOUT.as_secs()
            ),
        )
    })??;
    match route {
        Socks5Route::Connect(channel) => {
            let mut stream = channel.into_stream();
            let _ = tokio::io::copy_bidirectional(&mut client, &mut stream).await;
            Ok(())
        }
        Socks5Route::Udp { inbound, outbound } => {
            run_socks5_udp_relay(client, inbound, outbound).await
        }
    }
}

/// 读取客户端问候与请求：CONNECT 时顺带建立远端通道，UDP 时创建本地套接字。
async fn socks5_negotiate(
    remote: &SshRemote,
    client: &mut TcpStream,
) -> Result<Socks5Route, AppError> {
    negotiate_methods(client).await?;
    let request = read_request_header(client).await?;
    match request[1] {
        1 => connect_route(remote, client, request[3]).await,
        3 => udp_associate(client, request[3]).await,
        other => {
            let _ = client.write_all(&[5, 7, 0, 1, 0, 0, 0, 0, 0, 0]).await;
            Err(AppError::with_detail(
                ErrorCode::ForwardFailed,
                format!("仅支持 SOCKS5 CONNECT 与 UDP ASSOCIATE，实际命令: {other}"),
            ))
        }
    }
}

/// 读取版本与认证方式，完成免认证协商。
async fn negotiate_methods(client: &mut TcpStream) -> Result<(), AppError> {
    let mut greeting = [0u8; 2];
    client
        .read_exact(&mut greeting)
        .await
        .map_err(forward_error)?;
    if greeting[0] != 5 {
        return Err(AppError::with_detail(
            ErrorCode::ForwardFailed,
            format!("不支持的 SOCKS 版本: {}", greeting[0]),
        ));
    }
    let mut methods = vec![0u8; greeting[1] as usize];
    client
        .read_exact(&mut methods)
        .await
        .map_err(forward_error)?;
    if !methods.contains(&0) {
        client.write_all(&[5, 0xff]).await.map_err(forward_error)?;
        return Err(AppError::with_detail(
            ErrorCode::ForwardFailed,
            "SOCKS 客户端未提供免认证方式",
        ));
    }
    client.write_all(&[5, 0]).await.map_err(forward_error)?;
    Ok(())
}

/// 读取 4 字节请求头并校验版本。
async fn read_request_header(client: &mut TcpStream) -> Result<[u8; 4], AppError> {
    let mut request = [0u8; 4];
    client
        .read_exact(&mut request)
        .await
        .map_err(forward_error)?;
    if request[0] != 5 {
        let _ = client.write_all(&[5, 7, 0, 1, 0, 0, 0, 0, 0, 0]).await;
        return Err(AppError::with_detail(
            ErrorCode::ForwardFailed,
            format!("不支持的 SOCKS 版本: {}", request[0]),
        ));
    }
    Ok(request)
}

/// CONNECT：解析目标地址、建立远端通道并回应成功。
async fn connect_route(
    remote: &SshRemote,
    client: &mut TcpStream,
    atyp: u8,
) -> Result<Socks5Route, AppError> {
    let (host, port) = read_socks5_address_and_port(client, atyp).await?;
    let channel = match remote.open_direct_tcpip(&host, port).await {
        Ok(channel) => channel,
        Err(error) => {
            let _ = client.write_all(&[5, 1, 0, 1, 0, 0, 0, 0, 0, 0]).await;
            return Err(error);
        }
    };
    client
        .write_all(&[5, 0, 0, 1, 0, 0, 0, 0, 0, 0])
        .await
        .map_err(forward_error)?;
    Ok(Socks5Route::Connect(channel))
}

/// UDP ASSOCIATE：创建本地收发套接字并回应绑定地址。
async fn udp_associate(client: &mut TcpStream, atyp: u8) -> Result<Socks5Route, AppError> {
    let (_client_bind_host, _client_bind_port) = read_socks5_address_and_port(client, atyp).await?;
    let local_ip = client
        .local_addr()
        .map(|addr| addr.ip())
        .unwrap_or(IpAddr::V4(Ipv4Addr::LOCALHOST));
    let bind_ip = match local_ip {
        IpAddr::V4(v4) if v4.is_unspecified() => IpAddr::V4(Ipv4Addr::LOCALHOST),
        IpAddr::V6(v6) if v6.is_unspecified() => IpAddr::V6(Ipv6Addr::LOCALHOST),
        other => other,
    };

    // 监听客户端发送过来的 SOCKS5 UDP 数据报。
    let inbound_socket = UdpSocket::bind(SocketAddr::new(bind_ip, 0))
        .await
        .map_err(forward_error)?;
    let bound_addr = inbound_socket.local_addr().map_err(forward_error)?;

    // 负责向外部实际目标转发并接收回包的 socket。
    let outbound_bind_addr = match bound_addr {
        SocketAddr::V4(_) => SocketAddr::new(IpAddr::V4(Ipv4Addr::UNSPECIFIED), 0),
        SocketAddr::V6(_) => SocketAddr::new(IpAddr::V6(Ipv6Addr::UNSPECIFIED), 0),
    };
    let outbound_socket = UdpSocket::bind(outbound_bind_addr)
        .await
        .map_err(forward_error)?;

    // 返回成功应答，告知客户端应向哪个 UDP 端口发送数据。
    let mut response = vec![5, 0, 0];
    match bound_addr.ip() {
        IpAddr::V4(v4) => {
            response.push(1);
            response.extend_from_slice(&v4.octets());
        }
        IpAddr::V6(v6) => {
            response.push(4);
            response.extend_from_slice(&v6.octets());
        }
    }
    response.extend_from_slice(&bound_addr.port().to_be_bytes());
    client.write_all(&response).await.map_err(forward_error)?;

    Ok(Socks5Route::Udp {
        inbound: inbound_socket,
        outbound: outbound_socket,
    })
}

async fn read_socks5_address_and_port(
    client: &mut TcpStream,
    atyp: u8,
) -> Result<(String, u16), AppError> {
    let host = match atyp {
        1 => {
            let mut address = [0u8; 4];
            client
                .read_exact(&mut address)
                .await
                .map_err(forward_error)?;
            Ipv4Addr::from(address).to_string()
        }
        3 => {
            let mut length = [0u8; 1];
            client
                .read_exact(&mut length)
                .await
                .map_err(forward_error)?;
            let mut domain = vec![0u8; length[0] as usize];
            client
                .read_exact(&mut domain)
                .await
                .map_err(forward_error)?;
            String::from_utf8(domain)
                .map_err(|_| AppError::with_detail(ErrorCode::ForwardFailed, "SOCKS 域名非法"))?
        }
        4 => {
            let mut address = [0u8; 16];
            client
                .read_exact(&mut address)
                .await
                .map_err(forward_error)?;
            Ipv6Addr::from(address).to_string()
        }
        _ => {
            let _ = client.write_all(&[5, 8, 0, 1, 0, 0, 0, 0, 0, 0]).await;
            return Err(AppError::with_detail(
                ErrorCode::ForwardFailed,
                format!("不支持的 SOCKS 地址类型: {}", atyp),
            ));
        }
    };
    let mut port = [0u8; 2];
    client.read_exact(&mut port).await.map_err(forward_error)?;
    let port = u16::from_be_bytes(port);
    Ok((host, port))
}

async fn run_socks5_udp_relay(
    mut client: TcpStream,
    inbound: UdpSocket,
    outbound: UdpSocket,
) -> Result<(), AppError> {
    let mut client_udp_peer: Option<SocketAddr> = None;
    let mut in_buf = vec![0u8; 65535];
    let mut out_buf = vec![0u8; 65535];
    let mut tcp_peek = [0u8; 1];

    loop {
        tokio::select! {
            // TCP 控制连接断开时，立即终止 UDP 关联
            res = client.read(&mut tcp_peek) => {
                match res {
                    Ok(0) | Err(_) => break,
                    Ok(_) => {}
                }
            }
            // 接收客户端发送的 SOCKS5 UDP 封装数据包
            res = inbound.recv_from(&mut in_buf) => {
                let (len, peer) = match res {
                    Ok(val) => val,
                    Err(error) => {
                        log::debug!("接收客户端 UDP 数据失败，原因: {error}");
                        continue;
                    }
                };
                client_udp_peer = Some(peer);
                let Some((host, port, offset)) = parse_udp_target(&in_buf, len) else {
                    continue;
                };
                let payload = &in_buf[offset..len];
                let destination = format!("{host}:{port}");
                let _ = outbound.send_to(payload, destination).await;
            }
            // 接收目标服务返回的原生 UDP 数据，封装为 SOCKS5 UDP 头部返回给客户端
            res = outbound.recv_from(&mut out_buf) => {
                let (len, remote_peer) = match res {
                    Ok(val) => val,
                    Err(error) => {
                        log::debug!("接收目标服务器 UDP 响应失败，原因: {error}");
                        continue;
                    }
                };
                let Some(client_peer) = client_udp_peer else {
                    continue;
                };
                let packet = build_udp_response(remote_peer, &out_buf[..len]);
                let _ = inbound.send_to(&packet, client_peer).await;
            }
        }
    }

    Ok(())
}

/// 中止并等待全部在途连接退出。
///
/// abort 会取消 `copy_bidirectional`/协商 future，套接字与 SSH 通道随
/// future 一起释放；`join_next` 再逐个回收，保证返回时任务集合已清空。
async fn abort_connections(connections: &mut JoinSet<()>) {
    connections.abort_all();
    while connections.join_next().await.is_some() {}
}

/// 解析 SOCKS5 UDP 请求头，返回目标主机、端口与载荷起始偏移。
///
/// 只接受 RSV=0、FRAG=0 的单包请求；地址类型 1/3/4 分别对应 IPv4、
/// 域名与 IPv6，长度不足或未知类型返回 `None`，由调用方丢弃该包。
fn parse_udp_target(packet: &[u8], len: usize) -> Option<(String, u16, usize)> {
    if len < 4 || packet[0] != 0 || packet[1] != 0 || packet[2] != 0 {
        return None;
    }
    let mut offset = 4;
    let host = match packet[3] {
        1 => {
            if len < offset + 4 + 2 {
                return None;
            }
            let ip = Ipv4Addr::new(
                packet[offset],
                packet[offset + 1],
                packet[offset + 2],
                packet[offset + 3],
            );
            offset += 4;
            ip.to_string()
        }
        3 => {
            if len <= offset {
                return None;
            }
            let domain_len = packet[offset] as usize;
            offset += 1;
            if len < offset + domain_len + 2 {
                return None;
            }
            let domain = String::from_utf8_lossy(&packet[offset..offset + domain_len]).to_string();
            offset += domain_len;
            domain
        }
        4 => {
            if len < offset + 16 + 2 {
                return None;
            }
            let mut octets = [0u8; 16];
            octets.copy_from_slice(&packet[offset..offset + 16]);
            offset += 16;
            Ipv6Addr::from(octets).to_string()
        }
        _ => return None,
    };
    let port = u16::from_be_bytes([packet[offset], packet[offset + 1]]);
    offset += 2;
    Some((host, port, offset))
}

/// 把目标返回的 UDP 载荷封装成 SOCKS5 UDP 响应头。
fn build_udp_response(remote: SocketAddr, payload: &[u8]) -> Vec<u8> {
    let mut packet = Vec::with_capacity(payload.len() + 22);
    packet.extend_from_slice(&[0, 0, 0]); // RSV + FRAG
    match remote {
        SocketAddr::V4(v4) => {
            packet.push(1);
            packet.extend_from_slice(&v4.ip().octets());
            packet.extend_from_slice(&v4.port().to_be_bytes());
        }
        SocketAddr::V6(v6) => {
            packet.push(4);
            packet.extend_from_slice(&v6.ip().octets());
            packet.extend_from_slice(&v6.port().to_be_bytes());
        }
    }
    packet.extend_from_slice(payload);
    packet
}

#[cfg(test)]
mod tests {
    use super::{abort_connections, build_udp_response, parse_udp_target};
    use std::net::{Ipv4Addr, Ipv6Addr, SocketAddr};
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use tokio::task::JoinSet;

    /// 停止转发时，在途连接必须被取消并回收，而不是继续跑完。
    #[tokio::test]
    async fn abort_connections_cancels_in_flight_tasks() {
        let cancelled = Arc::new(AtomicUsize::new(0));
        let mut connections = JoinSet::new();
        for _ in 0..3 {
            let cancelled = cancelled.clone();
            connections.spawn(async move {
                struct Guard(Arc<AtomicUsize>);
                impl Drop for Guard {
                    fn drop(&mut self) {
                        self.0.fetch_add(1, Ordering::Relaxed);
                    }
                }
                let _guard = Guard(cancelled);
                std::future::pending::<()>().await;
            });
        }
        tokio::task::yield_now().await;
        abort_connections(&mut connections).await;
        assert_eq!(
            cancelled.load(Ordering::Relaxed),
            3,
            "停止转发必须取消全部在途连接"
        );
        assert!(connections.is_empty(), "回收后任务集合必须为空");
    }

    /// IPv4、域名与 IPv6 三种地址类型都能解析出目标与载荷偏移。
    #[test]
    fn parse_udp_target_supports_all_address_types() {
        let mut ipv4 = vec![0, 0, 0, 1, 127, 0, 0, 1, 0x1f, 0x90];
        ipv4.extend_from_slice(b"payload");
        let (host, port, offset) = parse_udp_target(&ipv4, ipv4.len()).expect("IPv4 应可解析");
        assert_eq!(host, "127.0.0.1");
        assert_eq!(port, 8080);
        assert_eq!(&ipv4[offset..], b"payload");

        let mut domain = vec![0, 0, 0, 3, 11];
        domain.extend_from_slice(b"example.com");
        domain.extend_from_slice(&[0x01, 0xbb]);
        domain.extend_from_slice(b"data");
        let (host, port, offset) = parse_udp_target(&domain, domain.len()).expect("域名应可解析");
        assert_eq!(host, "example.com");
        assert_eq!(port, 443);
        assert_eq!(&domain[offset..], b"data");

        let mut ipv6 = vec![0, 0, 0, 4];
        ipv6.extend_from_slice(&Ipv6Addr::LOCALHOST.octets());
        ipv6.extend_from_slice(&[0x00, 0x16]);
        ipv6.extend_from_slice(b"v6");
        let (host, port, offset) = parse_udp_target(&ipv6, ipv6.len()).expect("IPv6 应可解析");
        assert_eq!(host, "::1");
        assert_eq!(port, 22);
        assert_eq!(&ipv6[offset..], b"v6");
    }

    /// 分片、保留位非零与长度不足的包一律丢弃。
    #[test]
    fn parse_udp_target_rejects_fragmented_and_truncated_packets() {
        assert!(parse_udp_target(&[0, 0, 1, 1, 127, 0, 0, 1], 8).is_none());
        assert!(parse_udp_target(&[1, 0, 0, 1], 4).is_none());
        assert!(parse_udp_target(&[0, 0, 0, 1, 127, 0], 6).is_none());
        assert!(parse_udp_target(&[0, 0, 0, 9], 4).is_none());
        assert!(parse_udp_target(&[0, 0, 0, 3, 10, b'a'], 6).is_none());
    }

    /// 响应头按目标地址族编码，载荷原样跟在后面。
    #[test]
    fn build_udp_response_encodes_address_family() {
        let v4 = build_udp_response(SocketAddr::from((Ipv4Addr::new(10, 0, 0, 1), 8080)), b"ok");
        assert_eq!(&v4[..4], &[0, 0, 0, 1]);
        assert_eq!(&v4[4..8], &[10, 0, 0, 1]);
        assert_eq!(&v4[8..10], &8080u16.to_be_bytes());
        assert_eq!(&v4[10..], b"ok");

        let v6 = build_udp_response(SocketAddr::from((Ipv6Addr::LOCALHOST, 443)), b"v6");
        assert_eq!(v6[3], 4);
        assert_eq!(&v6[4..20], &Ipv6Addr::LOCALHOST.octets());
        assert_eq!(&v6[20..22], &443u16.to_be_bytes());
        assert_eq!(&v6[22..], b"v6");
    }
}
