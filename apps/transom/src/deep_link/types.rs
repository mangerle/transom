//! 启动意图解析与协议行序列化。

use std::path::{Path, PathBuf};
use transom_core::is_ssh_url;

/// 转发协议里目录请求的行前缀：路径可以合法地以 `/` 开头，必须先于
/// `ssh://` 判定剥离，避免把本地路径误当协议地址。
pub(crate) const DIR_LINE_PREFIX: &str = "dir:";

/// 转发协议里命令执行请求的行前缀，载荷是 JSON。
///
/// 为什么命令与目录合成一行而不是各自一行：主实例是按行依次处理的，拆成
/// 两行会先为 `dir:` 开一个交互式 Shell 标签，再为命令开第二个标签——用户
/// 点一次"在终端打开"却得到两个标签。
pub(crate) const EXEC_LINE_PREFIX: &str = "exec:";

/// 一次启动参数解析出的唤起意图。
///
/// 设计原理（Why）：`ssh://` 地址与目录参数互不冲突（`transom ssh://a ssh://b`
/// 合法，开多个标签），但目录只取第一个——第二个来源不明的目录该忽略而不是
/// 静默换靶。非法状态用枚举表达见 `DeepLinkRequest`，这里只是解析中间态。
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct LaunchRequest {
    /// 启动参数中出现的 `ssh://` 地址，按出现顺序。
    pub urls: Vec<String>,
    /// `--dir=` / `--dir X` / 裸绝对路径参数给出的工作目录（取第一个）。
    pub dir: Option<PathBuf>,
    /// `-e <命令...>` / `-- <命令...>` 给出的执行请求（取第一个）。
    ///
    /// 语义来自 `X-TerminalArgExec`：launcher 把要跑的命令按 argv 原样追加，
    /// 这里整段收下，不重新分词也不拼 Shell 字符串。
    pub command: Option<Vec<String>>,
}

/// 从命令行参数解析唤起意图（跳过程序名）。
///
/// 目录参数接受三种形态：`--dir=/path`（桌面条目 `X-TerminalArgDir` 让
/// launcher 采用的形式）、`--dir /path` 与裸位置参数（第三方工具直接传目录）；
/// 位置参数只在"绝对路径且当前存在为目录"时采信——协议上除
/// `ssh://` 外的参数本就忽略，这里再加存在性校验，杜绝把陈旧路径带进转发。
/// launcher 会先剥掉条目 Exec 里的 `%u` 再拼参数，因此目录参数不会与地址
/// 形态混淆。其余未知参数继续忽略（启动器附带杂参是常态）。
///
/// `-e` 与 `--` 是命令分隔符：其后所有参数都属于命令，逐字节保留（含空格
/// 与以 `-` 开头的参数），不再参与本函数的任何选项解析。
pub fn request_from_args<I: IntoIterator<Item = String>>(args: I) -> LaunchRequest {
    let mut request = LaunchRequest::default();
    let mut argv = args.into_iter().skip(1).peekable();
    while let Some(arg) = argv.next() {
        let arg = arg.trim().to_string();
        if let Some(rest) = arg.strip_prefix("--dir=") {
            request.set_dir(rest);
            continue;
        }
        if arg == "--dir" {
            if let Some(value) = argv.next() {
                request.set_dir(value.trim());
            } else {
                log::warn!("启动参数 --dir 后面缺少目录值，已忽略");
            }
            continue;
        }
        // 命令分隔符：`-e` 是 X-TerminalArgExec 的通行写法，`--` 是
        // xdg-terminal-exec 直接透传时的分隔符。两者之后的内容原样收下。
        if arg == "-e" || arg == "--" {
            let command: Vec<String> = argv.by_ref().collect();
            if command.is_empty() {
                log::warn!("启动参数 {arg} 后面缺少命令，已忽略");
            } else if request.command.is_some() {
                log::warn!("启动参数中出现第二个命令请求，已忽略: {command:?}");
            } else {
                request.command = Some(command);
            }
            break;
        }
        if is_ssh_url(&arg) {
            request.urls.push(arg);
            continue;
        }
        // 裸位置参数只在形如绝对路径时才可能是目录请求。
        if arg.starts_with('/') {
            request.set_dir(&arg);
        }
    }
    request
}

impl LaunchRequest {
    /// 记录第一个可信目录；后续目录参数忽略并留痕（歧义不猜）。
    fn set_dir(&mut self, raw: &str) {
        if self.dir.is_some() {
            log::warn!("启动参数中出现第二个目录请求，已忽略: {raw}");
            return;
        }
        let candidate = Path::new(raw);
        if candidate.is_absolute() && candidate.is_dir() {
            self.dir = Some(candidate.to_path_buf());
        } else {
            // 只采信现存绝对路径：相对形态既无从判定基准目录，也不符合
            // launcher 的传参约定，按忽略处理。
            log::debug!("目录参数不是现存绝对目录，已忽略: {raw}");
        }
    }
}

/// 单条转发请求的文本行：ssh 地址原样，目录加 `dir:` 前缀，命令转 JSON。
pub(crate) fn request_lines(request: &LaunchRequest) -> Vec<String> {
    let mut lines = request.urls.clone();
    if let Some(command) = &request.command {
        // 命令与目录合并为一行 JSON：见 `EXEC_LINE_PREFIX` 的说明。
        let payload = serde_json::json!({
            "command": command,
            "dir": request
                .dir
                .as_ref()
                .map(|dir| dir.to_string_lossy().into_owned()),
        });
        lines.push(format!("{EXEC_LINE_PREFIX}{payload}"));
    } else if let Some(dir) = &request.dir {
        lines.push(format!("{DIR_LINE_PREFIX}{}", dir.display()));
    }
    lines
}
