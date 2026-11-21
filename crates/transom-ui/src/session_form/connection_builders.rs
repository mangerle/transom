//! 会话表单中 Telnet、FTP 与串口的连接参数组装。
//!
//! 设计原理（Why）：这三类连接的字段少、校验同构（主机非空 + 端口范围），
//! 与 SSH/Mosh 的认证组装复杂度不在一个量级；单独成文件让 `builder.rs`
//! 专注 `build_session` 主流程与 SSH 身份/代理逻辑，行数守卫（500 行上限）
//! 也因此保持通过。方法仍归属 [`SessionForm`]，只是 impl 跨文件分布。

use super::types::SessionForm;
use gpui_kit::Context;
use transom_core::{Connection, FtpConnection, SerialConnection, TelnetConnection};
use zeroize::Zeroizing;

impl SessionForm {
    /// 组装 Telnet 连接参数：主机非空、端口合法，编码与选项随表单档位回填。
    ///
    /// # Errors
    /// 主机为空或端口非法/为零时返回本地化校验文案。
    pub(super) fn build_telnet_connection(
        &self,
        cx: &mut Context<impl Sized>,
    ) -> Result<TelnetConnection, String> {
        let host = self.host.read(cx).value().trim().to_string();
        if host.is_empty() {
            return Err(rust_i18n::t!("form.error_host").to_string());
        }
        let port: u16 = self
            .port
            .read(cx)
            .value()
            .trim()
            .parse()
            .map_err(|_| rust_i18n::t!("form.error_port").to_string())?;
        if port == 0 {
            return Err(rust_i18n::t!("form.error_port").to_string());
        }
        let mut options = self
            .base
            .as_ref()
            .and_then(|session| match &session.connection {
                Connection::Telnet(telnet) => Some(telnet.options.clone()),
                _ => None,
            })
            .unwrap_or_default();
        options.auto_reconnect = self.auto_reconnect;
        if let Ok(timeout) = self.connect_timeout.read(cx).value().trim().parse::<u64>()
            && (1..=120).contains(&timeout)
        {
            options.connect_timeout_secs = timeout;
        }
        if let Ok(keepalive) = self.keepalive.read(cx).value().trim().parse::<u64>()
            && keepalive <= 3600
        {
            options.keepalive_secs = keepalive;
        }
        Ok(TelnetConnection {
            host,
            port,
            encoding: self.telnet_encoding,
            options,
        })
    }

    /// 组装 FTP 连接参数与待保存密码。
    ///
    /// 匿名登录不隐式处理：用户名必填，匿名由用户显式填写 `anonymous`；
    /// 密码允许为空（部分匿名/免密服务端不设口令），非空时随 `BuiltSession`
    /// 走钥匙串写入，会话文件永不落明文。
    ///
    /// # Errors
    /// 主机为空、端口非法或用户名为空时返回本地化校验文案。
    pub(super) fn build_ftp_connection(
        &self,
        cx: &mut Context<impl Sized>,
    ) -> Result<(Connection, Option<Zeroizing<String>>), String> {
        let host = self.host.read(cx).value().trim().to_string();
        if host.is_empty() {
            return Err(rust_i18n::t!("form.error_host").to_string());
        }
        let port: u16 = self
            .port
            .read(cx)
            .value()
            .trim()
            .parse()
            .map_err(|_| rust_i18n::t!("form.error_port").to_string())?;
        if port == 0 {
            return Err(rust_i18n::t!("form.error_port").to_string());
        }
        let username = self.username.read(cx).value().trim().to_string();
        if username.is_empty() {
            return Err(rust_i18n::t!("form.error_ftp_username").to_string());
        }
        let password_value = self.password.read(cx).value().to_string();
        let password = (!password_value.is_empty()).then(|| Zeroizing::new(password_value));
        Ok((
            Connection::Ftp(FtpConnection {
                host,
                port,
                username,
                security: self.ftp_security,
                passive: self.ftp_passive,
            }),
            password,
        ))
    }

    /// 组装串口连接参数：设备名非空且波特率合法。
    ///
    /// # Errors
    /// 设备名为空或波特率非法/为零时返回本地化校验文案。
    pub(super) fn build_serial_connection(
        &self,
        cx: &mut Context<impl Sized>,
    ) -> Result<SerialConnection, String> {
        let port = self.serial_port.read(cx).value().trim().to_string();
        if port.is_empty() {
            return Err(rust_i18n::t!("form.error_serial_port").to_string());
        }
        let baud_rate: u32 = self
            .serial_baud
            .read(cx)
            .value()
            .trim()
            .parse()
            .map_err(|_| rust_i18n::t!("form.error_serial_baud").to_string())?;
        if baud_rate == 0 {
            return Err(rust_i18n::t!("form.error_serial_baud").to_string());
        }
        Ok(SerialConnection {
            port,
            baud_rate,
            data_bits: self.serial_data_bits,
            parity: self.serial_parity,
            stop_bits: self.serial_stop_bits,
            flow_control: self.serial_flow_control,
        })
    }
}
