//! 日志落地：log 门面 + fern 分发 + 自写按大小轮转的文件层。
//!
//! 设计原理（Why）：
//! - 为什么是 `log` 门面而不是继续用 tracing：gpui、russh 这些上游依赖本身就调
//!   `log`，接上门面后它们的内部日志与我们的埋点落进同一份文件、同一套级别，
//!   排障时才对得上时间线；工程里没有任何 span/`instrument` 用法，留着 tracing
//!   只是多养一层依赖而拿不到收益。
//! - 为什么自写轮转而不是换 log4rs/flexi_logger：需求只有"到大小切开、留 N 份"，
//!   一个几十行的 `Write` 实现就说完了；套完整框架等于把保留策略、文件命名与
//!   清理语义交出去，而设置页的"清理历史日志"必须与命名规则逐字一致，否则删不掉
//!   还以为删了。命名规则因此收进 `transom_core::paths`，两侧共用一个来源。
//! - 运行期可调：装配时把派发级别开到 `Trace`，真正的门槛交给
//!   `log::set_max_level`——它是运行期可写的原子量，设置页改完立刻生效，不必重启。
//!   stderr 链路单独压在 `Warn`：从终端拉起时把 `debug` 全量刷到屏幕上会淹掉交互，
//!   要看细节去读文件。
//! - 代价与局限：写入是同步的（fern 每条记录持锁写并 flush，落盘一次），所以热点
//!   循环只能打 `debug`/`trace`，绝不允许在逐字节路径上刷 `info`；文件层一旦出错
//!   即永久停用并退回 stderr——日志坏了不能拖垮主流程，也不做"下次再试试"，
//!   因为磁盘满与权限被改通常不会自己好，反复尝试只会把 stderr 刷满。

use std::fs::{self, File};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use transom_core::{LogLevel, paths, private_fs};

/// 单个日志文件的大小上限，到界即轮转。
const MAX_FILE_BYTES: u64 = 5 * 1024 * 1024;

/// 轮转保留的历史份数（`transom.log.1` 最新，编号越大越旧）。
///
/// 与 [`MAX_FILE_BYTES`] 一起决定总占用上限（约 25 MiB）：桌面应用的配置目录不该
/// 被日志吃掉几百兆，但只留一两份又不够回看一次完整的复现过程。
const KEEP_BACKUPS: usize = 4;

/// stderr 侧的最低门槛。
const STDERR_LEVEL: log::LevelFilter = log::LevelFilter::Warn;

/// 初始化全局日志。
///
/// 唯一的失败情形是进程里已经装过 logger（重复初始化）；调用方按"日志没接上"
/// 继续启动即可，不该因为日志把应用关在外面。
///
/// # Errors
/// `log::SetLoggerError`：全局 logger 已被设置。
pub(crate) fn init(level: LogLevel) -> Result<(), log::SetLoggerError> {
    let dispatch = fern::Dispatch::new()
        .format(|out, message, record| {
            out.finish(format_args!(
                "{}",
                format_line(record.level(), record.target(), message)
            ))
        })
        // 派发侧开到最宽：真正的门槛是运行期可改的 `log::set_max_level`。
        .level(log::LevelFilter::Trace)
        .chain(
            fern::Dispatch::new()
                .level(STDERR_LEVEL)
                .chain(std::io::stderr()),
        )
        .chain(file_chain(&paths::log_file_path()));

    log::set_boxed_logger(Box::new(dispatch.into_log().1))?;
    log::set_max_level(level.to_filter());
    // 崩溃兜底紧随日志装配：此后任何 panic 都要在日志里留下位置与原因。
    install_panic_hook();
    log::info!(
        "日志系统就绪，级别: {}，文件: {}",
        level.as_str(),
        paths::log_file_path().display()
    );
    Ok(())
}

/// 安装 panic 兜底钩子：崩溃现场先落日志，再交给默认钩子。
///
/// 设计原理（Why）：
/// - 桌面双击启动时 stderr 不可见，GPUI 主线程 panic 会让窗口"凭空消失"，
///   日志文件是唯一能留下现场的地方，必须在钩子里转一手；
/// - 先记日志再调用默认钩子：保留 rustc 默认的 stderr 输出与
///   `RUST_BACKTRACE` 行为，终端场景的排查体验不变；
/// - 代价与局限：若 panic 恰好发生在日志层持锁期间（例如格式化回调内部
///   崩溃），这条记录可能无法落盘；默认钩子仍会把信息打到 stderr，
///   信息不会彻底丢失。
fn install_panic_hook() {
    let default_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let location = info.location().map_or_else(
            || "未知位置".to_string(),
            |location| {
                format!(
                    "{}:{}:{}",
                    location.file(),
                    location.line(),
                    location.column()
                )
            },
        );
        let payload = info
            .payload()
            .downcast_ref::<&str>()
            .map(|text| (*text).to_string())
            .or_else(|| info.payload().downcast_ref::<String>().cloned())
            .unwrap_or_else(|| "非文本负载".to_string());
        log::error!("线程发生 panic，位置: {location}，原因: {payload}");
        default_hook(info);
    }));
}

/// 解析启动级别：环境变量优先于设置项。
///
/// 为什么留环境变量：用户报障时最常见的情形是"应用一起来就来不及改设置"，
/// `TRANSOM_LOG=debug`（也认更熟悉的 `RUST_LOG`）能直接把现场捞下来。
pub(crate) fn resolve_level(settings_level: LogLevel) -> LogLevel {
    LogLevel::from_env().unwrap_or(settings_level)
}

/// 组装一行的文本：`时间 级别 目标: 消息`。
///
/// 抽成纯函数是为了让"日志长什么样"能被单测钉住：装配闭包要等 fern 回调才有输入，
/// 直接测闭包就得把整条链路立起来，代价与收益不成比例。
fn format_line(level: log::Level, target: &str, message: &dyn std::fmt::Display) -> String {
    format!(
        "{} {:<5} {}: {}",
        chrono::Local::now().format("%Y-%m-%d %H:%M:%S%.3f"),
        level,
        target,
        message
    )
}

/// 装配文件链路；建目录或打开文件失败时让这一路空转，stderr 仍然完整。
fn file_chain(path: &Path) -> fern::Dispatch {
    match RotatingLogFile::open(path, MAX_FILE_BYTES, KEEP_BACKUPS) {
        Ok(writer) => fern::Dispatch::new().chain(fern::Output::writer(Box::new(writer), "\n")),
        Err(error) => {
            // 此刻 logger 还没装上，只能用 eprintln 留一次痕。
            eprintln!(
                "打开日志文件失败，本次运行只输出到标准错误输出，路径: {}，原因: {error}",
                path.display()
            );
            // 级别开到 Off：空链路会被 fern 直接丢弃，等于只留 stderr。
            fern::Dispatch::new().level(log::LevelFilter::Off)
        }
    }
}

/// 按大小轮转的日志文件写入器。
///
/// 生命周期：文件句柄常驻到进程结束；轮转发生在写入路径上（先把旧文件滚成 `.1`
/// 再切新文件），因此任何时刻磁盘占用都不超过 `MAX_FILE_BYTES * (KEEP_BACKUPS + 1)`。
struct RotatingLogFile {
    /// 主日志文件路径；轮转时由它推出各份备份的名字。
    ///
    /// 路径存在结构体里而不是每次去问 `paths::log_file_path()`：写入器必须对
    /// "自己在写哪个文件"负责，否则单测会绕开临时目录去滚真实配置目录的日志。
    path: PathBuf,
    /// 当前文件；`None` 表示文件层已停用。
    file: Option<File>,
    /// 当前文件已写字节数，用于判断是否到轮转边界。
    written: u64,
    max_bytes: u64,
    keep: usize,
}

impl RotatingLogFile {
    /// 打开（或创建）日志文件；目录不存在时逐级创建。
    fn open(path: &Path, max_bytes: u64, keep: usize) -> io::Result<Self> {
        if let Some(dir) = path.parent() {
            private_fs::ensure_private_dir(dir)?;
        }
        let file = private_fs::open_private_append(path)?;
        // 已有长度要如实记着：上次运行留下的尾巴也算在这份轮转窗口里。
        let written = file.metadata().map(|meta| meta.len()).unwrap_or(0);
        Ok(Self {
            path: path.to_path_buf(),
            file: Some(file),
            written,
            max_bytes,
            keep,
        })
    }

    /// 第 `index` 份备份的路径。
    fn backup(&self, index: usize) -> PathBuf {
        paths::rotated_backup_path(&self.path, index)
    }

    /// 轮转：主文件变 `.1`，已有的依次后移，最旧一份删除。
    ///
    /// 先关句柄再改名：Windows 上重命名一个正被打开的文件会失败，Linux 下虽可行，
    /// 但两侧走同一套行为才不会变成"只在某个平台坏掉"的轮转。
    fn rotate(&mut self) {
        let _ = self.file.take();
        let _ = fs::remove_file(self.backup(self.keep));
        for index in (1..self.keep).rev() {
            let _ = fs::rename(self.backup(index), self.backup(index + 1));
        }
        if fs::rename(&self.path, self.backup(1)).is_err() {
            self.file = None;
            return;
        }
        self.file = private_fs::open_private_append(&self.path).ok();
        self.written = 0;
    }
}

impl Write for RotatingLogFile {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        if self.file.is_none() {
            // 文件层已停用：吞掉数据并报告成功，让调用链继续跑下去。
            return Ok(buf.len());
        }
        if self.written.saturating_add(buf.len() as u64) > self.max_bytes {
            self.rotate();
        }
        // 走到这里仍是 `None`，只可能是轮转后重开文件失败，同样按停用处理。
        let Some(file) = self.file.as_mut() else {
            return Ok(buf.len());
        };
        match file.write_all(buf) {
            Ok(()) => {
                self.written = self.written.saturating_add(buf.len() as u64);
                Ok(buf.len())
            }
            Err(error) => {
                // 一次性停用并交出错误：fern 会把它回显到 stderr 一次，此后静默丢弃。
                self.file = None;
                Err(error)
            }
        }
    }

    fn flush(&mut self) -> io::Result<()> {
        match self.file.as_mut() {
            Some(file) => file.flush(),
            None => Ok(()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{RotatingLogFile, file_chain, format_line, install_panic_hook};
    use log::Log as _;
    use std::io::Write;
    use std::path::{Path, PathBuf};

    /// panic 钩子可安装且不放大崩溃：catch_unwind 仍能捕获 panic。
    #[test]
    fn panic_hook_installs_and_panic_stays_catchable() {
        install_panic_hook();
        let result = std::panic::catch_unwind(|| panic!("模拟崩溃"));
        assert!(result.is_err(), "钩子不得吞掉 panic");
    }

    /// 行格式必须带时间、级别与目标：排障时三样缺一件就没法把线索串起来。
    #[test]
    fn format_line_prefixes_time_level_and_target() {
        let line = format_line(log::Level::Warn, "transom_net::client", &"握手超时");
        assert!(
            line.ends_with("WARN  transom_net::client: 握手超时"),
            "级别要左对齐补位、目标与消息之间用冒号，实际内容: {line}"
        );
        // 行首必须是 `YYYY-MM-DD HH:MM:SS.mmm`，只留时分秒就没法跨天对齐。
        let head = &line[..23];
        assert_eq!(head.as_bytes()[4], b'-');
        assert_eq!(head.as_bytes()[10], b' ');
        assert_eq!(head.as_bytes()[19], b'.');
        assert!(
            head.as_bytes()[0].is_ascii_digit(),
            "年从数字开始，实际: {head}"
        );
    }

    /// 走一遍真实派发：一条 info 记录必须落到轮转文件里。
    ///
    /// 这一步单独测，是因为"门面接上了但没有链路写文件"这类装配错误编译不出来，
    /// 只能靠把记录真投进 logger 才能发现。
    #[test]
    fn dispatch_writes_records_into_rotating_file() {
        let dir = temp_dir("dispatch");
        let path = dir.join("transom.log");
        let (_, logger) = fern::Dispatch::new()
            .format(|out, message, record| {
                out.finish(format_args!(
                    "{}",
                    format_line(record.level(), record.target(), message)
                ))
            })
            .level(log::LevelFilter::Trace)
            .chain(file_chain(&path))
            .into_log();

        logger.log(
            &log::Record::builder()
                .args(format_args!("拨测 10.0.0.8:22 可达"))
                .level(log::Level::Info)
                .target("transom_ui::probe")
                .build(),
        );
        logger.flush();

        let text = std::fs::read_to_string(&path).unwrap_or_default();
        assert!(
            text.contains("INFO  transom_ui::probe: 拨测 10.0.0.8:22 可达"),
            "记录应完整落盘，实际文件内容: {text}"
        );
        cleanup(&dir);
    }

    /// 轮转必须真的把旧内容挪进 `.1` 并继续往主文件写。
    #[test]
    fn rotates_when_exceeding_limit() {
        let dir = temp_dir("rotate");
        let mut writer =
            RotatingLogFile::open(&dir.join("transom.log"), 16, 2).expect("临时日志应可打开");
        writer
            .write_all(b"0123456789abcdef")
            .expect("首轮写入应成功");
        // 再来 8 字节就越过 16 的上限，触发轮转。
        writer.write_all(b"xxxxxxxx").expect("跨边界写入应成功");
        writer.flush().expect("flush 应成功");

        let current = std::fs::read_to_string(dir.join("transom.log")).unwrap_or_default();
        let backup = std::fs::read_to_string(dir.join("transom.log.1")).unwrap_or_default();
        assert_eq!(current, "xxxxxxxx", "新内容应写在主文件里");
        assert_eq!(backup, "0123456789abcdef", "旧内容应整体落在 .1");
        cleanup(&dir);
    }

    /// 备份份数到上限后，最旧一份必须被挤掉而不是无限堆积。
    #[test]
    fn rotation_drops_oldest_backup_beyond_keep() {
        let dir = temp_dir("keep");
        let mut writer =
            RotatingLogFile::open(&dir.join("transom.log"), 8, 2).expect("临时日志应可打开");
        for chunk in 0..5u8 {
            let _ = writer.write_all(b"0123456789");
            let _ = writer.write_all(&[b'a' + chunk; 4]);
        }
        let _ = writer.flush();
        assert!(dir.join("transom.log.1").exists(), "应保留最新一份备份");
        assert!(dir.join("transom.log.2").exists(), "应保留次新一份备份");
        assert!(
            !dir.join("transom.log.3").exists(),
            "超出保留份数的第三份不该出现在磁盘上"
        );
        cleanup(&dir);
    }

    /// 文件层停用后写入要静默成功：日志坏了不能把调用方带崩。
    #[test]
    fn disabled_writer_accepts_and_drops_records() {
        let mut writer = RotatingLogFile {
            path: PathBuf::from("unused"),
            file: None,
            written: 0,
            max_bytes: 16,
            keep: 1,
        };
        assert_eq!(
            writer.write(b"discarded").expect("停用后写入不应报错"),
            9,
            "必须按吞掉的字节数报告成功"
        );
        writer.flush().expect("停用后 flush 应成功");
    }

    /// 目录建不出来时打开要返回错误，由装配层退回 stderr，而不是 panic。
    #[test]
    fn open_fails_when_parent_is_a_file() {
        let dir = temp_dir("bad-parent");
        let blocker = dir.join("not-a-dir");
        std::fs::write(&blocker, b"x").expect("占位文件应可写入");
        let error = match RotatingLogFile::open(&blocker.join("transom.log"), 16, 1) {
            Ok(_) => panic!("父路径是普通文件时，打开日志文件必须失败"),
            Err(error) => error,
        };
        assert!(!error.to_string().is_empty(), "错误要带系统原因");
        cleanup(&dir);
    }

    /// 临时目录按用例标签隔开，避免并行用例互相踩。
    fn temp_dir(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("transom-log-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("临时目录应可创建");
        dir
    }

    fn cleanup(dir: &Path) {
        let _ = std::fs::remove_dir_all(dir);
    }
}
