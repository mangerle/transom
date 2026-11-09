//! 按端点复用的 HTTP/1.1 连接池。

use super::{REQUEST_TIMEOUT, RequestBody, build_request};
use crate::error::ContainerError;
use crate::transport::EngineTransport;
use bytes::Bytes;
use hyper::Method;
use hyper::StatusCode;
use hyper::body::Incoming;
use hyper::client::conn::http1::SendRequest;
use hyper::header::HeaderMap;
use hyper_util::rt::TokioIo;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

/// 每个引擎端点保留的空闲连接上限。
const POOL_CAPACITY: usize = 4;
/// 可复用的 HTTP/1.1 连接。
struct PooledConnection {
    sender: SendRequest<RequestBody>,
    driver: tokio::task::JoinHandle<hyper::Result<()>>,
}
/// 按端点缓存的空闲连接池。
#[derive(Default)]
pub struct ConnectionPool {
    connections: Mutex<HashMap<String, Vec<PooledConnection>>>,
}
impl ConnectionPool {
    fn take(&self, key: &str) -> Option<PooledConnection> {
        let mut connections = self.connections.lock().ok()?;
        let entries = connections.get_mut(key)?;
        let connection = entries.pop();
        if entries.is_empty() {
            connections.remove(key);
        }
        connection
    }

    fn put(&self, key: String, connection: PooledConnection) {
        let Ok(mut connections) = self.connections.lock() else {
            connection.driver.abort();
            return;
        };
        let entries = connections.entry(key).or_default();
        if entries.len() >= POOL_CAPACITY {
            connection.driver.abort();
            return;
        }
        entries.push(connection);
    }
}

impl std::fmt::Debug for ConnectionPool {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ConnectionPool")
            .field(
                "endpoints",
                &self
                    .connections
                    .lock()
                    .map(|connections| connections.len())
                    .unwrap_or(0),
            )
            .finish()
    }
}
/// 响应体读完且连接健康时，把连接归还到池里；否则中止驱动。
pub struct ConnectionLease {
    key: String,
    pool: Arc<ConnectionPool>,
    connection: Option<PooledConnection>,
    reusable: bool,
}
impl ConnectionLease {
    /// 标记响应体已完整读取，可以在释放时归还连接。
    pub fn mark_reusable(&mut self) {
        self.reusable = true;
    }
}

impl Drop for ConnectionLease {
    fn drop(&mut self) {
        let Some(connection) = self.connection.take() else {
            return;
        };
        if self.reusable && !connection.sender.is_closed() {
            self.pool.put(self.key.clone(), connection);
        } else {
            connection.driver.abort();
        }
    }
}
/// 池化请求的响应：保留响应体与连接租约。
pub struct PooledResponse {
    pub status: StatusCode,
    pub headers: HeaderMap,
    pub body: Incoming,
    pub lease: ConnectionLease,
}
/// 从连接池复用或新建连接，发起一次无请求体的请求。
///
/// 只适用于普通 GET；`exec` 的 Upgrade 与上传流仍然使用独立连接。
pub async fn pooled_request(
    pool: &Arc<ConnectionPool>,
    transport: &EngineTransport,
    method: Method,
    path: &str,
) -> Result<PooledResponse, ContainerError> {
    let operation = format!("{method} {path}");
    let key = transport.describe();
    if let Some(connection) = pool.take(&key) {
        match send_on_connection(connection, method.clone(), path, &operation, &key, pool).await {
            Ok(response) => return Ok(response),
            Err(error) => {
                log::debug!("复用容器引擎连接失败，准备重连，端点: {key}，原因: {error}");
            }
        }
    }
    let connection = open_connection(transport, &operation).await?;
    send_on_connection(connection, method, path, &operation, &key, pool).await
}
async fn open_connection(
    transport: &EngineTransport,
    operation: &str,
) -> Result<PooledConnection, ContainerError> {
    let stream = transport.connect().await?;
    let (sender, connection) = tokio::time::timeout(
        REQUEST_TIMEOUT,
        hyper::client::conn::http1::handshake(TokioIo::new(stream)),
    )
    .await
    .map_err(|_| ContainerError::Http {
        operation: operation.to_string(),
        message: "HTTP 握手超时".to_string(),
    })?
    .map_err(|error| ContainerError::Http {
        operation: operation.to_string(),
        message: error.to_string(),
    })?;
    Ok(PooledConnection {
        sender,
        driver: tokio::spawn(connection),
    })
}
async fn send_on_connection(
    mut connection: PooledConnection,
    method: Method,
    path: &str,
    operation: &str,
    key: &str,
    pool: &Arc<ConnectionPool>,
) -> Result<PooledResponse, ContainerError> {
    // 连接在响应体读完的瞬间就归还进池，但 hyper 的连接驱动可能还没被调度到
    // “愿意接收下一个请求”的状态；此时直接发送会被 hyper 以
    // “connection was not ready”（错误文本显示为 operation was canceled）拒绝。
    // 先等待驱动就绪，避免把一条健康连接误判为失效后重连。
    match tokio::time::timeout(REQUEST_TIMEOUT, connection.sender.ready()).await {
        Ok(Ok(())) => {}
        Ok(Err(error)) => {
            connection.driver.abort();
            return Err(ContainerError::Http {
                operation: operation.to_string(),
                message: error.to_string(),
            });
        }
        Err(_) => {
            connection.driver.abort();
            return Err(ContainerError::Http {
                operation: operation.to_string(),
                message: "等待连接就绪超时".to_string(),
            });
        }
    }
    let request = build_request(method, path, Bytes::new(), None, false)?;
    let response = match tokio::time::timeout(
        REQUEST_TIMEOUT,
        connection.sender.send_request(request),
    )
    .await
    {
        Ok(Ok(response)) => response,
        Ok(Err(error)) => {
            connection.driver.abort();
            return Err(ContainerError::Http {
                operation: operation.to_string(),
                message: error.to_string(),
            });
        }
        Err(_) => {
            connection.driver.abort();
            return Err(ContainerError::Http {
                operation: operation.to_string(),
                message: "请求超时".to_string(),
            });
        }
    };
    let (parts, body) = response.into_parts();
    Ok(PooledResponse {
        status: parts.status,
        headers: parts.headers,
        body,
        lease: ConnectionLease {
            key: key.to_string(),
            pool: Arc::clone(pool),
            connection: Some(connection),
            reusable: false,
        },
    })
}
