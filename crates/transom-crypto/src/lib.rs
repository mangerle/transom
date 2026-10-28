//! 同步与备份的加密封装：TSYN 二进制信封与 JSON/YAML 内嵌信封。
//!
//! 设计原理（Why）：
//! 两种信封都是"文档落在不可信介质"（公开 S3 桶、私有 Git 仓库、导出的备份文件）
//! 上的线格式契约，被网络传输层（云同步/Git 同步）与存储层（备份读写）共用。
//! 独立为专用的 `transom-crypto` crate，既消除了 `transom-net` 与 `transom-store`
//! 之间的双向耦合，又避免让纯粹的数据模型契约底座 `transom-core` 背负沉重的加解密算法依赖。

pub mod backup;
pub mod envelope;

pub use backup::{BackupFormat, decrypt_document, encrypt_document};
pub use envelope::{is_envelope, open, seal};
