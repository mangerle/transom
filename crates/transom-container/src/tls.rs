//! Docker 远端 TLS 连接配置与客户端握手。
//!
//! 设计原理（Why）：生产环境中的 Docker 守护进程通常配置双向 TLS（mTLS），
//! 监听在 2376 端口。通过加载 CA 根证书、客户端证书与私钥，安全建立加密
//! 传输通道，杜绝明文传输导致的宿主机控制权外泄。

use crate::error::ContainerError;
use rustls::pki_types::pem::PemObject;
use rustls::pki_types::{CertificateDer, PrivateKeyDer, ServerName};
use rustls::{ClientConfig, RootCertStore};
use std::path::Path;
use std::sync::Arc;
use tokio::net::TcpStream;
use tokio_rustls::TlsConnector;
use tokio_rustls::client::TlsStream;
use transom_core::TlsConfig;

/// 基于 TLS 配置建立加密的 TCP 流。
///
/// # Errors
/// 证书文件读取失败、格式非法或 TLS 握手失败时返回 [`ContainerError`]。
pub async fn connect_tls(
    tcp_stream: TcpStream,
    host: &str,
    tls_config: &TlsConfig,
) -> Result<TlsStream<TcpStream>, ContainerError> {
    let client_config = build_client_config(tls_config)?;
    let connector = TlsConnector::from(Arc::new(client_config));
    let server_name = ServerName::try_from(host.to_string()).map_err(|error| {
        ContainerError::Connect {
            endpoint: format!("tls://{host}"),
            source: std::io::Error::new(std::io::ErrorKind::InvalidInput, error.to_string()),
        }
    })?;

    connector
        .connect(server_name, tcp_stream)
        .await
        .map_err(|source| ContainerError::Connect {
            endpoint: format!("tls://{host}"),
            source,
        })
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
            root_store.add(cert).map_err(|error| ContainerError::Connect {
                endpoint: format!("ca_cert: {}", tls.ca_cert),
                source: std::io::Error::new(std::io::ErrorKind::InvalidData, error.to_string()),
            })?;
        }
    }

    let builder = ClientConfig::builder().with_root_certificates(root_store);

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

        let client_key = PrivateKeyDer::from_pem_file(key_path).map_err(|source| {
            ContainerError::Connect {
                endpoint: format!("client_key: {}", tls.client_key),
                source: std::io::Error::other(source),
            }
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
