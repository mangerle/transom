//! RDP 客户端强类型错误定义。
//!
//! 设计原理（Why）：底层协议栈、网络 I/O、TLS 握手与剪贴板通道各具不同的失败模式，
//! 采用语义详尽的强类型错误枚举，便于调用方进行精准模式解构与局部容错恢复，
//! 避免将松散宽泛的通用错误直接向上层抛出。

use std::path::PathBuf;
use thiserror::Error;

/// RDP 客户端在连接、协议状态流转与外设交互中可能产生的各类错误。
#[derive(Debug, Error)]
pub enum RdpError {
    /// 网络拨号与传输层建立失败。
    #[error("连接远程目标主机失败，地址: {address}，原因: {source}")]
    Network {
        address: String,
        #[source]
        source: std::io::Error,
    },

    /// 拨号或协议协商阶段超时。
    #[error("连接远程 RDP 主机超时，阶段: {stage}，超时限制: {timeout_secs} 秒")]
    Timeout {
        stage: &'static str,
        timeout_secs: u64,
    },

    /// 代理通道握手失败。
    #[error("经过代理连接 RDP 服务器失败，代理地址: {proxy}，原因: {message}")]
    Proxy { proxy: String, message: String },

    /// TLS 加密通道升级或证书校验失败。
    #[error("TLS 安全握手失败，主机: {host}，原因: {message}")]
    Tls { host: String, message: String },

    /// 网络级别身份验证（NLA / CredSSP）认证失败。
    #[error("远程桌面 NLA/CredSSP 身份认证失败，用户名: {username}，原因: {message}")]
    Authentication { username: String, message: String },

    /// RDP 协议序列或 PDU 解析异常。
    #[error("RDP 协议解析错误，阶段: {stage}，详情: {detail}")]
    Protocol { stage: &'static str, detail: String },

    /// 剪贴板文件复制跨越沙箱受控目录（路径穿越防护）。
    #[error("检测到非法的跨目录文件路径，目标: {path}，拒绝传输")]
    PathTraversal { path: PathBuf },

    /// 本地音频输出设备初始化失败。
    #[error("初始化本地音频播放设备失败，原因: {message}")]
    AudioOutput { message: String },

    /// 会话通信通道异常中断。
    #[error("内部通信通道已断开，方向: {channel}")]
    ChannelClosed { channel: &'static str },

    /// 会话通信通道已满（背压保护）。
    #[error("内部通信通道已满，方向: {channel}")]
    ChannelFull { channel: &'static str },
}
