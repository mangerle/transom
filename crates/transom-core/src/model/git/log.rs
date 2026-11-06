//! Git 历史记录与引用模型。

use std::path::PathBuf;

use super::status::ChangeCode;

/// 一条提交记录。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommitEntry {
    /// 40 位完整提交号。
    pub oid: String,
    /// 父提交号列表；合并提交有多个。
    pub parents: Vec<String>,
    pub author_name: String,
    pub author_email: String,
    /// 作者时间（Unix 秒）。
    pub author_time: i64,
    /// 标题行。
    pub subject: String,
    /// 正文（不含标题）。
    pub body: String,
    /// 挂在该提交上的引用装饰（分支/标签/HEAD）。
    pub refs: Vec<RefDecor>,
}

impl CommitEntry {
    /// 短提交号（前 7 位），用于界面展示。
    pub fn short_oid(&self) -> &str {
        // 提交号是十六进制 ASCII，按字节截断不会切断字符。
        &self.oid[..self.oid.len().min(7)]
    }

    /// 是否为合并提交（父提交多于一个）。
    pub fn is_merge(&self) -> bool {
        self.parents.len() > 1
    }
}

/// 引用装饰种类。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RefKind {
    /// `HEAD -> main`：当前分支指针。
    HeadAt,
    /// 本地分支。
    Branch,
    /// 远程跟踪分支（名字含 `/`）。
    RemoteBranch,
    /// 标签。
    Tag,
}

/// 一条引用装饰。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RefDecor {
    pub kind: RefKind,
    pub name: String,
}

/// 日志查询条件。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LogQuery {
    /// 起始修订；`None` 表示 HEAD。
    pub head: Option<String>,
    /// 范围：当前分支还是全部引用。
    pub scope: LogScope,
    /// 跳过条数（分页偏移）。
    pub skip: usize,
    /// 本页条数上限。
    pub limit: usize,
    /// 标题/正文全文过滤。
    pub search: Option<String>,
    /// 作者过滤（子串匹配）。
    pub author: Option<String>,
    /// 路径过滤（仓库相对路径）。
    pub paths: Vec<String>,
    /// 检索词按正则解释；默认假，按固定字符串匹配。
    ///
    /// 为什么默认关：界面那个 `.*` 按钮就是 IDEA 的正则开关，用户在没点开它时
    /// 输入 `a(b` 期望的是字面量，默认开正则会让他以为搜索坏了。
    pub regex: bool,
    /// 检索词区分大小写；默认假，与 IDEA 的 `Cc` 按钮一致。
    pub case_sensitive: bool,
    /// 起始日期（git 接受 `2 weeks ago` 这类自然语言）。
    pub since: Option<String>,
    /// 截止日期。
    pub until: Option<String>,
}

impl Default for LogQuery {
    fn default() -> Self {
        Self {
            head: None,
            scope: LogScope::CurrentBranch,
            skip: 0,
            limit: 200,
            search: None,
            author: None,
            paths: Vec::new(),
            regex: false,
            case_sensitive: false,
            since: None,
            until: None,
        }
    }
}

/// 日志范围。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LogScope {
    /// 仅当前分支可达的历史。
    CurrentBranch,
    /// 所有本地与远程分支、标签（`--all`）。
    AllRefs,
    /// 指定引用。
    Ref(String),
}

/// 一页日志结果。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LogPage {
    /// 按 git 输出顺序（拓扑序，父提交在后）。
    pub entries: Vec<CommitEntry>,
    /// 是否还有更多历史可加载。
    pub has_more: bool,
}

/// 提交内改动的单个文件。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommitFile {
    pub path: String,
    pub old_path: Option<String>,
    pub change: ChangeCode,
}

/// 分支条目。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BranchEntry {
    /// 短名（`main`、`origin/main`）。
    pub name: String,
    /// 是否为远程跟踪分支。
    pub remote: bool,
    /// 分支顶端提交号。
    pub oid: String,
    /// 是否为当前检出分支。
    pub current: bool,
    pub upstream: Option<String>,
    /// 相对 HEAD 的领先/落后提交数。
    pub ahead: u32,
    pub behind: u32,
}

/// 标签条目。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TagEntry {
    pub name: String,
    /// 标签对象号（轻量标签等于目标提交号）。
    pub oid: String,
    /// 解引用后的目标提交号。
    pub target: String,
    /// 附注标签的说明。
    pub message: Option<String>,
}

/// 一次探测到的仓库位置信息。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RepoInfo {
    /// 工作区根目录（规范化绝对路径）。
    pub toplevel: PathBuf,
    /// `.git` 目录或文件（链接工作树指向主仓库的 `gitdir:`）。
    pub git_dir: PathBuf,
    /// 是否为链接工作树（`git worktree add` 创建）。
    pub linked_worktree: bool,
}
