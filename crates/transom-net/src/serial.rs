//! 串口终端传输实现。
//!
//! 串口没有 PTY 与窗口尺寸概念：读取使用 100ms 超时轮询，以便在关闭时
//! 及时退出阻塞读；写入直接复用 `serialport` 的 `Write` 实现。读写各自
//! 持有独立克隆的文件描述符，避免同一对象被两个线程同时借用。

use serialport::{DataBits, FlowControl, Parity, SerialPort, SerialPortType, StopBits};
use std::io::{self, Read};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;
use transom_core::{
    AppError, ErrorCode, SerialConnection, SerialDataBits, SerialFlowControl, SerialParity,
    SerialPortDescriptor, SerialPortProvider, SerialStopBits, TerminalControl,
    TerminalControlHandle, TerminalIo,
};

/// 串口读取超时；同时决定关闭请求的最长响应延迟。
const READ_TIMEOUT: Duration = Duration::from_millis(100);

/// 已建立的串口终端通道。
pub struct SerialSession {
    pub io: TerminalIo,
    /// 读取端退出（设备拔出、关闭或读错误）时触发。
    pub exit: tokio::sync::oneshot::Receiver<()>,
}

/// 打开串口并构造与终端控件对接的字节流。
///
/// # Errors
/// 设备路径为空、波特率为 0，或系统拒绝打开/配置串口时返回 `AppError`。
pub fn open_serial(connection: &SerialConnection) -> Result<SerialSession, AppError> {
    if connection.port.trim().is_empty() {
        return Err(AppError::with_detail(
            ErrorCode::Configuration,
            "串口设备路径不能为空",
        ));
    }
    if connection.baud_rate == 0 {
        return Err(AppError::with_detail(
            ErrorCode::Configuration,
            "串口波特率必须大于 0",
        ));
    }

    let port = serialport::new(connection.port.trim(), connection.baud_rate)
        .data_bits(map_data_bits(connection.data_bits))
        .parity(map_parity(connection.parity))
        .stop_bits(map_stop_bits(connection.stop_bits))
        .flow_control(map_flow_control(connection.flow_control))
        .timeout(READ_TIMEOUT)
        .open()
        .map_err(|error| {
            AppError::with_detail(
                ErrorCode::ConnectionFailed,
                format!(
                    "打开串口失败，端口: {}，波特率: {}，原因: {error}",
                    connection.port, connection.baud_rate
                ),
            )
        })?;
    let writer = port.try_clone().map_err(|error| {
        AppError::with_detail(
            ErrorCode::Io,
            format!(
                "复制串口写句柄失败，端口: {}，原因: {error}",
                connection.port
            ),
        )
    })?;

    let closed = Arc::new(AtomicBool::new(false));
    let (control, mut control_rx) = TerminalControlHandle::channel(16);
    let (exit_tx, exit_rx) = tokio::sync::oneshot::channel();
    let control_closed = closed.clone();
    std::thread::Builder::new()
        .name("transom-serial-control".into())
        .spawn(move || {
            while let Some(message) = control_rx.blocking_recv() {
                if matches!(message, TerminalControl::Close) {
                    break;
                }
            }
            // Close 或控制通道断开都意味着终端视图已退出，唤醒阻塞读。
            control_closed.store(true, Ordering::Release);
        })
        .map_err(|error| AppError::with_detail(ErrorCode::Io, error.to_string()))?;

    let reader = SerialReader {
        port,
        closed,
        exit: Some(exit_tx),
    };
    let io = TerminalIo::new(Box::new(reader), writer, control);
    Ok(SerialSession { io, exit: exit_rx })
}

struct SerialReader {
    port: Box<dyn SerialPort>,
    closed: Arc<AtomicBool>,
    exit: Option<tokio::sync::oneshot::Sender<()>>,
}

impl Read for SerialReader {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        loop {
            if self.closed.load(Ordering::Acquire) {
                return Ok(0);
            }
            match self.port.read(buffer) {
                Ok(0) => {
                    // 部分平台在超时时返回 0 而不是 TimedOut；短暂休眠避免忙等。
                    std::thread::sleep(Duration::from_millis(20));
                }
                Ok(count) => return Ok(count),
                Err(error)
                    if matches!(
                        error.kind(),
                        io::ErrorKind::TimedOut
                            | io::ErrorKind::WouldBlock
                            | io::ErrorKind::Interrupted
                    ) => {}
                Err(error) => return Err(error),
            }
        }
    }
}

impl Drop for SerialReader {
    fn drop(&mut self) {
        if let Some(exit) = self.exit.take() {
            let _ = exit.send(());
        }
    }
}

fn map_data_bits(bits: SerialDataBits) -> DataBits {
    match bits {
        SerialDataBits::Five => DataBits::Five,
        SerialDataBits::Six => DataBits::Six,
        SerialDataBits::Seven => DataBits::Seven,
        SerialDataBits::Eight => DataBits::Eight,
    }
}

fn map_parity(parity: SerialParity) -> Parity {
    match parity {
        SerialParity::None => Parity::None,
        SerialParity::Odd => Parity::Odd,
        SerialParity::Even => Parity::Even,
    }
}

fn map_stop_bits(stop_bits: SerialStopBits) -> StopBits {
    match stop_bits {
        SerialStopBits::One => StopBits::One,
        SerialStopBits::Two => StopBits::Two,
    }
}

fn map_flow_control(flow: SerialFlowControl) -> FlowControl {
    match flow {
        SerialFlowControl::None => FlowControl::None,
        SerialFlowControl::Software => FlowControl::Software,
        SerialFlowControl::Hardware => FlowControl::Hardware,
    }
}

/// 基于操作系统设备树的串口枚举实现。
pub struct SystemSerialPortProvider;

impl SerialPortProvider for SystemSerialPortProvider {
    fn list(&self) -> Result<Vec<SerialPortDescriptor>, AppError> {
        // serialport 在异常平台布局下可能 panic（例如缺少 /sys/class/tty），
        // 在硬件边界捕获并降级为可展示错误，避免拖垮整个 UI 线程。
        let ports = std::panic::catch_unwind(serialport::available_ports)
            .map_err(|_| {
                AppError::with_detail(ErrorCode::Io, "串口设备枚举失败：平台接口异常终止")
            })?
            .map_err(|error| {
                AppError::with_detail(ErrorCode::Io, format!("串口设备枚举失败: {error}"))
            })?;

        let mut descriptors: Vec<SerialPortDescriptor> = ports
            .into_iter()
            .map(|info| SerialPortDescriptor {
                port_name: info.port_name,
                description: describe_port_type(&info.port_type),
            })
            .collect();
        descriptors.sort_by(|a, b| a.port_name.cmp(&b.port_name));
        Ok(descriptors)
    }
}

fn describe_port_type(port_type: &SerialPortType) -> Option<String> {
    match port_type {
        SerialPortType::UsbPort(info) => info.product.clone().or_else(|| info.manufacturer.clone()),
        SerialPortType::PciPort => Some("PCI 串口".to_string()),
        SerialPortType::BluetoothPort => Some("蓝牙串口".to_string()),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(unix)]
    #[test]
    fn opens_pty_and_transfers_bytes() {
        use serialport::TTYPort;
        use std::io::Write;

        let (mut master, slave) = TTYPort::pair().unwrap();
        let name = slave.name().unwrap();
        drop(slave);
        let mut session = open_serial(&SerialConnection::new(name, 9_600)).unwrap();

        session.io.writer.write_all(b"ping").unwrap();
        session.io.writer.flush().unwrap();
        master.set_timeout(Duration::from_millis(500)).unwrap();
        let mut received = [0u8; 4];
        let mut total = 0;
        while total < received.len() {
            match master.read(&mut received[total..]) {
                Ok(0) => break,
                Ok(count) => total += count,
                Err(error)
                    if matches!(
                        error.kind(),
                        io::ErrorKind::TimedOut | io::ErrorKind::WouldBlock
                    ) =>
                {
                    break;
                }
                Err(error) => panic!("读取伪终端失败: {error}"),
            }
        }
        assert_eq!(&received[..total], b"ping");

        master.write_all(b"pong").unwrap();
        master.flush().unwrap();
        let mut incoming = [0u8; 4];
        let count = session.io.reader.read(&mut incoming).unwrap();
        assert_eq!(&incoming[..count], b"pong");
    }

    #[test]
    fn choice_enums_map_to_serialport_types() {
        assert!(matches!(
            map_data_bits(SerialDataBits::Seven),
            DataBits::Seven
        ));
        assert!(matches!(map_parity(SerialParity::Even), Parity::Even));
        assert!(matches!(map_stop_bits(SerialStopBits::Two), StopBits::Two));
        assert!(matches!(
            map_flow_control(SerialFlowControl::Hardware),
            FlowControl::Hardware
        ));
    }
}
