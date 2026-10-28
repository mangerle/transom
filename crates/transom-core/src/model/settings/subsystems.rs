//! 语义块、日志等级、历史命令、审计日志、快捷指令片段、SFTP 与快捷键子系统配置。

use crate::ids::SnippetId;
use crate::protocol::sftp::{ChecksumAlgorithm, VerifyMode};
use crate::protocol::sftp_filter::DEFAULT_TRANSFER_EXCLUDES;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

fn default_true() -> bool {
    true
}

/// 语义块外观样式。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum BlockViewStyle {
    /// 现代卡片：块之间留呼吸间距、柔和背景。
    #[default]
    Card,
    /// 极简边缘线：不额外占行，只在行号边缘画状态线。
    Minimal,
}

/// OSC 133 语义块视图偏好。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct BlockViewSettings {
    /// 是否启用语义块视图；关闭时回落到经典连续流。
    #[serde(default = "default_true")]
    pub enabled: bool,
    /// 块外观样式。
    #[serde(default)]
    pub style: BlockViewStyle,
    /// 输出超过该行数时自动折叠；`0` 表示从不自动折叠。
    #[serde(default = "default_block_collapse_rows")]
    pub auto_collapse_rows: usize,
}

impl Default for BlockViewSettings {
    fn default() -> Self {
        Self {
            enabled: true,
            style: BlockViewStyle::Card,
            auto_collapse_rows: default_block_collapse_rows(),
        }
    }
}

impl BlockViewSettings {
    /// 叠加会话级覆盖："强制开启/关闭"只改开关，样式与阈值仍取全局。
    pub fn resolved(self, override_: crate::model::session::BlockViewOverride) -> Self {
        let mut settings = self;
        match override_ {
            crate::model::session::BlockViewOverride::FollowGlobal => {}
            crate::model::session::BlockViewOverride::Enabled => settings.enabled = true,
            crate::model::session::BlockViewOverride::Disabled => settings.enabled = false,
        }
        settings
    }
}

/// 自动折叠阈值的默认值：构建日志一两百行就该收起，普通命令输出不受影响。
fn default_block_collapse_rows() -> usize {
    150
}

/// 运行日志设置。
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct LogSettings {
    /// 全局记录级别。改完立即生效：`log::set_max_level` 本身就是运行期可写的原子量。
    #[serde(default)]
    pub level: LogLevel,
}

/// 日志级别档位。
///
/// 为什么用枚举而不是裸字符串：级别要同时参与设置持久化、界面下拉与到
/// `log::LevelFilter` 的映射，散着写字符串必然出现"用户调了级别但没生效"这类
/// 最难查的失配。
///
/// 为什么默认 `Info`：这是给日常使用的第一档——生命周期节点与告警都在，量又不大。
/// `Debug` 留给排障（含上下文状态变迁），`Trace` 是协议与字节级数据流，
/// 生产环境常开会把磁盘打满。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum LogLevel {
    Error,
    Warn,
    /// 默认档：`#[serde(default)]` 与 `Default` 都指向它。
    #[default]
    Info,
    Debug,
    Trace,
}

impl LogLevel {
    /// 全部档位（由严到宽），供设置页下拉与遍历使用。
    pub const ALL: [Self; 5] = [
        Self::Error,
        Self::Warn,
        Self::Info,
        Self::Debug,
        Self::Trace,
    ];

    /// 稳定小写名：同时是设置文件的取值与 i18n key 的后缀（`settings.log_level.{}`）。
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Error => "error",
            Self::Warn => "warn",
            Self::Info => "info",
            Self::Debug => "debug",
            Self::Trace => "trace",
        }
    }

    /// 读取环境变量里的级别覆盖（`TRANSOM_LOG`，兼容更熟悉的 `RUST_LOG`）。
    ///
    /// 为什么留这条通道：用户报障最常见的情形是"应用一起来就来不及改设置"，
    /// 一条环境变量就能把现场捞下来。
    ///
    /// 为什么忽略 `目标=级别` 这类写法：那是过滤器而不是整体级别，硬当级别解析会
    /// 悄悄退成 `info`，用户以为调到了 debug 其实没有；返回 `None` 让设置值继续
    /// 生效更诚实。
    pub fn from_env() -> Option<Self> {
        ["TRANSOM_LOG", "RUST_LOG"]
            .iter()
            .find_map(|key| std::env::var(key).ok())
            .map(|raw| raw.trim().to_string())
            .filter(|raw| !raw.is_empty() && !raw.contains('='))
            .map(|raw| Self::parse(&raw))
    }

    /// 映射到 log 门面的过滤级别。
    pub fn to_filter(self) -> log::LevelFilter {
        match self {
            Self::Error => log::LevelFilter::Error,
            Self::Warn => log::LevelFilter::Warn,
            Self::Info => log::LevelFilter::Info,
            Self::Debug => log::LevelFilter::Debug,
            Self::Trace => log::LevelFilter::Trace,
        }
    }

    /// 解析设置里的级别。
    ///
    /// 设置文件属于外部输入：认不出来的值一律退回默认档 `Info`，绝不允许因为
    /// 一个手写的错值把整个日志关掉（那样反而没有日志可排查）。
    pub fn parse(value: &str) -> Self {
        match value.trim().to_ascii_lowercase().as_str() {
            "error" => Self::Error,
            "warn" | "warning" => Self::Warn,
            "debug" => Self::Debug,
            "trace" | "verbose" => Self::Trace,
            _ => Self::Info,
        }
    }
}

/// 反序列化走 [`LogLevel::parse`]：未知值降级为默认档，而不是让整个设置文件报错。
impl<'de> serde::Deserialize<'de> for LogLevel {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let raw = <String as serde::Deserialize>::deserialize(deserializer)?;
        Ok(Self::parse(&raw))
    }
}

/// 命令历史与建议设置。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HistorySettings {
    /// 总开关：关闭后不再采集命令，也不再弹建议。
    ///
    /// 为什么默认开启：采集到的只是"这一行敲过什么"，与 shell 自己在
    /// `~/.bash_history` 里留的内容同性质，且不落远端；建议功能只有在
    /// 有历史时才出现，关掉它反而让多数人以为功能坏了。
    #[serde(default = "default_true")]
    pub enabled: bool,
}

impl Default for HistorySettings {
    fn default() -> Self {
        Self { enabled: true }
    }
}

/// 会话输出审计落盘设置。
///
/// 为什么默认关闭：审计文件会把远端输出的全部内容（含可能出现在屏幕上的
/// 口令、令牌与业务数据）落到本地磁盘。这是合规诉求，但不是每个人都愿意
/// 在不知情时被记录，所以必须由用户显式开启。
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct AuditSettings {
    /// 总开关；关闭时不创建文件，也不产生任何额外 IO。
    #[serde(default)]
    pub enabled: bool,
}

/// 快捷命令条上的一键命令。
///
/// 为什么放在全局设置而不是会话里：这条特性的动机是"同一条长命令在几十台机器上反复敲"，
/// 命令本身与具体会话无关；会话级差异由终端里已有的 cd/别名承担。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Snippet {
    pub id: SnippetId,
    /// 按钮上显示的名字。
    pub name: String,
    /// 实际写入终端的文本。
    pub command: String,
}

/// 名字长度上限：按钮要能并排放进底栏。
const SNIPPET_NAME_MAX_CHARS: usize = 40;
/// 命令长度上限，避免把整段脚本塞进一个按钮后无法点击。
const SNIPPET_COMMAND_MAX_CHARS: usize = 4000;

impl Snippet {
    /// 命令条目的最小合法性：名字与命令都不能为空，长度要有界。
    ///
    /// 校验放在 core 而不是界面层，设置文件属外部输入（可手工编辑、可被导入），
    /// 任何一条不合格就整条丢弃，而不是让底栏画出个空白按钮。
    pub fn is_valid(&self) -> bool {
        let name = self.name.trim();
        let command = self.command.trim();
        !name.is_empty()
            && !command.is_empty()
            && name.chars().count() <= SNIPPET_NAME_MAX_CHARS
            && command.chars().count() <= SNIPPET_COMMAND_MAX_CHARS
    }
}

/// 快捷命令库。命令内容与“直接执行”开关跨重启保持，面板显隐属运行期不在此列。
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SnippetSettings {
    #[serde(default)]
    pub items: Vec<Snippet>,
    /// 点击命令后是否直接发送并回车。
    ///
    /// 默认关闭：终端里误执行一条危险命令的代价远高于多点一次回车，
    /// 因此开启后由界面明确提示当前处于“点击即执行”状态。
    #[serde(default)]
    pub execute_directly: bool,
}

impl SnippetSettings {
    /// 丢弃不合法条目并按名字去重（同名以后出现的那条为准）。
    pub fn sanitized(mut self) -> Self {
        self.items.retain(Snippet::is_valid);
        let mut deduped: Vec<Snippet> = Vec::with_capacity(self.items.len());
        for item in self.items {
            if let Some(position) = deduped
                .iter()
                .position(|existing| existing.name.trim() == item.name.trim())
            {
                deduped[position] = item;
            } else {
                deduped.push(item);
            }
        }
        self.items = deduped;
        self
    }
}

/// 用户覆盖的快捷键配置。
///
/// 仅保存被用户修改的动作；未出现的动作在运行时使用内置默认值。
/// 每个动作可配置多个备选键位，空列表或全部非法时回退到默认键位，
/// 避免手工编辑配置后出现“完全无法操作”的状态。
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ShortcutSettings {
    #[serde(default)]
    pub bindings: BTreeMap<String, Vec<String>>,
}

/// SFTP 传输偏好：排除规则位于设置中，跨会话与重启保持生效。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SftpSettings {
    #[serde(default = "default_transfer_filter")]
    pub transfer_filter: String,
    /// 传输完成后是否计算并比对两端 SHA256；默认关闭以避免大文件额外读盘。
    #[serde(default)]
    pub verify_after_transfer: bool,
    /// 传输完成后的校验模式，默认为完整校验。
    #[serde(default)]
    pub verify_mode: VerifyMode,
    /// 校验所使用的摘要算法，默认 SHA-256。
    #[serde(default)]
    pub checksum_algorithm: ChecksumAlgorithm,
    /// 同时执行的传输任务数上限，默认 3。
    #[serde(default = "default_max_concurrent_transfers")]
    pub max_concurrent_transfers: usize,
    /// 内置编辑器是否软换行：长行折行显示而不出横向滚动条。
    #[serde(default = "default_true")]
    pub editor_soft_wrap: bool,
    /// 内置编辑器是否显示行号。
    #[serde(default = "default_true")]
    pub editor_line_numbers: bool,
    /// 文件面板的显示排除规则（glob 文本，逐条匹配文件名）。
    ///
    /// 与传输过滤共用语法但语义不同：这里只影响面板可见性，不拦截传输。
    #[serde(default)]
    pub panel_hidden_patterns: String,
    /// 内置编辑器可打开的单文件字节上限，默认 8 MiB。
    ///
    /// 设计原理（Why）：编辑远程文本走「全量下载到临时文件 → 读入缓冲区 →
    /// 全量回传覆盖」，文件大小直接决定下载耗时、Rope 常驻内存与高亮重绘开销；
    /// 双击一个几百 MB 的日志会把整份内容拉进界面，因此在打开前按目录列表已知
    /// 的尺寸拦截，并提示改用远端编辑器。超限属于误操作而非故障，故不进设置页，
    /// 由 config.toml 单点承载，需要放宽时改一处即可。
    #[serde(default = "default_edit_max_bytes")]
    pub edit_max_bytes: u64,
    /// 内置编辑器回传覆盖前，是否先把远端当前内容备份为 `{路径}.transom-bak`。
    ///
    /// 设计原理（Why）：回传是"整文件覆盖"而非原子替换，写入中断会在远端留下
    /// 半份文件；备份是 SFTP 协议下零 inode 副作用的最低成本兜底（rename 原子
    /// 替换会改变 inode，可能破坏正在读取该文件的进程，故不采用）。默认关闭：
    /// 备份会在远端留下额外文件，是否可接受属于部署方策略，交给 config.toml
    /// 单点承载（与 edit_max_bytes 同理，不进设置页）。
    #[serde(default)]
    pub editor_write_backup: bool,
}

impl Default for SftpSettings {
    fn default() -> Self {
        Self {
            transfer_filter: default_transfer_filter(),
            verify_after_transfer: false,
            verify_mode: VerifyMode::default(),
            checksum_algorithm: ChecksumAlgorithm::default(),
            max_concurrent_transfers: default_max_concurrent_transfers(),
            edit_max_bytes: default_edit_max_bytes(),
            editor_write_backup: false,
            editor_soft_wrap: true,
            editor_line_numbers: true,
            panel_hidden_patterns: String::new(),
        }
    }
}

fn default_max_concurrent_transfers() -> usize {
    3
}

fn default_edit_max_bytes() -> u64 {
    8 * 1024 * 1024
}

fn default_transfer_filter() -> String {
    DEFAULT_TRANSFER_EXCLUDES.to_string()
}
