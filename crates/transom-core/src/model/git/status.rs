//! Git 仓库状态模型。

/// 仓库当前头指针形态。
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum HeadState {
    /// 位于具名分支上。
    OnBranch(String),
    /// 游离头指针（detached），具体提交见 [`RepoStatus::oid`]。
    ///
    /// 也是默认值：状态采集失败或字段缺失时宁可显示「游离」，不伪造一个分支名。
    #[default]
    Detached,
    /// 尚无任何提交（新初始化仓库），HEAD 指向一个还不存在的分支。
    Initial {
        /// 计划中的分支名，来自 `# branch.head`。
        branch: String,
    },
}

/// 一次 `git status --porcelain=v2` 的完整快照。
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct RepoStatus {
    /// 头指针形态。
    pub head: HeadState,
    /// HEAD 指向的提交号；空仓库为 `None`。
    pub oid: Option<String>,
    /// 当前分支的上游引用短名（如 `origin/main`）。
    pub upstream: Option<String>,
    /// 领先上游的提交数；无上游时为 0。
    pub ahead: u32,
    /// 落后上游的提交数；无上游时为 0。
    pub behind: u32,
    /// 全部变更条目，顺序与 git 输出一致（按路径字典序）。
    pub entries: Vec<StatusEntry>,
}

impl RepoStatus {
    /// 已加入索引、等待提交的条目。
    pub fn staged(&self) -> Vec<&StatusEntry> {
        self.entries.iter().filter(|entry| entry.staged()).collect()
    }

    /// 工作区相对索引有改动、尚未暂存的条目。
    pub fn unstaged(&self) -> Vec<&StatusEntry> {
        self.entries
            .iter()
            .filter(|entry| entry.unstaged())
            .collect()
    }

    /// 未跟踪文件。
    pub fn untracked(&self) -> Vec<&StatusEntry> {
        self.entries
            .iter()
            .filter(|entry| matches!(entry.record, StatusRecord::Untracked))
            .collect()
    }

    /// 合并冲突条目。
    pub fn conflicts(&self) -> Vec<&StatusEntry> {
        self.entries
            .iter()
            .filter(|entry| matches!(entry.record, StatusRecord::Conflict))
            .collect()
    }

    /// 是否存在需要人工处理的冲突。
    pub fn has_conflicts(&self) -> bool {
        self.entries
            .iter()
            .any(|entry| matches!(entry.record, StatusRecord::Conflict))
    }
}

/// 单个文件的变更条目。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StatusEntry {
    /// 仓库根相对路径，`/` 分隔。
    ///
    /// git 以原始字节回传路径，这里按 UTF-8 宽松解码：非 UTF-8 文件名会带
    /// 替换字符，对其发起的后续 git 调用会失败并如实报错，不会误伤其他文件。
    pub path: String,
    /// 重命名/复制的来源路径。
    pub old_path: Option<String>,
    /// 该条目的形态。
    pub record: StatusRecord,
}

impl StatusEntry {
    /// 索引侧是否有待提交内容（冲突文件也视为待处理）。
    pub fn staged(&self) -> bool {
        match &self.record {
            StatusRecord::Tracked { index, .. } => *index != ChangeCode::Unmodified,
            StatusRecord::Conflict => true,
            _ => false,
        }
    }

    /// 工作区侧是否有未暂存改动。
    pub fn unstaged(&self) -> bool {
        matches!(
            &self.record,
            StatusRecord::Tracked { worktree, .. } if *worktree != ChangeCode::Unmodified
        )
    }
}

/// 条目形态：两位状态码，或三种特殊类别。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StatusRecord {
    /// 已被跟踪的文件，分别记录索引侧与工作区侧状态。
    Tracked {
        index: ChangeCode,
        worktree: ChangeCode,
    },
    /// 三方合并未解决。
    Conflict,
    /// 未跟踪。
    Untracked,
    /// 被忽略（仅在请求包含忽略项时出现）。
    Ignored,
}

/// porcelain 状态码字符。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChangeCode {
    Added,
    Modified,
    Deleted,
    Renamed,
    Copied,
    /// 类型变更（普通文件 ↔ 符号链接 ↔ gitlink）。
    TypeChange,
    /// 未合并（仅出现在冲突记录里）。
    Unmerged,
    /// 该侧无变化，即 `.`。
    Unmodified,
    /// 未识别的状态码：保留原文而不是丢弃，避免新版本 git 扩展字符时静默漏项。
    Unknown(char),
}

impl ChangeCode {
    /// 解析单个状态码字符。
    pub fn from_char(value: char) -> Self {
        match value {
            'A' => Self::Added,
            'M' => Self::Modified,
            'D' => Self::Deleted,
            'R' => Self::Renamed,
            'C' => Self::Copied,
            'T' => Self::TypeChange,
            'U' => Self::Unmerged,
            '.' => Self::Unmodified,
            other => Self::Unknown(other),
        }
    }
}
