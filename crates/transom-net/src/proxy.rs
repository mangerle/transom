//! 出站代理连接器：SOCKS5（RFC 1928/1929）与 HTTP CONNECT 隧道。
//!
//! 设计原理（Why）：
//! - SSH 客户端需要的是"一条到目标 host:port 的透明字节流"，代理握手只是
//!   建立这条流的前置步骤；把两种代理协议收敛到同一个
//!   [`connect_via_proxy`] 入口，上层握手代码不必知道字节从哪来；
//! - SOCKS5 目标地址一律以域名形式下发（ATYP 0x03）交由代理远端解析，
//!   本机不做 DNS 查询，避免受限内网的 DNS 劫持与解析泄露；字面 IP 直接
//!   按 IP 编码，不产生解析行为；
//! - HTTP CONNECT 的响应头逐字节读取，杜绝一次读到隧道起始数据后无法回退
//!   的问题；头长度设上限，防止恶意代理用超长响应拖住连接。
//!
//! 代价与局限：不实现 SOCKS4、HTTPS 代理与代理链；超时由调用方统一包裹，
//! 本模块只负责协议本身。

use crate::error::NetError;
use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use std::net::IpAddr;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use transom_core::{OutboundProxy, ProxyAuth};

/// HTTP CONNECT 响应头上限；正常代理响应只有几百字节。
const HTTP_HEADER_LIMIT: usize = 16 * 1024;
/// SOCKS5 用户名/密码字段上限（RFC 1929）。
const SOCKS_CREDENTIAL_LIMIT: usize = 255;

/// 经代理策略建立到目标地址的 TCP 隧道。
///
/// # Errors
/// 代理不可达、握手被拒、鉴权失败、响应非法或超时时返回
/// [`NetError::Proxy`]，错误文本包含代理地址与目标，但绝不含口令。
pub(crate) async fn connect_via_proxy(
    target_host: &str,
    target_port: u16,
    proxy: &OutboundProxy,
) -> Result<TcpStream, NetError> {
    match proxy {
        OutboundProxy::Direct => connect_tcp(target_host, target_port).await,
        OutboundProxy::Socks5 { host, port, auth } => {
            let mut stream = connect_tcp(host, *port).await?;
            socks5_handshake(&mut stream, auth.as_ref(), target_host, target_port)
                .await
                .map_err(|reason| proxy_error(proxy, target_host, target_port, reason))?;
            Ok(stream)
        }
        OutboundProxy::Http { host, port, auth } => {
            let mut stream = connect_tcp(host, *port).await?;
            http_connect(&mut stream, auth.as_ref(), target_host, target_port)
                .await
                .map_err(|reason| proxy_error(proxy, target_host, target_port, reason))?;
            Ok(stream)
        }
    }
}

fn proxy_error(proxy: &OutboundProxy, host: &str, port: u16, reason: String) -> NetError {
    NetError::Proxy {
        proxy: proxy.describe(),
        target: format!("{host}:{port}"),
        reason,
    }
}

async fn connect_tcp(host: &str, port: u16) -> Result<TcpStream, NetError> {
    TcpStream::connect((host, port))
        .await
        .map_err(|source| NetError::Connect {
            host: host.to_string(),
            port,
            source,
        })
}

/// SOCKS5 握手：方法协商 -> 可选鉴权 -> CONNECT 请求 -> 读取应答。
async fn socks5_handshake(
    stream: &mut TcpStream,
    auth: Option<&ProxyAuth>,
    target_host: &str,
    target_port: u16,
) -> Result<(), String> {
    // 方法协商：无认证始终可选；有认证信息时同时提供用户名/密码方式。
    let methods: &[u8] = if auth.is_some() {
        &[0x00, 0x02]
    } else {
        &[0x00]
    };
    let mut greeting = Vec::with_capacity(2 + methods.len());
    greeting.push(0x05);
    greeting.push(methods.len() as u8);
    greeting.extend_from_slice(methods);
    write_all(stream, &greeting, "发送 SOCKS5 方法协商失败").await?;

    let mut reply = [0u8; 2];
    read_exact(stream, &mut reply, "读取 SOCKS5 方法协商应答失败").await?;
    if reply[0] != 0x05 {
        return Err(format!("代理返回了非法 SOCKS 版本: {}", reply[0]));
    }
    match reply[1] {
        0x00 => {}
        0x02 => {
            let auth = auth.ok_or_else(|| "代理要求用户名/密码认证，但未配置凭据".to_string())?;
            socks5_authenticate(stream, auth).await?;
        }
        0xFF => return Err("代理拒绝了全部认证方式".to_string()),
        other => return Err(format!("代理选择了不支持的认证方式: 0x{other:02x}")),
    }

    // CONNECT 请求：远端解析域名，本机不做 DNS 查询。
    let mut request = Vec::with_capacity(32);
    request.extend_from_slice(&[0x05, 0x01, 0x00]);
    encode_socks_address(&mut request, target_host)?;
    request.extend_from_slice(&target_port.to_be_bytes());
    write_all(stream, &request, "发送 SOCKS5 CONNECT 请求失败").await?;

    let mut head = [0u8; 4];
    read_exact(stream, &mut head, "读取 SOCKS5 CONNECT 应答失败").await?;
    if head[0] != 0x05 {
        return Err(format!("代理返回了非法 SOCKS 版本: {}", head[0]));
    }
    if head[1] != 0x00 {
        return Err(format!("代理拒绝建立隧道: {}", socks5_reply_text(head[1])));
    }
    match head[3] {
        0x01 => skip_bytes(stream, 4 + 2, "读取 SOCKS5 绑定 IPv4 地址失败").await?,
        0x04 => skip_bytes(stream, 16 + 2, "读取 SOCKS5 绑定 IPv6 地址失败").await?,
        0x03 => {
            let mut length = [0u8; 1];
            read_exact(stream, &mut length, "读取 SOCKS5 绑定域名长度失败").await?;
            skip_bytes(stream, length[0] as usize + 2, "读取 SOCKS5 绑定域名失败").await?;
        }
        other => return Err(format!("代理返回了不支持的地址类型: 0x{other:02x}")),
    }
    Ok(())
}

async fn socks5_authenticate(stream: &mut TcpStream, auth: &ProxyAuth) -> Result<(), String> {
    let username = auth.username.as_bytes();
    let password = auth.password.as_bytes();
    if username.len() > SOCKS_CREDENTIAL_LIMIT || password.len() > SOCKS_CREDENTIAL_LIMIT {
        return Err("SOCKS5 用户名或密码超过 255 字节上限".to_string());
    }
    let mut packet = Vec::with_capacity(3 + username.len() + password.len());
    packet.push(0x01);
    packet.push(username.len() as u8);
    packet.extend_from_slice(username);
    packet.push(password.len() as u8);
    packet.extend_from_slice(password);
    write_all(stream, &packet, "发送 SOCKS5 鉴权信息失败").await?;

    let mut reply = [0u8; 2];
    read_exact(stream, &mut reply, "读取 SOCKS5 鉴权结果失败").await?;
    if reply[0] != 0x01 {
        return Err(format!("代理返回了非法鉴权版本: {}", reply[0]));
    }
    if reply[1] != 0x00 {
        return Err("代理用户名或密码被拒绝".to_string());
    }
    Ok(())
}

/// 按 SOCKS5 地址格式写入目标：字面 IP 用 ATYP 1/4，域名用 ATYP 3。
fn encode_socks_address(buffer: &mut Vec<u8>, host: &str) -> Result<(), String> {
    match host.parse::<IpAddr>() {
        Ok(IpAddr::V4(address)) => {
            buffer.push(0x01);
            buffer.extend_from_slice(&address.octets());
        }
        Ok(IpAddr::V6(address)) => {
            buffer.push(0x04);
            buffer.extend_from_slice(&address.octets());
        }
        Err(_) => {
            let bytes = host.as_bytes();
            if bytes.is_empty() || bytes.len() > SOCKS_CREDENTIAL_LIMIT {
                return Err(format!("目标域名长度非法: {} 字节", bytes.len()));
            }
            buffer.push(0x03);
            buffer.push(bytes.len() as u8);
            buffer.extend_from_slice(bytes);
        }
    }
    Ok(())
}

fn socks5_reply_text(code: u8) -> String {
    match code {
        0x01 => "代理内部错误".to_string(),
        0x02 => "代理规则禁止该连接".to_string(),
        0x03 => "目标网络不可达".to_string(),
        0x04 => "目标主机不可达".to_string(),
        0x05 => "目标拒绝连接".to_string(),
        0x06 => "连接已过期".to_string(),
        0x07 => "代理不支持该命令".to_string(),
        0x08 => "代理不支持该地址类型".to_string(),
        other => format!("未知错误码 0x{other:02x}"),
    }
}

/// HTTP CONNECT：发送请求头 -> 逐字节读取响应头 -> 校验 200。
async fn http_connect(
    stream: &mut TcpStream,
    auth: Option<&ProxyAuth>,
    target_host: &str,
    target_port: u16,
) -> Result<(), String> {
    let authority = authority(target_host, target_port);
    let mut request = String::with_capacity(128);
    request.push_str(&format!("CONNECT {authority} HTTP/1.1\r\n"));
    request.push_str(&format!("Host: {authority}\r\n"));
    request.push_str("Proxy-Connection: Keep-Alive\r\n");
    if let Some(auth) = auth {
        let raw = format!("{}:{}", auth.username, auth.password);
        let encoded = STANDARD.encode(raw.as_bytes());
        request.push_str(&format!("Proxy-Authorization: Basic {encoded}\r\n"));
    }
    request.push_str("\r\n");
    write_all(stream, request.as_bytes(), "发送 HTTP CONNECT 请求失败").await?;

    let mut header = Vec::with_capacity(256);
    let mut previous = 0u8;
    loop {
        let mut byte = [0u8; 1];
        read_exact(stream, &mut byte, "读取 HTTP CONNECT 响应失败").await?;
        header.push(byte[0]);
        if previous == b'\r' && byte[0] == b'\n' {
            let len = header.len();
            if len >= 4 && header[len - 4..] == *b"\r\n\r\n" {
                break;
            }
        }
        previous = byte[0];
        if header.len() > HTTP_HEADER_LIMIT {
            return Err(format!("代理响应头超过 {HTTP_HEADER_LIMIT} 字节上限"));
        }
    }

    let status_line = header
        .split(|byte| *byte == b'\n')
        .next()
        .map(|line| String::from_utf8_lossy(line).trim().to_string())
        .unwrap_or_default();
    let status = status_line.split_whitespace().nth(1).unwrap_or_default();
    if status != "200" {
        return Err(format!("代理拒绝建立隧道，响应: {status_line}"));
    }
    Ok(())
}

/// CONNECT 的 authority 形式；IPv6 字面量必须加方括号。
fn authority(host: &str, port: u16) -> String {
    if host.contains(':') && !host.starts_with('[') {
        format!("[{host}]:{port}")
    } else {
        format!("{host}:{port}")
    }
}

async fn write_all(stream: &mut TcpStream, bytes: &[u8], reason: &str) -> Result<(), String> {
    stream
        .write_all(bytes)
        .await
        .map_err(|error| format!("{reason}: {error}"))
}

async fn read_exact(stream: &mut TcpStream, bytes: &mut [u8], reason: &str) -> Result<(), String> {
    stream
        .read_exact(bytes)
        .await
        .map_err(|error| format!("{reason}: {error}"))?;
    Ok(())
}

async fn skip_bytes(stream: &mut TcpStream, count: usize, reason: &str) -> Result<(), String> {
    let mut buffer = vec![0u8; count];
    read_exact(stream, &mut buffer, reason).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn socks_address_encodes_ipv4_and_domain() {
        let mut buffer = Vec::new();
        encode_socks_address(&mut buffer, "127.0.0.1").unwrap();
        assert_eq!(buffer, vec![0x01, 127, 0, 0, 1]);

        let mut buffer = Vec::new();
        encode_socks_address(&mut buffer, "example.com").unwrap();
        assert_eq!(buffer[0], 0x03);
        assert_eq!(buffer[1] as usize, "example.com".len());
        assert_eq!(&buffer[2..], b"example.com");
    }

    #[test]
    fn socks_address_encodes_ipv6() {
        let mut buffer = Vec::new();
        encode_socks_address(&mut buffer, "::1").unwrap();
        assert_eq!(buffer[0], 0x04);
        assert_eq!(buffer.len(), 17);
    }

    #[test]
    fn http_authority_brackets_ipv6() {
        assert_eq!(authority("web-01", 22), "web-01:22");
        assert_eq!(authority("::1", 22), "[::1]:22");
        assert_eq!(authority("[::1]", 22), "[::1]:22");
    }

    #[test]
    fn reply_codes_have_chinese_descriptions() {
        assert_eq!(socks5_reply_text(0x05), "目标拒绝连接");
        assert!(socks5_reply_text(0x42).contains("0x42"));
    }
}
