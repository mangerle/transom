//! 串口设备发现的应用层契约。
//!
//! UI 只依赖本 trait，不直接链接平台串口库；实际枚举由应用入口注入。
//! 这样便于在无硬件环境（CI、单元测试）替换为空实现，也避免 UI crate
//! 被迫承担系统设备访问依赖。

use crate::error::AppError;

/// 已发现的串口设备。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SerialPortDescriptor {
    /// 可直接传给串口打开函数的设备路径，例如 `/dev/ttyUSB0` 或 `COM3`。
    pub port_name: String,
    /// 厂商/产品描述，无法获取时为 `None`。
    pub description: Option<String>,
}

/// 串口设备枚举服务。
pub trait SerialPortProvider: Send + Sync {
    /// 列出当前系统可用的串口。
    ///
    /// # Errors
    /// 平台设备枚举失败时返回 `AppError`；调用方应允许用户手动输入设备路径。
    fn list(&self) -> Result<Vec<SerialPortDescriptor>, AppError>;
}
