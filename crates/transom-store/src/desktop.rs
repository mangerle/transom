//! 桌面集成：把 transom 注册为系统默认终端（及撤销注册）。
//!
//! 设计原理（Why）：
//! - 遵循 freedesktop 的 Default Terminal Execution 规范而不是自创机制：
//!   GNOME（含 Ubuntu）的终端快捷键与文件管理器"在终端打开"都经
//!   `xdg-terminal-exec` 解析默认终端，写一份偏好列表文件即可同时覆盖
//!   两条入口，无需触碰桌面环境的私有配置（gsettings keybinding 等）；
//! - 偏好载体是 `~/.config/xdg-terminals.list`：规范规定其按行给出桌面条目
//!   ID，越靠前优先级越高。我们只做"把自己的 ID 置顶、其余行原样保留"的
//!   最小干预，不垄断文件——用户手写的其他偏好仍然生效且可见；
//! - `.desktop` 里的 Exec 写当前可执行文件的绝对路径而非裸命令名：应用经
//!   AppImage/自定义前缀等方式分发时不在 PATH，裸命令名会让 launcher 解析
//!   失败；路径随构建位置漂移，所以开关打开期间每次启动都重言一次注册；
//! - 条目声明 `X-TerminalArgDir=--dir=`：规范要求 launcher 把目标目录显式
//!   拼成该选项传给终端。为什么不靠"缺该键时 launcher 自己 cd"的兜底——
//!   那样 transom 收不到任何参数，冷启动虽能继承目录，但已有实例在跑时
//!   第二个进程只能转交"仅唤起"，文件管理器连点"在终端打开"开不出新标签；
//!   显式参数两条路径都成立且语义无歧义（实测 nautilus 50.2.2 走
//!   `xdg-terminal-exec --dir=%f`）；
//! - 图标随注册一起装进 hicolor 主题（只装内嵌 SVG）：`Icon=` 引用主题名
//!   而非仓库相对路径，矢量出图在任何尺寸与 DPI 下都没有颗粒感。
//!
//! 代价与局限（Trade-offs）：
//! - 只对实现了规范的桌面环境生效（GNOME 46+/KDE 新版本）；旧环境仍走
//!   各自的私有配置，本模块不做适配——静默改写用户桌面配置的收益不抵风险；
//! - 撤销注册只移除我们自己的列表行并把 `.desktop` 改名为 `.disabled`
//!   （可逆、不销毁用户数据），不恢复"注册前的默认终端是谁"——规范里
//!   没有可查询的旧优先级快照，伪造恢复反而危险。

use std::fs;
use std::io::Write as _;
use std::path::{Path, PathBuf};
use transom_core::{AppError, ErrorCode, paths::APP_ID};

/// 偏好列表文件名（规范固定名）。
const PREFERENCE_LIST: &str = "xdg-terminals.list";

/// 当前会话的桌面 token（`XDG_CURRENT_DESKTOP` 按 `:` 拆分并转小写）。
///
/// 为什么必须按 token 展开：launcher 的配置链是"降序优先级"，先读
/// `<token>-xdg-terminals.list`（对每个 token 依次），最后才读通用同名文件。
/// 只写通用文件时，任何既有的桌面专属文件都会把它整个盖住——Ubuntu GNOME
/// 上 `ubuntu-` 与 `gnome-` 两级就是这种情况，表现为"开关开了但默认终端
/// 没变"。取不到变量时返回空表，只剩通用文件这一级。
fn desktop_tokens() -> Vec<String> {
    std::env::var("XDG_CURRENT_DESKTOP")
        .map(|value| {
            value
                .split(':')
                .map(str::trim)
                .filter(|token| !token.is_empty())
                .map(str::to_lowercase)
                .collect()
        })
        .unwrap_or_else(|_| Vec::new())
}

/// 按优先级列出本应用需要置顶的偏好文件路径（与 launcher 的读取次序一致）。
fn preference_paths_in(config_root: &Path, tokens: &[String]) -> Vec<PathBuf> {
    let mut paths = Vec::with_capacity(tokens.len() + 1);
    for token in tokens {
        paths.push(config_root.join(format!("{token}-{PREFERENCE_LIST}")));
    }
    paths.push(config_list_in(config_root));
    paths
}

/// 解析一个 XDG 基础根目录：显式环境变量优先，缺失或为空时退回 `$HOME`
/// 下的规范默认值。
///
/// 为什么不用 directories crate 的 DataDirs：它对用户级目录同样只认
/// `XDG_DATA_HOME`，但主目录取自 `getpwuid` 而非 `$HOME`，与 launcher 脚本
/// 的解析顺序不一致。这里逐字对齐规范。环境变量解析只收口在这两个生产
/// 入口——`_in` 系列是纯路径函数，端到端测试传临时沙箱根，绝不触碰真实
/// 用户配置，也不会被开发机上显式设置的 `XDG_*` 变量污染。
fn xdg_root(env_var: &str, fallback_rel: &Path) -> Result<PathBuf, AppError> {
    let from_env = std::env::var_os(env_var)
        .filter(|value| !value.is_empty())
        .map(PathBuf::from);
    if let Some(dir) = from_env {
        return Ok(dir);
    }
    let home = std::env::var_os("HOME")
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
        .ok_or_else(|| {
            AppError::with_detail(
                ErrorCode::Configuration,
                format!("无法定位用户目录：环境变量 {env_var} 与 HOME 均未设置"),
            )
        })?;
    Ok(home.join(fallback_rel))
}

/// 偏好列表所在的 XDG 配置根。
fn config_root() -> Result<PathBuf, AppError> {
    xdg_root("XDG_CONFIG_HOME", Path::new(".config"))
}

/// 桌面条目与图标所在的 XDG 数据根。
fn data_root() -> Result<PathBuf, AppError> {
    xdg_root("XDG_DATA_HOME", Path::new(".local/share"))
}

/// 纯路径形态：`root` 即 XDG 配置根。
fn config_list_in(root: &Path) -> PathBuf {
    root.join(PREFERENCE_LIST)
}

/// 纯路径形态：`root` 即 XDG 数据根。
fn applications_dir_in(root: &Path) -> PathBuf {
    root.join("applications")
}

/// 纯路径形态：`root` 即 XDG 数据根。
fn hicolor_dir_in(root: &Path) -> PathBuf {
    root.join("icons").join("hicolor")
}

/// 纯路径形态：`root` 即 XDG 数据根；图标落点 `hicolor/scalable/apps/APP_ID.svg`，
/// `.desktop` 的 `Icon=` 用不带扩展名的主题名引用它。
///
/// 为什么只装 SVG 不铺多档 PNG：颗粒感的根源是 launcher 把小位图放大，而
/// GNOME/KDE 等现代实现经 librsvg 矢量出图，任何尺寸与 DPI 都锐利；预生成
/// PNG 反而要求"改 logo 必须重跑生成脚本"，一致性风险大于兼容性收益。
fn icon_theme_path_in(root: &Path) -> PathBuf {
    hicolor_dir_in(root)
        .join("scalable")
        .join("apps")
        .join(format!("{APP_ID}.svg"))
}

/// 桌面条目 ID：文件名必须与 WM_CLASS、`StartupWMClass` 逐字一致（见
/// `transom_core::paths::APP_ID` 的契约），加 `.desktop` 后缀即规范名。
pub fn desktop_entry_id() -> String {
    format!("{APP_ID}.desktop")
}

/// 应用图标（舷窗 logo）源文件，编译期内嵌。
///
/// 设计原理（Why）：图标注册发生在任意运行位置（target/debug、AppImage 挂载点、
/// /usr/bin），按相对路径找仓库里的 `logo/` 必然漂移；1KB 的 SVG 随二进制分发，
/// 注册出的图标与应用本体永远同源。仓库内 `logo/transom-logo.svg` 是唯一事实
/// 来源，静态资源（README、Release 附件）继续直接引用该文件。
const ICON_SVG: &str = include_str!("../../../logo/transom-logo.svg");

/// 图标安装的纯路径版本，便于在临时沙箱里做端到端验证。
fn install_icon_at(path: &Path) -> Result<(), AppError> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|error| {
            AppError::with_detail(
                ErrorCode::Io,
                format!(
                    "创建图标主题目录失败，路径: {}，原因: {error}",
                    parent.display()
                ),
            )
        })?;
    }
    if fs::read_to_string(path).is_ok_and(|existing| existing == ICON_SVG) {
        return Ok(());
    }
    write_file_0644(path, ICON_SVG)
}

/// 移除主题图标；文件本就不存在视为成功（撤销语义是"回到未注册"）。
fn remove_icon_at(path: &Path) {
    if let Err(error) = fs::remove_file(path)
        && error.kind() != std::io::ErrorKind::NotFound
    {
        log::warn!("移除主题图标失败，路径: {}，原因: {error}", path.display());
    }
}

/// 生成桌面条目内容。
///
/// `Exec` 用绝对路径并保留 `%u`：launcher 拉起时把 `ssh://` 地址作为单个
/// 参数传入。`X-TerminalArgDir=--dir=` 让 launcher 把目标目录显式拼成选项
/// 参数（理由见模块头），由 `apps/transom` 的 `deep_link::request_from_args`
/// 解析。`Icon` 用不带扩展名的主题名，指向 [`install_icon_at`] 安装的
/// `hicolor/scalable/apps/{APP_ID}.svg`。
pub fn desktop_entry_content(exe: &Path) -> String {
    format!(
        "[Desktop Entry]\n\
         Type=Application\n\
         Version=1.0\n\
         Name=Transom\n\
         GenericName=SSH Client\n\
         GenericName[zh_CN]=SSH 客户端\n\
         Comment=Native SSH, serial console and terminal workspace with integrated SFTP\n\
         Comment[zh_CN]=原生 SSH、串口与终端工作台，内置 SFTP 文件传输\n\
         Exec={exe} %u\n\
         Icon={app_id}\n\
         Terminal=false\n\
         Categories=System;TerminalEmulator;\n\
         Keywords=ssh;sftp;serial;terminal;远程登录;串口;终端;\n\
         StartupNotify=true\n\
         StartupWMClass={app_id}\n\
         MimeType=x-scheme-handler/ssh;\n\
         X-TerminalArgDir=--dir=\n",
        exe = exe.display(),
        app_id = APP_ID,
    )
}

/// 把 `id` 置顶写回偏好列表：去重（同 ID 只留一份）、其余行原样保留、
/// 忽略 `!` 开头的禁用行语义不改动（那是用户与 launcher 之间的约定）。
pub fn preference_with_prepended(list: &str, id: &str) -> String {
    let rest: Vec<&str> = list.lines().filter(|line| line.trim() != id).collect();
    let mut out = String::with_capacity(list.len() + id.len() + 1);
    out.push_str(id);
    out.push('\n');
    for line in rest {
        out.push_str(line);
        out.push('\n');
    }
    out
}

/// 从偏好列表移除 `id`（含带 `:` 动作后缀的引用形式），其余行保持原序。
pub fn preference_without(list: &str, id: &str) -> String {
    let kept = list
        .lines()
        .filter(|line| {
            let entry = line.trim();
            !entry.is_empty() && entry != id && !entry.starts_with(&format!("{id}:"))
        })
        .collect::<Vec<_>>()
        .join("\n");
    // 非空内容统一以换行结尾，与写入约定一致
    if kept.is_empty() { kept } else { kept + "\n" }
}

/// 按开关意图应用注册状态：唯一的对外执行入口，集中解析当前可执行文件路径。
///
/// 设计原理（Why）：设置页开关与启动重言必须走同一条路径，否则两处对
/// "已注册"的判定迟早漂移；可执行文件路径只在这里解析一次。
///
/// # Errors
/// 透传 [`register_default_terminal`] / [`unregister_default_terminal`] 的
/// 领域错误，由调用方（设置页或启动流程）记录与提示。
pub fn apply_default_terminal(enabled: bool) -> Result<(), AppError> {
    if enabled {
        let exe = std::env::current_exe().map_err(|error| {
            AppError::with_detail(
                ErrorCode::Io,
                format!("无法定位当前可执行文件，无法注册默认终端，原因: {error}"),
            )
        })?;
        register_default_terminal(&exe)
    } else {
        unregister_default_terminal()
    }
}

/// 注册为默认终端：写桌面条目 + 置顶偏好列表 + 刷新桌面数据库。
///
/// 幂等：重复调用只是覆盖同名文件并把 ID 保持在列表首位，可随启动重言。
///
/// # Errors
/// 目录创建、文件写入或改名失败时返回 `Io`/`Configuration` 域错误，
/// 附中文路径与原因，由调用方（设置页）统一记录与提示。
pub fn register_default_terminal(exe: &Path) -> Result<(), AppError> {
    let config = config_root()?;
    let data = data_root()?;
    let result = register_default_terminal_in(&config, &data, &desktop_tokens(), exe);
    // 无论成败都刷新缓存：半途失败时已写入的碎片同样需要被桌面环境看见或绕开。
    refresh_desktop_database(&applications_dir_in(&data));
    refresh_icon_cache_in(&data);
    result
}

/// 注册的纯路径版本：落点由显式传入的 XDG 配置根、桌面 token 列表与数据根
/// 推导。端到端测试用它把文件写进临时沙箱（不读环境变量，绝不误伤真实用户
/// 配置）。
///
/// 每个优先级层的偏好文件都要置顶：launcher 按层合并候选，漏掉任何一层都会
/// 让该层里已有的终端排在我們前面。写入是"置顶自身、其余行原样保留"，用户
/// 手写的其他偏好一条不丢。
fn register_default_terminal_in(
    config_root: &Path,
    data_root: &Path,
    tokens: &[String],
    exe: &Path,
) -> Result<(), AppError> {
    let apps_dir = applications_dir_in(data_root);
    fs::create_dir_all(&apps_dir).map_err(|error| {
        AppError::with_detail(
            ErrorCode::Io,
            format!(
                "创建桌面条目目录失败，路径: {}，原因: {error}",
                apps_dir.display()
            ),
        )
    })?;
    let desktop_path = apps_dir.join(desktop_entry_id());
    // 先删旧的同名 `.disabled`：撤销后再次开启时，避免残留文件与正式条目并存
    // 造成桌面环境展示两份入口。
    let _ = fs::remove_file(apps_dir.join(format!("{}.disabled", desktop_entry_id())));
    write_file_0644(&desktop_path, &desktop_entry_content(exe))?;
    install_icon_at(&icon_theme_path_in(data_root))?;

    fs::create_dir_all(config_root).map_err(|error| {
        AppError::with_detail(
            ErrorCode::Io,
            format!(
                "创建 XDG 配置目录失败，路径: {}，原因: {error}",
                config_root.display()
            ),
        )
    })?;
    let id = desktop_entry_id();
    for list_path in preference_paths_in(config_root, tokens) {
        let existing = read_list(&list_path)?;
        let updated = preference_with_prepended(&existing, &id);
        fs::write(&list_path, updated).map_err(|error| {
            AppError::with_detail(
                ErrorCode::Io,
                format!(
                    "写入默认终端偏好列表失败，路径: {}，原因: {error}",
                    list_path.display()
                ),
            )
        })?;
    }
    Ok(())
}

/// 撤销注册（生产入口）：语义见 `_in` 版本；随后刷新桌面与图标缓存。
pub fn unregister_default_terminal() -> Result<(), AppError> {
    let config = config_root()?;
    let data = data_root()?;
    let result = unregister_default_terminal_in(&config, &data, &desktop_tokens());
    refresh_desktop_database(&applications_dir_in(&data));
    refresh_icon_cache_in(&data);
    result
}

/// 撤销注册：移除偏好列表中的自身条目，并把桌面条目改名为 `.disabled`
/// （规范规定的可逆禁用方式，不销毁文件）。列表移除后为空则删除文件。
/// 主题图标直接删除：文件内嵌于二进制，重装即恢复，留 `.disabled` 只会
/// 让应用网格在极端时序下捡到旧图。
///
/// # Errors
/// 偏好列表或桌面条目读写失败时返回带路径与原因的 `Io` 域错误。
fn unregister_default_terminal_in(
    config_root: &Path,
    data_root: &Path,
    tokens: &[String],
) -> Result<(), AppError> {
    let id = desktop_entry_id();
    for list_path in preference_paths_in(config_root, tokens) {
        if !list_path.exists() {
            continue;
        }
        let existing = read_list(&list_path)?;
        let updated = preference_without(&existing, &id);
        if updated.trim().is_empty() {
            fs::remove_file(&list_path).map_err(|error| {
                AppError::with_detail(
                    ErrorCode::Io,
                    format!(
                        "删除空偏好列表失败，路径: {}，原因: {error}",
                        list_path.display()
                    ),
                )
            })?;
        } else {
            fs::write(&list_path, updated).map_err(|error| {
                AppError::with_detail(
                    ErrorCode::Io,
                    format!(
                        "更新默认终端偏好列表失败，路径: {}，原因: {error}",
                        list_path.display()
                    ),
                )
            })?;
        }
    }
    let apps_dir = applications_dir_in(data_root);
    let desktop_path = apps_dir.join(&id);
    if desktop_path.exists() {
        fs::rename(&desktop_path, apps_dir.join(format!("{id}.disabled"))).map_err(|error| {
            AppError::with_detail(
                ErrorCode::Io,
                format!(
                    "禁用桌面条目失败，路径: {}，原因: {error}",
                    desktop_path.display()
                ),
            )
        })?;
    }
    remove_icon_at(&icon_theme_path_in(data_root));
    Ok(())
}

/// 当前是否已是 transom 置顶的默认终端注册（供启动重言判定与界面显示）。
///
/// 只做轻量的读取比对，不写文件；调用方据此决定是否重言注册。图标与
/// 桌面条目任一漂移都判"未注册"，触发重言自愈。
pub fn default_terminal_registered(exe: &Path) -> bool {
    let Ok(config) = config_root() else {
        return false;
    };
    let Ok(data) = data_root() else {
        return false;
    };
    default_terminal_registered_in(&config, &data, &desktop_tokens(), exe)
}

fn default_terminal_registered_in(
    config_root: &Path,
    data_root: &Path,
    tokens: &[String],
    exe: &Path,
) -> bool {
    let id = desktop_entry_id();
    // 生效链首位：launcher 按层合并候选，真正决定默认终端的是"第一个非空
    // 层里的第一行"。某层被用户新建并把别的终端排在前面，就等于把我们挤下
    // 首位，这里必须判为未注册并重言自愈。
    let mut chain_head_is_ours = false;
    for list_path in preference_paths_in(config_root, tokens) {
        let Ok(list) = read_list(&list_path) else {
            return false;
        };
        let Some(head) = list.lines().map(str::trim).find(|line| !line.is_empty()) else {
            continue;
        };
        chain_head_is_ours = head == id;
        break;
    }
    if !chain_head_is_ours {
        return false;
    }
    let desktop_path = applications_dir_in(data_root).join(&id);
    if !fs::read_to_string(&desktop_path)
        .map(|content| content.contains(&format!("Exec={}", exe.display())))
        .unwrap_or(false)
    {
        return false;
    }
    fs::read_to_string(icon_theme_path_in(data_root)).is_ok_and(|content| content == ICON_SVG)
}

/// 写入 0644 文件（桌面条目与图标共用：桌面环境要求全局可读，二者都不含机密）。
fn write_file_0644(path: &Path, content: &str) -> Result<(), AppError> {
    let fail = |error: &std::io::Error| {
        AppError::with_detail(
            ErrorCode::Io,
            format!(
                "写入桌面集成文件失败，路径: {}，原因: {error}",
                path.display()
            ),
        )
    };
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        let mut file = std::fs::OpenOptions::new()
            .create(true)
            .truncate(true)
            .write(true)
            .mode(0o644)
            .open(path)
            .map_err(|error| fail(&error))?;
        file.write_all(content.as_bytes())
            .map_err(|error| fail(&error))?;
    }
    #[cfg(not(unix))]
    {
        fs::write(path, content).map_err(|error| fail(&error))?;
    }
    Ok(())
}

/// 读偏好列表；不存在视为空列表（首次注册就是这一形态）。
fn read_list(path: &Path) -> Result<String, AppError> {
    match fs::read_to_string(path) {
        Ok(text) => Ok(text),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(String::new()),
        Err(error) => Err(AppError::with_detail(
            ErrorCode::Io,
            format!(
                "读取默认终端偏好列表失败，路径: {}，原因: {error}",
                path.display()
            ),
        )),
    }
}

/// 刷新 hicolor 图标主题缓存（`data_root` 为 XDG 数据根）；失败只降级
/// （无缓存时桌面环境回退目录扫描），不外抛。子进程同线程收割，防僵尸。
fn refresh_icon_cache_in(data_root: &Path) {
    let theme_dir = hicolor_dir_in(data_root);
    if !theme_dir.exists() {
        return;
    }
    match std::process::Command::new("gtk-update-icon-cache")
        .arg(&theme_dir)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
    {
        Ok(mut child) => {
            let _ = child.wait();
        }
        Err(error) => {
            log::warn!(
                "刷新图标主题缓存失败（不影响注册，桌面环境会自行扫描），路径: {}，原因: {error}",
                theme_dir.display()
            );
        }
    }
}

/// 刷新桌面数据库缓存；失败只降级（gio 会回退到逐目录扫描），不外抛。
fn refresh_desktop_database(apps_dir: &Path) {
    match std::process::Command::new("update-desktop-database")
        .arg(apps_dir)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
    {
        // 同线程收割，杜绝僵尸进程；退出码不影响正确性，缓存新旧只影响可见延迟。
        Ok(mut child) => {
            let _ = child.wait();
        }
        Err(error) => {
            log::warn!(
                "刷新桌面条目数据库失败（不影响注册，桌面环境会自行扫描），路径: {}，原因: {error}",
                apps_dir.display()
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_id() -> &'static str {
        "com.github.mangerle.transom.desktop"
    }

    #[test]
    fn prepend_puts_id_first_and_keeps_others() {
        let list = "org.gnome.Ptyxis.desktop:new-window\ntabby.desktop\n";
        let updated = preference_with_prepended(list, sample_id());
        let lines: Vec<&str> = updated.lines().collect();
        assert_eq!(lines[0], sample_id());
        assert_eq!(lines[1], "org.gnome.Ptyxis.desktop:new-window");
        assert_eq!(lines[2], "tabby.desktop");
    }

    #[test]
    fn prepend_dedupes_existing_entry() {
        let list = format!("{}\nsomething.desktop\n", sample_id());
        let updated = preference_with_prepended(&list, sample_id());
        assert_eq!(updated.lines().count(), 2, "同 ID 不应重复出现");
        assert_eq!(updated.lines().next().unwrap(), sample_id());
    }

    #[test]
    fn prepend_on_empty_list_yields_single_line() {
        let updated = preference_with_prepended("", sample_id());
        assert_eq!(updated, format!("{}\n", sample_id()));
    }

    #[test]
    fn remove_drops_id_and_action_refs_but_keeps_others() {
        let list = format!(
            "{}:new-window\n{}\nsome.desktop\n",
            sample_id(),
            sample_id()
        );
        let updated = preference_without(&list, sample_id());
        assert_eq!(updated, "some.desktop\n");
    }

    #[test]
    fn remove_of_only_entry_yields_empty_string() {
        let list = format!("{}\n", sample_id());
        assert_eq!(preference_without(&list, sample_id()), "");
    }

    #[test]
    fn desktop_entry_content_carries_absolute_exec_and_dir_arg_option() {
        let content = desktop_entry_content(Path::new("/opt/transom/bin/transom"));
        assert!(content.contains("Exec=/opt/transom/bin/transom %u"));
        assert!(content.contains("Categories=System;TerminalEmulator;"));
        assert!(content.contains(&format!("StartupWMClass={APP_ID}")));
        assert!(content.contains(&format!("Icon={APP_ID}")));
        // 目录必须以显式选项传进来：缺该键时 launcher 只会自己 cd，
        // 已有实例的交接路径就拿不到目录了。
        assert!(
            content.contains("X-TerminalArgDir=--dir="),
            "条目必须声明 X-TerminalArgDir，否则已运行实例开不出带目录的标签"
        );
    }

    /// 端到端闭环：临时沙箱里注册 -> 查询 -> 撤销 -> 再注册。
    ///
    /// 为什么走 `_in` 纯路径版本而不是设环境变量：`set_var` 是进程级全局
    /// 且并行测试会互踩，显式传沙箱根与 token 表既隔离又确定性；这条测试
    /// 覆盖的是"目录创建、文件内容、列表置顶与撤销还原"的真实文件系统形态。
    #[test]
    fn register_write_unregister_round_trip_in_sandbox() {
        let sandbox = tempfile::tempdir().expect("沙箱目录应可创建");
        let config = sandbox.path().join("config");
        let data = sandbox.path().join("data");
        let tokens: Vec<String> = Vec::new();
        let exe = Path::new("/opt/transom/bin/transom");
        let id = desktop_entry_id();

        // 初始未注册
        assert!(!default_terminal_registered_in(
            &config, &data, &tokens, exe
        ));

        // 注册：三类落点齐备，列表首位是自己的 ID
        register_default_terminal_in(&config, &data, &tokens, exe).expect("沙箱注册应成功");
        let desktop = applications_dir_in(&data).join(&id);
        assert!(
            fs::read_to_string(&desktop)
                .unwrap()
                .contains("Exec=/opt/transom/bin/transom %u")
        );
        let icon = icon_theme_path_in(&data);
        assert_eq!(fs::read_to_string(&icon).unwrap(), ICON_SVG);
        let list = fs::read_to_string(config_list_in(&config)).unwrap();
        assert_eq!(list.lines().next().unwrap(), id);
        assert!(default_terminal_registered_in(&config, &data, &tokens, exe));

        // 重言幂等：重复注册不新增列表行
        register_default_terminal_in(&config, &data, &tokens, exe).unwrap();
        let list = fs::read_to_string(config_list_in(&config)).unwrap();
        assert_eq!(list.lines().filter(|line| line.trim() == id).count(), 1);

        // 可执行文件路径漂移后判定失效，触发重言自愈
        assert!(!default_terminal_registered_in(
            &config,
            &data,
            &tokens,
            Path::new("/elsewhere/transom")
        ));
        register_default_terminal_in(&config, &data, &tokens, Path::new("/elsewhere/transom"))
            .unwrap();
        assert!(default_terminal_registered_in(
            &config,
            &data,
            &tokens,
            Path::new("/elsewhere/transom")
        ));

        // 撤销：条目改名 `.disabled`、图标删除、空列表文件移除
        unregister_default_terminal_in(&config, &data, &tokens).expect("沙箱撤销应成功");
        assert!(!desktop.exists());
        assert!(
            applications_dir_in(&data)
                .join(format!("{id}.disabled"))
                .exists()
        );
        assert!(!icon.exists());
        assert!(!config_list_in(&config).exists());
        assert!(!default_terminal_registered_in(
            &config,
            &data,
            &tokens,
            Path::new("/elsewhere/transom")
        ));
    }

    /// 撤销只摘除自身：用户手写的其他偏好行必须原样保留。
    #[test]
    fn unregister_keeps_foreign_preferences() {
        let sandbox = tempfile::tempdir().expect("沙箱目录应可创建");
        let config = sandbox.path().join("config");
        let data = sandbox.path().join("data");
        let tokens: Vec<String> = Vec::new();
        fs::create_dir_all(&config).unwrap();
        let other = "org.gnome.Ptyxis.desktop:new-window";
        fs::write(config_list_in(&config), format!("{other}\n")).unwrap();

        register_default_terminal_in(&config, &data, &tokens, Path::new("/usr/bin/transom"))
            .unwrap();
        unregister_default_terminal_in(&config, &data, &tokens).unwrap();

        let list = fs::read_to_string(config_list_in(&config)).unwrap();
        assert_eq!(list, format!("{other}\n"), "撤销后应只剩用户原有偏好行");
    }

    /// 回归：桌面专属层必须一起置顶。
    ///
    /// 缺陷现场：launcher 的配置链是 `ubuntu-` > `gnome-` > 通用（降序优先级、
    /// 逐层合并候选）。早期只写通用文件，用户机器上已存在的
    /// `ubuntu-xdg-terminals.list`（内容是 Ptyxis）把我们的首行整个盖住，
    /// 表现为"开关打开但默认终端没变"。这里钉死三层都置顶、且判定看的是
    /// 生效链首位而不是通用文件。
    #[test]
    fn register_prepends_every_desktop_layer() {
        let sandbox = tempfile::tempdir().expect("沙箱目录应可创建");
        let config = sandbox.path().join("config");
        let data = sandbox.path().join("data");
        fs::create_dir_all(&config).unwrap();
        let tokens = ["ubuntu".to_string(), "gnome".to_string()];
        let ptyxis = "org.gnome.Ptyxis.desktop:new-window";
        // 复现现场：两层桌面文件都已存在且首位是别的终端
        fs::write(
            config.join("ubuntu-xdg-terminals.list"),
            format!("{ptyxis}\n"),
        )
        .unwrap();
        fs::write(
            config.join("gnome-xdg-terminals.list"),
            format!("org.gnome.Terminal.desktop\n{ptyxis}\n"),
        )
        .unwrap();

        // 注册前：生效链首位不是我们
        let exe = Path::new("/usr/bin/transom");
        assert!(!default_terminal_registered_in(
            &config, &data, &tokens, exe
        ));

        register_default_terminal_in(&config, &data, &tokens, exe).unwrap();
        for name in [
            "ubuntu-xdg-terminals.list",
            "gnome-xdg-terminals.list",
            "xdg-terminals.list",
        ] {
            let content = fs::read_to_string(config.join(name)).unwrap();
            assert_eq!(
                content.lines().next().unwrap(),
                desktop_entry_id(),
                "层 {name} 的首行必须是本应用"
            );
        }
        assert!(default_terminal_registered_in(&config, &data, &tokens, exe));

        // 撤销：三层里的自身行都摘掉，用户其余行原样保留
        unregister_default_terminal_in(&config, &data, &tokens).unwrap();
        assert_eq!(
            fs::read_to_string(config.join("ubuntu-xdg-terminals.list")).unwrap(),
            format!("{ptyxis}\n")
        );
        assert_eq!(
            fs::read_to_string(config.join("gnome-xdg-terminals.list")).unwrap(),
            format!("org.gnome.Terminal.desktop\n{ptyxis}\n")
        );
        assert!(
            !config.join("xdg-terminals.list").exists(),
            "只含自身行的通用文件应被删除"
        );
    }

    /// 某层被用户事后插队时，判定必须翻成"未注册"以便启动重言自愈。
    #[test]
    fn registered_check_detects_layer_overtake() {
        let sandbox = tempfile::tempdir().expect("沙箱目录应可创建");
        let config = sandbox.path().join("config");
        let data = sandbox.path().join("data");
        let tokens = ["ubuntu".to_string()];
        let exe = Path::new("/usr/bin/transom");
        register_default_terminal_in(&config, &data, &tokens, exe).unwrap();
        assert!(default_terminal_registered_in(&config, &data, &tokens, exe));

        // 用户在最高优先层前面又塞了别的终端
        let high = config.join("ubuntu-xdg-terminals.list");
        let mine = fs::read_to_string(&high).unwrap();
        fs::write(&high, format!("other.desktop\n{mine}")).unwrap();
        assert!(
            !default_terminal_registered_in(&config, &data, &tokens, exe),
            "被插队后应判为未注册"
        );
    }
}
