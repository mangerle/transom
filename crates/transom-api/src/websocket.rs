//! WebSocket 连接、发送与接收事件服务。

use super::ApiExecutionPlan;
use crate::http_support::safe_url_label;
use futures::stream::SplitSink;
use futures::{SinkExt, StreamExt};
use thiserror::Error;
use tokio::net::TcpStream;
use tokio::runtime::Handle;
use tokio::sync::{mpsc, oneshot, watch};
use tokio::task::JoinHandle;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::tungstenite::protocol::WebSocketConfig;
use tokio_tungstenite::{MaybeTlsStream, WebSocketStream};

mod handshake;

use handshake::build_handshake;

const COMMAND_CAPACITY: usize = 32;
const EVENT_CAPACITY: usize = 16;
const MAX_MESSAGE_BYTES: usize = 512 * 1024;
const WRITE_BUFFER_BYTES: usize = 32 * 1024;
const MAX_WRITE_BUFFER_BYTES: usize = 2 * 1024 * 1024;
const CONNECT_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(15);
const IO_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(10);

/// WebSocket 控制命令。
pub(crate) enum WsCommand {
    Message(String),
}

/// WebSocket 连接、认证、消息与任务生命周期错误。
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum ApiWebSocketError {
    #[error("构建 WebSocket 请求失败，目标: {host}，原因: {reason}")]
    BuildRequest { host: String, reason: String },
    #[error("WebSocket 请求头名称无效，名称: {name}，原因: {reason}")]
    InvalidHeaderName { name: String, reason: String },
    #[error("WebSocket 请求头内容无效，名称: {name}，原因: {reason}")]
    InvalidHeaderValue { name: String, reason: String },
    #[error("WebSocket 认证配置无效，原因: {reason}")]
    InvalidAuth { reason: String },
    #[error("WebSocket 暂时不支持 {kind} 认证")]
    UnsupportedAuth { kind: String },
    #[error("WebSocket 握手超时，目标: {host}，超时: {seconds} 秒")]
    ConnectTimeout { host: String, seconds: u64 },
    #[error("WebSocket 握手失败，目标: {host}，原因: {reason}")]
    Connect { host: String, reason: String },
    #[error("WebSocket 发送队列不可用，原因: {reason}")]
    SendQueue { reason: String },
    #[error("WebSocket 消息超过 {limit} 字节上限")]
    MessageTooLarge { limit: usize },
    #[error("WebSocket 连接任务在返回结果前结束")]
    TaskEnded,
    #[error("WebSocket 连接任务异常退出，原因: {reason}")]
    TaskFailed { reason: String },
    #[error("WebSocket 事件接收端已关闭")]
    EventReceiverClosed,
}

/// WebSocket UI 收到的状态和消息。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ApiWebSocketEvent {
    Opened,
    Message(String),
    Closed(String),
    Error(String),
}

/// WebSocket 连接句柄；Drop 时发出关闭意图并让连接任务自行收尾。
pub struct ApiWebSocketHandle {
    command_tx: mpsc::Sender<WsCommand>,
    close_tx: watch::Sender<bool>,
    event_rx: mpsc::Receiver<ApiWebSocketEvent>,
    task: Option<JoinHandle<()>>,
}

/// WebSocket 启动句柄。
pub struct ApiWebSocketStart {
    receiver: oneshot::Receiver<Result<ApiWebSocketHandle, ApiWebSocketError>>,
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
    pub fn start(&self, plan: ApiExecutionPlan) -> ApiWebSocketStart {
        let (result_tx, result_rx) = oneshot::channel();
        let (command_tx, command_rx) = mpsc::channel(COMMAND_CAPACITY);
        let (close_tx, close_rx) = watch::channel(false);
        let (event_tx, event_rx) = mpsc::channel(EVENT_CAPACITY);
        let task = self.runtime.spawn(async move {
            let result = connect(plan, command_rx, close_rx, event_tx).await;
            match result {
                Ok(nested_task) => {
                    let _ = result_tx.send(Ok(ApiWebSocketHandle {
                        command_tx,
                        close_tx,
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
    pub async fn into_handle(mut self) -> Result<ApiWebSocketHandle, ApiWebSocketError> {
        let result = (&mut self.receiver)
            .await
            .map_err(|_| ApiWebSocketError::TaskEnded)?;
        if let Some(task) = self.task.take() {
            task.await.map_err(|error| ApiWebSocketError::TaskFailed {
                reason: error.to_string(),
            })?;
        }
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
    ///
    /// # Errors
    /// 消息超过上限、连接已关闭或发送队列已满时返回
    /// [`ApiWebSocketError`]。
    pub fn try_send_text(&self, text: String) -> Result<(), ApiWebSocketError> {
        validate_message_size(text.len())?;
        self.command_tx
            .try_send(WsCommand::Message(text))
            .map_err(|error| ApiWebSocketError::SendQueue {
                reason: error.to_string(),
            })
    }

    /// 发送文本消息。
    ///
    /// # Errors
    /// 消息超过上限或连接已关闭时返回 [`ApiWebSocketError`]。
    pub async fn send_text(&self, text: String) -> Result<(), ApiWebSocketError> {
        validate_message_size(text.len())?;
        self.command_tx
            .send(WsCommand::Message(text))
            .await
            .map_err(|_| ApiWebSocketError::SendQueue {
                reason: "WebSocket 连接已关闭".to_string(),
            })
    }

    /// 请求关闭连接。
    ///
    /// 关闭请求通过独立的 watch 通道立即生效，不依赖消息队列剩余容量。
    ///
    /// # Errors
    /// 连接任务已经停止时返回 [`ApiWebSocketError::SendQueue`]。
    pub fn request_close(&self) -> Result<(), ApiWebSocketError> {
        self.close_tx
            .send(true)
            .map_err(|_| ApiWebSocketError::SendQueue {
                reason: "WebSocket 连接已关闭".to_string(),
            })
    }

    /// 请求关闭并等待连接任务完成发送侧收尾。
    ///
    /// # Errors
    /// 关闭请求无法提交，或连接任务异常退出时返回
    /// [`ApiWebSocketError`]。
    pub async fn shutdown(mut self) -> Result<(), ApiWebSocketError> {
        self.request_close()?;
        if let Some(task) = self.task.take() {
            task.await.map_err(|error| ApiWebSocketError::TaskFailed {
                reason: error.to_string(),
            })?;
        }
        Ok(())
    }

    /// 取走事件接收端，交给 GPUI 异步任务逐条消费。
    pub fn take_events(&mut self) -> mpsc::Receiver<ApiWebSocketEvent> {
        let (_, placeholder) = mpsc::channel(1);
        std::mem::replace(&mut self.event_rx, placeholder)
    }
}

impl Drop for ApiWebSocketHandle {
    fn drop(&mut self) {
        let _ = self.close_tx.send(true);
    }
}

fn validate_message_size(size: usize) -> Result<(), ApiWebSocketError> {
    if size > MAX_MESSAGE_BYTES {
        Err(ApiWebSocketError::MessageTooLarge {
            limit: MAX_MESSAGE_BYTES,
        })
    } else {
        Ok(())
    }
}

type WsWrite = SplitSink<WebSocketStream<MaybeTlsStream<TcpStream>>, Message>;

async fn connect(
    plan: ApiExecutionPlan,
    command_rx: mpsc::Receiver<WsCommand>,
    close_rx: watch::Receiver<bool>,
    event_tx: mpsc::Sender<ApiWebSocketEvent>,
) -> Result<JoinHandle<()>, ApiWebSocketError> {
    let (request, environment, cookies) = plan.into_parts();
    let host = safe_url_label(&request.url);
    let handshake = build_handshake(&request, &environment, &cookies)?;
    let config = WebSocketConfig::default()
        .read_buffer_size(64 * 1024)
        .write_buffer_size(WRITE_BUFFER_BYTES)
        .max_write_buffer_size(MAX_WRITE_BUFFER_BYTES)
        .max_message_size(Some(MAX_MESSAGE_BYTES))
        .max_frame_size(Some(MAX_MESSAGE_BYTES));
    let socket = tokio::time::timeout(
        CONNECT_TIMEOUT,
        tokio_tungstenite::connect_async_with_config(handshake, Some(config), false),
    )
    .await
    .map_err(|_| ApiWebSocketError::ConnectTimeout {
        host: host.clone(),
        seconds: CONNECT_TIMEOUT.as_secs(),
    })?
    .map_err(|error| ApiWebSocketError::Connect {
        host,
        reason: error.to_string(),
    })?
    .0;
    if event_tx.send(ApiWebSocketEvent::Opened).await.is_err() {
        return Err(ApiWebSocketError::EventReceiverClosed);
    }
    Ok(tokio::spawn(run_socket(
        socket, command_rx, close_rx, event_tx,
    )))
}

async fn run_socket(
    socket: WebSocketStream<MaybeTlsStream<TcpStream>>,
    mut command_rx: mpsc::Receiver<WsCommand>,
    mut close_rx: watch::Receiver<bool>,
    event_tx: mpsc::Sender<ApiWebSocketEvent>,
) {
    let (mut write, mut read) = socket.split();
    loop {
        tokio::select! {
            command = command_rx.recv() => match command {
                Some(WsCommand::Message(text)) => {
                    if !send_message(&mut write, text, &event_tx).await {
                        break;
                    }
                }
                None => {
                    close_socket(&mut write, &event_tx).await;
                    break;
                }
            },
            close = close_rx.changed() => {
                let should_close = close.is_err() || *close_rx.borrow_and_update();
                if should_close {
                    close_socket(&mut write, &event_tx).await;
                    break;
                }
            },
            message = read.next() => {
                if !handle_incoming(&event_tx, message).await {
                    break;
                }
            }
        }
    }
}

async fn send_message(
    write: &mut WsWrite,
    text: String,
    event_tx: &mpsc::Sender<ApiWebSocketEvent>,
) -> bool {
    match tokio::time::timeout(IO_TIMEOUT, write.send(Message::Text(text.into()))).await {
        Ok(Ok(())) => true,
        Ok(Err(error)) => {
            emit_event(
                event_tx,
                ApiWebSocketEvent::Error(format!("发送 WebSocket 消息失败: {error}")),
            )
            .await;
            false
        }
        Err(_) => {
            emit_event(
                event_tx,
                ApiWebSocketEvent::Error("发送 WebSocket 消息超时".to_string()),
            )
            .await;
            false
        }
    }
}

async fn handle_incoming(
    event_tx: &mpsc::Sender<ApiWebSocketEvent>,
    message: Option<Result<Message, tokio_tungstenite::tungstenite::Error>>,
) -> bool {
    let event = match message {
        Some(Ok(Message::Text(text))) => ApiWebSocketEvent::Message(text.to_string()),
        Some(Ok(Message::Binary(bytes))) => {
            ApiWebSocketEvent::Message(format!("[二进制 {} 字节]", bytes.len()))
        }
        Some(Ok(Message::Close(frame))) => ApiWebSocketEvent::Closed(frame.map_or_else(
            || "连接已关闭".to_string(),
            |frame| frame.reason.to_string(),
        )),
        Some(Ok(Message::Ping(_) | Message::Pong(_) | Message::Frame(_))) => return true,
        Some(Err(error)) => ApiWebSocketEvent::Error(format!("接收 WebSocket 消息失败: {error}")),
        None => ApiWebSocketEvent::Closed("连接已关闭".to_string()),
    };
    let keep_open = !matches!(
        event,
        ApiWebSocketEvent::Closed(_) | ApiWebSocketEvent::Error(_)
    );
    emit_event(event_tx, event).await && keep_open
}

async fn close_socket(write: &mut WsWrite, event_tx: &mpsc::Sender<ApiWebSocketEvent>) {
    match tokio::time::timeout(IO_TIMEOUT, write.close()).await {
        Ok(Ok(())) => {}
        Ok(Err(error)) => {
            emit_event(
                event_tx,
                ApiWebSocketEvent::Error(format!("关闭 WebSocket 连接失败: {error}")),
            )
            .await;
        }
        Err(_) => {
            emit_event(
                event_tx,
                ApiWebSocketEvent::Error("关闭 WebSocket 连接超时".to_string()),
            )
            .await;
        }
    }
}

async fn emit_event(event_tx: &mpsc::Sender<ApiWebSocketEvent>, event: ApiWebSocketEvent) -> bool {
    event_tx.send(event).await.is_ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{ApiAuth, ApiEnvironment, ApiMethod, ApiProtocol, ApiRequest};
    use tokio::net::TcpListener;

    fn plan(url: &str, environment: ApiEnvironment, auth: ApiAuth) -> ApiExecutionPlan {
        let mut request = ApiRequest::new("WebSocket 测试", None);
        request.protocol = ApiProtocol::WebSocket;
        request.method = ApiMethod::Get;
        request.url = url.to_string();
        request.auth = auth;
        ApiExecutionPlan::new(request, environment, Vec::new()).expect("测试执行计划应合法")
    }

    #[tokio::test]
    async fn rejects_invalid_websocket_url() {
        let service = ApiWebSocketService::new(Handle::current());
        let start = service.start(plan("not-a-url", ApiEnvironment::empty(), ApiAuth::None));
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
            .start(plan(
                &format!("ws://{address}"),
                ApiEnvironment::empty(),
                ApiAuth::None,
            ))
            .into_handle()
            .await;
        assert!(handle.is_ok());
        server.abort();
    }
}
