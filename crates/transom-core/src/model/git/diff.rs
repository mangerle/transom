//! Git 差异对比模型。

/// 单个文件的两侧文本内容，供界面做分栏差异。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiffContent {
    pub path: String,
    /// 旧侧文本；新增文件时为空串。
    pub old_text: String,
    /// 新侧文本；删除文件时为空串。
    pub new_text: String,
    /// 任一侧为二进制内容：界面只显示状态，不做文本比较。
    pub binary: bool,
    /// 因超过单侧字节上限而被截断。
    pub truncated: bool,
}

/// 差异比较的一侧来源。
///
/// 与 `transom-git` 的读取实现解耦：命令里带的是这个契约，引擎 crate 负责
/// 把它翻译成 `git show <rev>:<path>` 或工作区文件读取。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GitSide {
    /// 工作区文件本体。
    Worktree,
    /// 索引（暂存区）。
    Index,
    /// 指定修订（提交号或引用名）。
    Commit(String),
}

/// 把分支头提交压进父提交的方式（IDEA 的「压缩到／优选到」）。
///
/// 设计原理（Why）：两种折叠只差在说明文本上——压缩把两条说明串起来交给人
/// 继续编辑，优选直接丢掉尾部提交的说明（这正是 fixup 的用途：修个错别字或
/// 补一次漏改，不该在历史里留下「修复上一步」这种噪音）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GitFoldMode {
    /// 保留两条提交说明并串接。
    Squash,
    /// 只保留父提交的说明。
    Fixup,
}
