//! Telnet 终端传输实现（RFC 854/855 最小客户端）子模块。

mod connect;
mod stream;

#[cfg(test)]
mod tests;

pub use connect::connect_telnet;
pub use stream::TelnetSession;
