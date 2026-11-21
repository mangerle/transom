//! 集成测试共享模块。

/// FTP 回环测试的进程内服务端；仅该测试使用，其他测试 crate 编译
/// common 时按 dead_code 放行。
#[allow(dead_code)]
pub mod mock_ftp;
/// SFTP 回环测试的进程内服务端；仅该测试使用，其他测试 crate 编译
/// common 时按 dead_code 放行。
#[allow(dead_code)]
pub mod mock_sftp;
