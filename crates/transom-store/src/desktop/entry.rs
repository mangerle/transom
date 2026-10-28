//! 桌面集成的纯文本与纯路径层：条目内容构造、偏好列表增删、落点拼装。
//!
//! 设计原理（Why）：这一层不碰环境变量也不碰文件系统，输入输出全是字符串与
//! 路径，因此回归测试可以直接断言"置顶、去重、摘除后逐字节还原"这些最容易
//! 出错的字面行为，不需要沙箱目录；副作用与 XDG 解析留在上级模块。

use transom_core::{AppError, ErrorCode};
use transom_platform::paths::APP_ID;

/// 偏好列表文件名（规范固定名）。
const PREFERENCE_LIST: &str = "xdg-terminals.list";

/// 桌面条目 ID：文件名必须与 WM_CLASS、`StartupWMClass` 逐字一致（见
/// `transom_platform::paths::APP_ID` 的契约），加 `.desktop` 后缀即规范名。
pub fn desktop_entry_id() -> String {
    format!("{APP_ID}.desktop")
}

/// 应用图标（舷窗 logo）源文件，编译期内嵌。
///
/// 设计原理（Why）：图标注册发生在任意运行位置（target/debug、AppImage 挂载点、
/// /usr/bin），按相对路径找仓库里的 `logo/` 必然漂移；1KB 的 SVG 随二进制分发，
/// 注册出的图标与应用本体永远同源。仓库内 `logo/transom-logo.svg` 是唯一事实
/// 来源，静态资源（README、Release 附件）继续直接引用该文件。
pub(super) const ICON_SVG: &str = include_str!("../../../../logo/transom-logo.svg");

/// 纯路径形态：`root` 即 XDG 配置根。
pub(super) fn config_list_in(root: &std::path::Path) -> std::path::PathBuf {
    root.join(PREFERENCE_LIST)
}

/// 纯路径形态：`root` 即 XDG 数据根。
pub(super) fn applications_dir_in(root: &std::path::Path) -> std::path::PathBuf {
    root.join("applications")
}

/// 纯路径形态：`root` 即 XDG 数据根。
pub(super) fn hicolor_dir_in(root: &std::path::Path) -> std::path::PathBuf {
    root.join("icons").join("hicolor")
}

/// 纯路径形态：`root` 即 XDG 数据根；图标落点 `hicolor/scalable/apps/APP_ID.svg`，
/// `.desktop` 的 `Icon=` 用不带扩展名的主题名引用它。
///
/// 为什么只装 SVG 不铺多档 PNG：颗粒感的根源是 launcher 把小位图放大，而
/// GNOME/KDE 等现代实现经 librsvg 矢量出图，任何尺寸与 DPI 都锐利；预生成
/// PNG 反而要求"改 logo 必须重跑生成脚本"，一致性风险大于兼容性收益。
pub(super) fn icon_theme_path_in(root: &std::path::Path) -> std::path::PathBuf {
    hicolor_dir_in(root)
        .join("scalable")
        .join("apps")
        .join(format!("{APP_ID}.svg"))
}

/// 按优先级列出本应用需要置顶的偏好文件路径（与 launcher 的读取次序一致）。
pub(super) fn preference_paths_in(
    config_root: &std::path::Path,
    tokens: &[String],
) -> Vec<std::path::PathBuf> {
    let mut paths = Vec::with_capacity(tokens.len() + 1);
    for token in tokens {
        paths.push(config_root.join(format!("{token}-{PREFERENCE_LIST}")));
    }
    paths.push(config_list_in(config_root));
    paths
}

/// 生成桌面条目内容。
///
/// `Exec` 用绝对路径并保留 `%u`：launcher 拉起时把 `ssh://` 地址作为单个
/// 参数传入。`X-TerminalArgDir=--dir=` 让 launcher 把目标目录显式拼成选项
/// 参数（理由见上级模块头），由 `apps/transom` 的
/// `deep_link::request_from_args` 解析。`Icon` 用不带扩展名的主题名，指向
/// 上级模块安装的 `hicolor/scalable/apps/{APP_ID}.svg`。
pub fn desktop_entry_content(exe: &std::path::Path) -> String {
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

/// 把 `id` 置顶写回偏好列表：去重（同 ID 只留一份）、其余行原样保留。
///
/// `!`/`+`/`-` 前缀的排除语义是用户与 launcher 之间的约定，这里原样搬运，
/// 不做解释也不改写。
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

/// 写入 0644 文件（桌面条目与图标共用：桌面环境要求全局可读，二者都不含机密）。
pub(super) fn write_file_0644(path: &std::path::Path, content: &str) -> Result<(), AppError> {
    use std::io::Write as _;
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
        std::fs::write(path, content).map_err(|error| fail(&error))?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

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

    /// 偏好层的顺序必须与 launcher 一致：桌面 token 在前、通用文件在后。
    #[test]
    fn preference_paths_follow_launcher_order() {
        let tokens = vec!["ubuntu".to_string(), "gnome".to_string()];
        let paths = preference_paths_in(Path::new("/home/u/.config"), &tokens);
        let names: Vec<String> = paths
            .iter()
            .map(|p| p.file_name().unwrap().to_string_lossy().into_owned())
            .collect();
        assert_eq!(
            names,
            vec![
                "ubuntu-xdg-terminals.list",
                "gnome-xdg-terminals.list",
                "xdg-terminals.list"
            ]
        );
    }
}
