use crate::client::SshRemote;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream, UdpSocket};
use tokio::sync::watch;
use tokio::task::JoinHandle;
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
                    "SOCKS authentication is not supported yet",
                ));
            }
            start_dynamic(remote, bind).await
        }
        ForwardKind::Remote { bind, target } => start_remote(remote, bind, target).await,
        _ => Err(AppError::with_detail(
            ErrorCode::ForwardFailed,
            "unsupported forwarding mode",
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
        loop {
            tokio::select! {
                _ = stop_rx.changed() => break,
                accepted = listener.accept() => {
                    let Ok((mut tcp, _)) = accepted else {
                        continue;
                    };
                    let remote = remote.clone();
                    let target_host = target_host.clone();
                    tokio::spawn(async move {
                        if let Ok(channel) = remote.open_direct_tcpip(&target_host, target_port).await {
                            let mut stream = channel.into_stream();
                            let _ = tokio::io::copy_bidirectional(&mut tcp, &mut stream).await;
                        }
                    });
                }
            }
        }
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
        loop {
            tokio::select! {
                _ = stop_rx.changed() => break,
                accepted = listener.accept() => {
                    let Ok((tcp, _)) = accepted else {
                        continue;
                    };
                    let remote = remote.clone();
                    tokio::spawn(async move {
                        if let Err(error) = run_socks5(remote, tcp).await {
                            tracing::debug!(%error, "SOCKS5 forwarding connection failed");
                        }
                    });
                }
            }
        }
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

async fn run_socks5(remote: SshRemote, mut client: TcpStream) -> Result<(), AppError> {
    let mut greeting = [0u8; 2];
    client
        .read_exact(&mut greeting)
        .await
        .map_err(forward_error)?;
    if greeting[0] != 5 {
        return Err(AppError::with_detail(
            ErrorCode::ForwardFailed,
            "unsupported SOCKS version",
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
            "SOCKS client did not offer no-auth",
        ));
    }
    client.write_all(&[5, 0]).await.map_err(forward_error)?;

    let mut request = [0u8; 4];
    client
        .read_exact(&mut request)
        .await
        .map_err(forward_error)?;
    if request[0] != 5 {
        let _ = client.write_all(&[5, 7, 0, 1, 0, 0, 0, 0, 0, 0]).await;
        return Err(AppError::with_detail(
            ErrorCode::ForwardFailed,
            "unsupported SOCKS version",
        ));
    }

    match request[1] {
        1 => {
            // SOCKS5 CONNECT
            let (host, port) = read_socks5_address_and_port(&mut client, request[3]).await?;
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
            let mut stream = channel.into_stream();
            let _ = tokio::io::copy_bidirectional(&mut client, &mut stream).await;
            Ok(())
        }
        3 => {
            // SOCKS5 UDP ASSOCIATE
            let (_client_bind_host, _client_bind_port) =
                read_socks5_address_and_port(&mut client, request[3]).await?;

            // 获取与客户端通信的本地 IP 地址
            let local_ip = client
                .local_addr()
                .map(|addr| addr.ip())
                .unwrap_or(IpAddr::V4(Ipv4Addr::LOCALHOST));
            let bind_ip = match local_ip {
                IpAddr::V4(v4) => {
                    if v4.is_unspecified() {
                        IpAddr::V4(Ipv4Addr::LOCALHOST)
                    } else {
                        IpAddr::V4(v4)
                    }
                }
                IpAddr::V6(v6) => {
                    if v6.is_unspecified() {
                        IpAddr::V6(Ipv6Addr::LOCALHOST)
                    } else {
                        IpAddr::V6(v6)
                    }
                }
            };

            // 监听客户端发送过来的 SOCKS5 UDP 数据报
            let inbound_socket = UdpSocket::bind(SocketAddr::new(bind_ip, 0))
                .await
                .map_err(forward_error)?;
            let bound_addr = inbound_socket.local_addr().map_err(forward_error)?;

            // 负责向外部实际目标转发并接收回包的 socket
            let outbound_bind_addr = match bound_addr {
                SocketAddr::V4(_) => SocketAddr::new(IpAddr::V4(Ipv4Addr::UNSPECIFIED), 0),
                SocketAddr::V6(_) => SocketAddr::new(IpAddr::V6(Ipv6Addr::UNSPECIFIED), 0),
            };
            let outbound_socket = UdpSocket::bind(outbound_bind_addr)
                .await
                .map_err(forward_error)?;

            // 返回成功的 SOCKS5 应答，告知客户端应向哪个 UDP 端口发送数据
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

            run_socks5_udp_relay(client, inbound_socket, outbound_socket).await
        }
        _ => {
            let _ = client.write_all(&[5, 7, 0, 1, 0, 0, 0, 0, 0, 0]).await;
            Err(AppError::with_detail(
                ErrorCode::ForwardFailed,
                "only SOCKS CONNECT and UDP ASSOCIATE are supported",
            ))
        }
    }
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
            String::from_utf8(domain).map_err(|_| {
                AppError::with_detail(ErrorCode::ForwardFailed, "invalid SOCKS domain")
            })?
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
                "unsupported SOCKS address type",
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
                        tracing::debug!(%error, "接收客户端 UDP 数据失败");
                        continue;
                    }
                };
                client_udp_peer = Some(peer);
                if len < 4 {
                    continue;
                }
                // RSV (2 字节) 为 0，FRAG (1 字节) 为 0
                if in_buf[0] != 0 || in_buf[1] != 0 || in_buf[2] != 0 {
                    continue;
                }
                let atyp = in_buf[3];
                let mut offset = 4;
                let target_addr: Option<(String, u16)> = match atyp {
                    1 => {
                        if len >= offset + 4 + 2 {
                            let ip = Ipv4Addr::new(
                                in_buf[offset],
                                in_buf[offset + 1],
                                in_buf[offset + 2],
                                in_buf[offset + 3],
                            );
                            offset += 4;
                            let port = u16::from_be_bytes([in_buf[offset], in_buf[offset + 1]]);
                            offset += 2;
                            Some((ip.to_string(), port))
                        } else {
                            None
                        }
                    }
                    3 => {
                        if len > offset {
                            let domain_len = in_buf[offset] as usize;
                            offset += 1;
                            if len >= offset + domain_len + 2 {
                                let domain = String::from_utf8_lossy(
                                    &in_buf[offset..offset + domain_len],
                                )
                                .to_string();
                                offset += domain_len;
                                let port = u16::from_be_bytes([in_buf[offset], in_buf[offset + 1]]);
                                offset += 2;
                                Some((domain, port))
                            } else {
                                None
                            }
                        } else {
                            None
                        }
                    }
                    4 => {
                        if len >= offset + 16 + 2 {
                            let mut octets = [0u8; 16];
                            octets.copy_from_slice(&in_buf[offset..offset + 16]);
                            offset += 16;
                            let ip = Ipv6Addr::from(octets);
                            let port = u16::from_be_bytes([in_buf[offset], in_buf[offset + 1]]);
                            offset += 2;
                            Some((ip.to_string(), port))
                        } else {
                            None
                        }
                    }
                    _ => None,
                };

                let Some((host, port)) = target_addr else {
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
                        tracing::debug!(%error, "接收目标服务器 UDP 响应失败");
                        continue;
                    }
                };
                let Some(client_peer) = client_udp_peer else {
                    continue;
                };
                let mut packet = Vec::with_capacity(len + 22);
                packet.extend_from_slice(&[0, 0, 0]); // RSV + FRAG
                match remote_peer {
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
                packet.extend_from_slice(&out_buf[..len]);
                let _ = inbound.send_to(&packet, client_peer).await;
            }
        }
    }

    Ok(())
}
