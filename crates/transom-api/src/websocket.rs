//! WebSocket 连接、发送与接收事件服务。

use super::{ApiAuth, ApiEnvironment, ApiHeader};
use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use futures::{SinkExt, StreamExt};
use tokio::runtime::Handle;
use tokio::sync::{mpsc, oneshot};
use tokio::task::JoinHandle;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::http::{HeaderName, HeaderValue};

const COMMAND_CAPACITY: usize = 32;
const EVENT_CAPACITY: usize = 64;
const CONNECT_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(15);
const IO_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(10);

/// WebSocket 控制命令。
pub(crate) enum WsCommand {
    Message(String),
    Close,
}

/// WebSocket UI 收到的状态和消息。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ApiWebSocketEvent {
    Opened,
    Message(String),
    Closed(String),
    Error(String),
}

/// WebSocket 连接句柄；Drop 时中止任务并关闭连接。
pub struct ApiWebSocketHandle {
    command_tx: mpsc::Sender<WsCommand>,
    event_rx: mpsc::Receiver<ApiWebSocketEvent>,
    task: Option<JoinHandle<()>>,
}

/// WebSocket 启动句柄。
pub struct ApiWebSocketStart {
    receiver: oneshot::Receiver<Result<ApiWebSocketHandle, String>>,
    task: Option<JoinHandle<()>>,
}

/// 应用级 WebSocket 服务。
#[derive(Clone)]
pub struct ApiWebSocketService {
    runtime: Handle,
}

impl ApiWebSocketService {
    /// 使用应用控制面 Tokio Runtime 构造服务。
    pub fn new(runtime: Handle) -> Self {
        Self { runtime }
    }

    /// 新建连接任务；调用方等待连接结果后再启用发送按钮。
    pub fn start(
        &self,
        url: String,
        headers: Vec<ApiHeader>,
        environment: ApiEnvironment,
        auth: ApiAuth,
    ) -> ApiWebSocketStart {
        let (result_tx, result_rx) = oneshot::channel();
        let (command_tx, command_rx) = mpsc::channel(COMMAND_CAPACITY);
        let (event_tx, event_rx) = mpsc::channel(EVENT_CAPACITY);
        let task = self.runtime.spawn(async move {
            let result = connect(url, headers, environment, auth, command_rx, event_tx).await;
            match result {
                Ok(nested_task) => {
                    let _ = result_tx.send(Ok(ApiWebSocketHandle {
                        command_tx,
                        event_rx,
                        task: Some(nested_task),
                    }));
                }
                Err(error) => {
                    let _ = result_tx.send(Err(error));
                }
            }
        });
        ApiWebSocketStart {
            receiver: result_rx,
            task: Some(task),
        }
    }
}

impl ApiWebSocketStart {
    /// 等待连接建立。
    ///
    /// # Errors
    /// URL、请求头或握手失败时返回中文错误。
    pub async fn into_handle(mut self) -> Result<ApiWebSocketHandle, String> {
        let result = (&mut self.receiver)
            .await
            .map_err(|_| "WebSocket 连接任务在返回结果前结束".to_string())?;
        self.task.take();
        result
    }
}

impl Drop for ApiWebSocketStart {
    fn drop(&mut self) {
        if let Some(task) = self.task.take() {
            task.abort();
        }
    }
}

impl ApiWebSocketHandle {
    /// 非阻塞发送文本；命令队列满或连接关闭时快速失败。
    pub fn try_send_text(&self, text: String) -> Result<(), String> {
        self.command_tx
            .try_send(WsCommand::Message(text))
            .map_err(|error| format!("WebSocket 发送队列不可用: {error}"))
    }

    /// 发送文本消息。
    pub async fn send_text(&self, text: String) -> Result<(), String> {
        self.command_tx
            .send(WsCommand::Message(text))
            .await
            .map_err(|_| "WebSocket 连接已关闭".to_string())
    }

    /// 请求关闭连接。
    pub fn close(&self) {
        let _ = self.command_tx.try_send(WsCommand::Close);
    }

    /// 取走事件接收端，交给 GPUI 异步任务逐条消费。
    pub fn take_events(&mut self) -> mpsc::Receiver<ApiWebSocketEvent> {
        let (_, placeholder) = mpsc::channel(1);
        std::mem::replace(&mut self.event_rx, placeholder)
    }
}

impl Drop for ApiWebSocketHandle {
    fn drop(&mut self) {
        if let Some(task) = self.task.take() {
            task.abort();
        }
    }
}

async fn connect(
    url: String,
    headers: Vec<ApiHeader>,
    environment: ApiEnvironment,
    auth: ApiAuth,
    command_rx: mpsc::Receiver<WsCommand>,
    event_tx: mpsc::Sender<ApiWebSocketEvent>,
) -> Result<JoinHandle<()>, String> {
    let mut request = url
        .into_client_request()
        .map_err(|error| format!("构建 WebSocket 请求失败: {error}"))?;
    for header in headers.iter().filter(|header| header.enabled) {
        let name = environment.resolve(&header.name);
        let value = environment.resolve(&header.value);
        let header_name: HeaderName = name
            .parse()
            .map_err(|error| format!("WebSocket 请求头名称无效，名称: {name}，原因: {error}"))?;
        request.headers_mut().insert(
            header_name,
            HeaderValue::from_str(&value).map_err(|error| {
                format!("WebSocket 请求头内容无效，名称: {name}，原因: {error}")
            })?,
        );
    }
    apply_websocket_auth(&mut request, &environment, auth)?;
    let (socket, _) =
        tokio::time::timeout(CONNECT_TIMEOUT, tokio_tungstenite::connect_async(request))
            .await
            .map_err(|_| "WebSocket 握手超时，目标未在 15 秒内完成连接".to_string())?
            .map_err(|error| format!("WebSocket 握手失败: {error}"))?;
    let _ = event_tx.send(ApiWebSocketEvent::Opened).await;
    let (mut write, mut read) = socket.split();
    let mut command_rx = command_rx;
    let task = tokio::spawn(async move {
        loop {
            tokio::select! {
                command = command_rx.recv() => match command {
                    Some(WsCommand::Message(text)) => {
                        match tokio::time::timeout(IO_TIMEOUT, write.send(Message::Text(text.into()))).await {
                            Ok(Ok(())) => {}
                            Ok(Err(error)) => {
                                let _ = event_tx.send(ApiWebSocketEvent::Error(format!("发送 WebSocket 消息失败: {error}"))).await;
                                break;
                            }
                            Err(_) => {
                                let _ = event_tx.send(ApiWebSocketEvent::Error("发送 WebSocket 消息超时".to_string())).await;
                                break;
                            }
                        }
                    }
                    Some(WsCommand::Close) | None => {
                        if let Ok(Err(error)) = tokio::time::timeout(IO_TIMEOUT, write.close()).await {
                            let _ = event_tx.send(ApiWebSocketEvent::Error(format!("关闭 WebSocket 连接失败: {error}"))).await;
                        }
                        break;
                    }
                },
                message = read.next() => match message {
                    Some(Ok(Message::Text(text))) => {
                        let _ = event_tx.send(ApiWebSocketEvent::Message(text.to_string())).await;
                    }
                    Some(Ok(Message::Binary(bytes))) => {
                        let _ = event_tx.send(ApiWebSocketEvent::Message(format!("[二进制 {} 字节]", bytes.len()))).await;
                    }
                    Some(Ok(Message::Close(frame))) => {
                        let _ = event_tx.send(ApiWebSocketEvent::Closed(frame.map_or_else(|| "连接已关闭".to_string(), |frame| frame.reason.to_string()))).await;
                        break;
                    }
                    Some(Ok(Message::Ping(_)) | Ok(Message::Pong(_)) | Ok(Message::Frame(_))) => {}
                    Some(Err(error)) => {
                        let _ = event_tx.send(ApiWebSocketEvent::Error(format!("接收 WebSocket 消息失败: {error}"))).await;
                        break;
                    }
                    None => {
                        let _ = event_tx.send(ApiWebSocketEvent::Closed("连接已关闭".to_string())).await;
                        break;
                    }
                }
            }
        }
    });
    Ok(task)
}

fn apply_websocket_auth(
    request: &mut tokio_tungstenite::tungstenite::http::Request<()>,
    environment: &ApiEnvironment,
    auth: ApiAuth,
) -> Result<(), String> {
    let value = match auth {
        ApiAuth::None => return Ok(()),
        ApiAuth::Bearer { token_env } => {
            let token = environment_value(environment, &token_env)?;
            format!("Bearer {token}")
        }
        ApiAuth::Basic {
            username_env,
            password_env,
        } => {
            let username = environment_value(environment, &username_env)?;
            let password = environment_value(environment, &password_env)?;
            format!(
                "Basic {}",
                STANDARD.encode(format!("{username}:{password}"))
            )
        }
        ApiAuth::OAuth2ClientCredentials { .. } | ApiAuth::AwsSigV4 { .. } => {
            return Err("WebSocket 暂只支持 Bearer、Basic 与自定义请求头认证".to_string());
        }
    };
    request.headers_mut().insert(
        "authorization",
        HeaderValue::from_str(&value)
            .map_err(|error| format!("WebSocket 认证头内容无效: {error}"))?,
    );
    Ok(())
}

fn environment_value(environment: &ApiEnvironment, name: &str) -> Result<String, String> {
    environment
        .variables
        .iter()
        .find(|variable| variable.enabled && variable.name == name)
        .map(|variable| variable.value.clone())
        .filter(|value| !value.is_empty())
        .ok_or_else(|| format!("WebSocket 认证环境变量 {name} 未配置"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::net::TcpListener;

    #[tokio::test]
    async fn rejects_invalid_websocket_url() {
        let service = ApiWebSocketService::new(Handle::current());
        let start = service.start(
            "not-a-url".to_string(),
            Vec::new(),
            ApiEnvironment::empty(),
            ApiAuth::None,
        );
        let result = start.into_handle().await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn connects_to_local_websocket_server() {
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("绑定测试端口");
        let address = listener.local_addr().expect("读取测试地址");
        let server = tokio::spawn(async move {
            let (stream, _) = listener.accept().await.expect("接受连接");
            let _ = tokio_tungstenite::accept_async(stream)
                .await
                .expect("WebSocket 握手");
        });
        let service = ApiWebSocketService::new(Handle::current());
        let handle = service
            .start(
                format!("ws://{address}"),
                Vec::new(),
                ApiEnvironment::empty(),
                ApiAuth::None,
            )
            .into_handle()
            .await;
        assert!(handle.is_ok());
        server.abort();
    }
}
