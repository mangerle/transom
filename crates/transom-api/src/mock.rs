//! 本地 Mock HTTP 服务。

use super::ApiHeader;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::runtime::Handle;
use tokio::sync::oneshot;
use tokio::task::JoinHandle;

/// 一条 Mock 路由。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct ApiMockRoute {
    pub method: String,
    pub path: String,
    pub status: u16,
    #[serde(default)]
    pub headers: Vec<ApiHeader>,
    #[serde(default)]
    pub body: String,
    #[serde(default)]
    pub delay_millis: u64,
}

/// Mock 服务启动结果；等待端口时若被取消会中止底层任务。
pub struct ApiMockStart {
    receiver: oneshot::Receiver<u16>,
    task: Option<JoinHandle<()>>,
    stop: Arc<AtomicBool>,
}

/// 已启动的 Mock 服务句柄；Drop 时发出停止信号并中止监听任务。
pub struct ApiMockHandle {
    pub port: u16,
    task: Option<JoinHandle<()>>,
    stop: Arc<AtomicBool>,
}

/// 本地 Mock 服务；使用独立任务和 127.0.0.1 回环地址。
#[derive(Clone)]
pub struct ApiMockService {
    runtime: Handle,
}

impl ApiMockService {
    /// 使用应用控制面的 Tokio Runtime 构造服务。
    pub fn new(runtime: Handle) -> Self {
        Self { runtime }
    }

    /// 启动 Mock HTTP 服务，等待端口绑定后再返回句柄。
    ///
    /// # Errors
    /// 端口绑定失败或任务在绑定前结束时返回中文错误。
    pub async fn start(&self, routes: Vec<ApiMockRoute>) -> Result<ApiMockHandle, String> {
        let start = self.spawn(routes);
        start.into_handle().await
    }

    /// 启动任务并返回可等待端口的启动句柄。
    pub fn spawn(&self, routes: Vec<ApiMockRoute>) -> ApiMockStart {
        let (sender, receiver) = oneshot::channel();
        let stop = Arc::new(AtomicBool::new(false));
        let task_stop = Arc::clone(&stop);
        let task = self.runtime.spawn(async move {
            match TcpListener::bind(("127.0.0.1", 0)).await {
                Ok(listener) => {
                    let port = listener
                        .local_addr()
                        .map(|address| address.port())
                        .unwrap_or(0);
                    if sender.send(port).is_err() {
                        return;
                    }
                    serve(listener, routes, task_stop).await;
                }
                Err(error) => {
                    let _ = sender.send(0);
                    log::warn!("启动 Mock HTTP 服务失败，原因: {error}");
                }
            }
        });
        ApiMockStart {
            receiver,
            task: Some(task),
            stop,
        }
    }
}

impl ApiMockStart {
    /// 等待端口绑定并转移任务所有权。
    ///
    /// # Errors
    /// 绑定失败或任务在返回端口前结束时返回中文错误。
    pub async fn into_handle(mut self) -> Result<ApiMockHandle, String> {
        let port = (&mut self.receiver)
            .await
            .map_err(|_| "Mock HTTP 服务在返回端口前结束".to_string())?;
        if port == 0 {
            return Err("Mock HTTP 服务未能绑定本地端口".to_string());
        }
        Ok(ApiMockHandle {
            port,
            task: self.task.take(),
            stop: Arc::clone(&self.stop),
        })
    }
}

impl Drop for ApiMockStart {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        if let Some(task) = self.task.take() {
            task.abort();
        }
    }
}

impl Drop for ApiMockHandle {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        if let Some(task) = self.task.take() {
            task.abort();
        }
    }
}

async fn serve(listener: TcpListener, routes: Vec<ApiMockRoute>, stop: Arc<AtomicBool>) {
    loop {
        if stop.load(Ordering::Acquire) {
            return;
        }
        let Ok((stream, _)) = listener.accept().await else {
            return;
        };
        if let Err(error) = serve_connection(stream, &routes).await {
            log::warn!("Mock HTTP 连接处理失败，原因: {error}");
        }
    }
}

async fn serve_connection(mut stream: TcpStream, routes: &[ApiMockRoute]) -> std::io::Result<()> {
    let mut bytes = Vec::with_capacity(1024);
    let mut buffer = [0_u8; 1024];
    loop {
        let read = stream.read(&mut buffer).await?;
        if read == 0 {
            break;
        }
        bytes.extend_from_slice(&buffer[..read]);
        if bytes.windows(4).any(|window| window == b"\r\n\r\n") {
            break;
        }
    }
    let request_line = bytes
        .split(|byte| *byte == b'\n')
        .next()
        .and_then(|line| std::str::from_utf8(line).ok())
        .unwrap_or_default();
    let mut parts = request_line.split_whitespace();
    let method = parts.next().unwrap_or_default();
    let path = parts.next().unwrap_or_default();
    let route = routes
        .iter()
        .find(|route| route.method.eq_ignore_ascii_case(method) && route.path == path);
    let response = match route {
        Some(route) => {
            if route.delay_millis > 0 {
                tokio::time::sleep(std::time::Duration::from_millis(route.delay_millis)).await;
            }
            let body = route.body.as_bytes();
            let mut headers = String::new();
            for header in route.headers.iter().filter(|header| header.enabled) {
                headers.push_str(&format!("{}: {}\r\n", header.name, header.value));
            }
            let mut response = format!(
                "HTTP/1.1 {} {}\r\n{}Content-Type: text/plain; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                route.status,
                status_text(route.status),
                headers,
                body.len()
            )
            .into_bytes();
            response.extend_from_slice(body);
            response
        }
        None => {
            b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n".to_vec()
        }
    };
    stream.write_all(&response).await
}

fn status_text(status: u16) -> &'static str {
    match status {
        200 => "OK",
        201 => "Created",
        202 => "Accepted",
        204 => "No Content",
        400 => "Bad Request",
        401 => "Unauthorized",
        403 => "Forbidden",
        404 => "Not Found",
        409 => "Conflict",
        429 => "Too Many Requests",
        500 => "Internal Server Error",
        _ => "Mock Response",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::AsyncReadExt;

    #[tokio::test]
    async fn mock_server_returns_configured_response() {
        let service = ApiMockService::new(Handle::current());
        let handle = service
            .start(vec![ApiMockRoute {
                method: "GET".to_string(),
                path: "/ping".to_string(),
                status: 200,
                headers: Vec::new(),
                body: "pong".to_string(),
                delay_millis: 0,
            }])
            .await
            .expect("Mock 服务应启动");
        let mut stream = TcpStream::connect(("127.0.0.1", handle.port))
            .await
            .expect("连接 Mock 服务");
        stream
            .write_all(b"GET /ping HTTP/1.1\r\nHost: localhost\r\n\r\n")
            .await
            .expect("写入请求");
        let mut response = Vec::with_capacity(128);
        stream.read_to_end(&mut response).await.expect("读取响应");
        let text = String::from_utf8(response).expect("UTF-8 响应");
        assert!(text.starts_with("HTTP/1.1 200 OK"));
        assert!(text.contains("pong"), "实际响应: {text}");
    }
}
