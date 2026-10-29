//! SFTP 远程递归文件与内容检索数据结构契约定义。
//!
//! 设计原理（Why）：
//! - 实现初衷：为远端主机提供文件模糊匹配（`find`）与全文文本检索（`grep`）的强类型结构表达；
//! - 核心优势：清晰区分文件名搜索与内容搜索两种维度，结构体自带分页截断与深度限制，防止无上限检索拖垮远端与客户端内存；
//! - 代价与局限：仅定义参数契约与结果形态，具体 Shell 命令转义与解析由网络控制器负责。

use serde::{Deserialize, Serialize};

/// 远程检索请求参数对象。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RemoteSearchQuery {
    /// 搜索起始根路径
    pub path: String,
    /// 搜索匹配表达式或文本模式
    pub pattern: String,
    /// 是否为文本内容检索（true 为内容 grep，false 为文件名 find）
    pub is_content: bool,
    /// 是否区分大小写
    pub case_sensitive: bool,
    /// 递归最大深度（None 表示不限制）
    pub max_depth: Option<u32>,
    /// 最大返回条目上限
    pub max_results: usize,
}

impl Default for RemoteSearchQuery {
    fn default() -> Self {
        Self {
            path: "/".to_string(),
            pattern: String::new(),
            is_content: false,
            case_sensitive: false,
            max_depth: Some(5),
            max_results: 100,
        }
    }
}

/// 单条检索结果项。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RemoteSearchResultItem {
    /// 匹配到的远程绝对路径
    pub path: String,
    /// 匹配到的文本所在行号（文件名检索时为 None）
    pub line_number: Option<usize>,
    /// 匹配到的行文本摘要内容（文件名检索时为 None）
    pub line_content: Option<String>,
}

impl RemoteSearchResultItem {
    /// 构造纯路径匹配结果。
    pub fn new_path(path: impl Into<String>) -> Self {
        Self {
            path: path.into(),
            line_number: None,
            line_content: None,
        }
    }

    /// 构造带行号与内容的全文匹配结果。
    pub fn new_content(
        path: impl Into<String>,
        line_number: usize,
        line_content: impl Into<String>,
    ) -> Self {
        Self {
            path: path.into(),
            line_number: Some(line_number),
            line_content: Some(line_content.into()),
        }
    }
}
