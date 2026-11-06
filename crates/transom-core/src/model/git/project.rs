//! Git 项目与分组模型。

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

use crate::ids::{GitFolderId, GitProjectId};
use crate::model::session_appearance::{SessionColor, SessionIcon};

/// 一个被纳入管理的 Git 项目（宿主机上的仓库工作区）。
///
/// 只存路径、展示名、分组与外观标记：分支、脏文件数等都是运行期从仓库读出来的
/// 快照，落盘只会过期，不会更快。外观沿用会话那套枚举（[`SessionIcon`] /
/// [`SessionColor`]）而不是再造一套：两个面板是同一个应用里的同类清单，
/// 「生产/测试」这类环境标记的语义与色板只该有一份定义，各自调色迟早分裂。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct GitProject {
    pub id: GitProjectId,
    /// 工作区根目录（绝对路径，添加时已规范化）。
    pub path: PathBuf,
    /// 用户自定义显示名；为空时界面取目录名。
    #[serde(default)]
    pub label: Option<String>,
    /// 所属分组；`None` 表示直接挂在列表根层。
    ///
    /// 只有单层：分组下只能是项目，不能再套子分组，因此不需要 `parent` 字段，
    /// 也就不需要会话目录那套父子环检测。
    #[serde(default)]
    pub folder_id: Option<GitFolderId>,
    /// 专属图标；`Auto` 时界面按仓库类型取默认图标。
    #[serde(default)]
    pub icon: SessionIcon,
    /// 环境色；`None` 时行首图标保持语义色（有改动=警示、读不到=红）。
    #[serde(default)]
    pub color: SessionColor,
    /// 加入时间（Unix 秒）：只作记录与展示，不参与排序。
    ///
    /// 列表顺序由用户在侧栏上下拖拽排定并原样落盘，任何按时间的默认排序
    /// 都会把用户排好的顺序静默覆盖掉。
    #[serde(default)]
    pub added_at: i64,
}

impl GitProject {
    /// 以当前时间新建一条项目记录。
    pub fn new(path: PathBuf) -> Self {
        // 系统时钟回拨时取 0：加入时间只用于记录，宁可排在最前也不 panic。
        let added_at = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |elapsed| elapsed.as_secs() as i64);
        Self {
            id: GitProjectId::new(),
            path,
            label: None,
            folder_id: None,
            icon: SessionIcon::default(),
            color: SessionColor::default(),
            added_at,
        }
    }

    /// 界面展示名：优先用户别名，其次目录名。
    pub fn display_name(&self) -> String {
        if let Some(label) = self.label.as_deref().filter(|text| !text.trim().is_empty()) {
            return label.to_string();
        }
        self.path
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            // 根目录没有 file_name，退化成完整路径而不是空串。
            .unwrap_or_else(|| self.path.display().to_string())
    }
}

/// Git 项目分组（侧栏的一行组标题）。
///
/// 设计原理（Why）：单层、无父级，`collapsed` 与项目顺序一起落盘——分组的作用
/// 就是把十几个仓库收成几行，折叠态不记住就等于每次重启都要重新收一遍。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct GitFolder {
    pub id: GitFolderId,
    /// 分组名。
    pub name: String,
    /// 是否折叠（折叠后组内项目不在侧栏显示，但仍在列表里、仍可被拖入）。
    #[serde(default)]
    pub collapsed: bool,
}

impl GitFolder {
    /// 新建一个默认展开的分组。
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            id: GitFolderId::new(),
            name: name.into(),
            collapsed: false,
        }
    }
}

/// Git 侧栏的完整登记表：项目 + 分组。
///
/// 设计原理（Why）：两者必须一起落盘——分开保存会出现「分组已写、项目里的
/// `folder_id` 还指着旧表」的撕裂状态（会话目录 `SessionStore` 同一个理由）。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct GitRegistry {
    pub projects: Vec<GitProject>,
    pub folders: Vec<GitFolder>,
}
