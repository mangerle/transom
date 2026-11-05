//! Docker 远端 TLS 连接配置与客户端握手。
//!
//! 设计原理（Why）：生产环境中的 Docker 守护进程通常配置双向 TLS（mTLS），
//! 监听在 2376 端口。通过加载 CA 根证书、客户端证书与私钥，安全建立加密
//! 传输通道，杜绝明文传输导致的宿主机控制权外泄。

use crate::error::ContainerError;
use rustls::pki_types::pem::PemObject;
use rustls::pki_types::{CertificateDer, PrivateKeyDer, ServerName};
use rustls::{ClientConfig, RootCertStore};
use std::collections::HashMap;
use std::path::Path;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;
use tokio::net::TcpStream;
use tokio_rustls::TlsConnector;
use tokio_rustls::client::TlsStream;
use transom_core::TlsConfig;

/// TLS 握手超时：TCP 已连接但对端不完成握手时不能永久等待。
const TLS_HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(10);
/// 证书配置缓存上限；路径与更新时间不同的配置各自独立。
const TLS_CONFIG_CACHE_LIMIT: usize = 16;

/// 基于 TLS 配置建立加密的 TCP 流。
///
/// # Errors
/// 证书文件读取失败、格式非法或 TLS 握手失败时返回 [`ContainerError`]。
pub async fn connect_tls(
    tcp_stream: TcpStream,
    host: &str,
    tls_config: &TlsConfig,
) -> Result<TlsStream<TcpStream>, ContainerError> {
    let client_config = cached_client_config(tls_config)?;
    let connector = TlsConnector::from(client_config);
    let server_name =
        ServerName::try_from(host.to_string()).map_err(|error| ContainerError::Connect {
            endpoint: format!("tls://{host}"),
            source: std::io::Error::new(std::io::ErrorKind::InvalidInput, error.to_string()),
        })?;

    // TLS 握手同样属于外部 I/O：对端接受 TCP 后不再响应时必须快速失败。
    tokio::time::timeout(
        TLS_HANDSHAKE_TIMEOUT,
        connector.connect(server_name, tcp_stream),
    )
    .await
    .map_err(|_| ContainerError::Connect {
        endpoint: format!("tls://{host}"),
        source: std::io::Error::new(std::io::ErrorKind::TimedOut, "TLS 握手超时"),
    })?
    .map_err(|source| ContainerError::Connect {
        endpoint: format!("tls://{host}"),
        source,
    })
}

/// 证书配置缓存：同一组证书文件在进程内只解析一次。
fn cached_client_config(tls: &TlsConfig) -> Result<Arc<ClientConfig>, ContainerError> {
    static CACHE: OnceLock<Mutex<HashMap<String, Arc<ClientConfig>>>> = OnceLock::new();
    let key = tls_cache_key(tls);
    if let Ok(cache) = CACHE
        .get_or_init(|| Mutex::new(HashMap::with_capacity(4)))
        .lock()
        && let Some(config) = cache.get(&key)
    {
        return Ok(Arc::clone(config));
    }
    let config = Arc::new(build_client_config(tls)?);
    if let Ok(mut cache) = CACHE
        .get_or_init(|| Mutex::new(HashMap::with_capacity(4)))
        .lock()
    {
        if cache.len() >= TLS_CONFIG_CACHE_LIMIT {
            cache.clear();
        }
        cache.insert(key, Arc::clone(&config));
    }
    Ok(config)
}

/// 缓存键包含证书路径与文件时间戳：证书轮换后不会继续使用旧配置。
fn tls_cache_key(tls: &TlsConfig) -> String {
    format!(
        "{}\u{1f}{}\u{1f}{}\u{1f}{}\u{1f}{}\u{1f}{}",
        tls.ca_cert,
        file_stamp(&tls.ca_cert),
        tls.client_cert,
        file_stamp(&tls.client_cert),
        tls.client_key,
        file_stamp(&tls.client_key)
    )
}

fn file_stamp(path: &str) -> String {
    if path.is_empty() {
        return String::new();
    }
    match std::fs::metadata(path) {
        Ok(metadata) => {
            let modified = metadata
                .modified()
                .ok()
                .and_then(|time| time.duration_since(std::time::UNIX_EPOCH).ok())
                .map(|duration| duration.as_nanos())
                .unwrap_or(0);
            format!("{}:{modified}", metadata.len())
        }
        Err(_) => "missing".to_string(),
    }
}

/// 解析证书配置并构建 rustls [`ClientConfig`]。
fn build_client_config(tls: &TlsConfig) -> Result<ClientConfig, ContainerError> {
    let mut root_store = RootCertStore::empty();

    if !tls.ca_cert.is_empty() {
        let ca_path = Path::new(&tls.ca_cert);
        let certs = CertificateDer::pem_file_iter(ca_path)
            .map_err(|source| ContainerError::Connect {
                endpoint: format!("ca_cert: {}", tls.ca_cert),
                source: std::io::Error::other(source),
            })?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|source| ContainerError::Connect {
                endpoint: format!("ca_cert: {}", tls.ca_cert),
                source: std::io::Error::other(source),
            })?;

        for cert in certs {
            root_store
                .add(cert)
                .map_err(|error| ContainerError::Connect {
                    endpoint: format!("ca_cert: {}", tls.ca_cert),
                    source: std::io::Error::new(std::io::ErrorKind::InvalidData, error.to_string()),
                })?;
        }
    }

    // 显式指定 ring provider：进程内可能同时链接了其它 rustls 使用方，
    // `ClientConfig::builder()` 依赖全局默认 provider，缺失时会直接 panic。
    let builder =
        ClientConfig::builder_with_provider(Arc::new(rustls::crypto::ring::default_provider()))
            .with_safe_default_protocol_versions()
            .map_err(|error| ContainerError::Connect {
                endpoint: "rustls".to_string(),
                source: std::io::Error::other(error.to_string()),
            })?
            .with_root_certificates(root_store);

    if !tls.client_cert.is_empty() && !tls.client_key.is_empty() {
        let cert_path = Path::new(&tls.client_cert);
        let key_path = Path::new(&tls.client_key);

        let client_certs = CertificateDer::pem_file_iter(cert_path)
            .map_err(|source| ContainerError::Connect {
                endpoint: format!("client_cert: {}", tls.client_cert),
                source: std::io::Error::other(source),
            })?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|source| ContainerError::Connect {
                endpoint: format!("client_cert: {}", tls.client_cert),
                source: std::io::Error::other(source),
            })?;

        let client_key =
            PrivateKeyDer::from_pem_file(key_path).map_err(|source| ContainerError::Connect {
                endpoint: format!("client_key: {}", tls.client_key),
                source: std::io::Error::other(source),
            })?;

        builder
            .with_client_auth_cert(client_certs, client_key)
            .map_err(|error| ContainerError::Connect {
                endpoint: format!("client_auth: {}", tls.client_cert),
                source: std::io::Error::new(std::io::ErrorKind::InvalidData, error.to_string()),
            })
    } else {
        Ok(builder.with_no_client_auth())
    }
}

#[cfg(test)]
mod tests {
    use super::cached_client_config;
    use transom_core::TlsConfig;

    #[test]
    fn identical_tls_config_is_reused_from_cache() {
        let config = TlsConfig::default();
        let first = cached_client_config(&config).expect("空 TLS 配置必须可构建");
        let second = cached_client_config(&config).expect("空 TLS 配置必须可复用");
        assert!(std::sync::Arc::ptr_eq(&first, &second));
    }
}
