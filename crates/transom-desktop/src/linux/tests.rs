//! 桌面集成模块单元测试与沙箱端到端回归测试。

use super::entry::config_list_in;
use super::*;
use std::fs;
use std::path::Path;

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
    register_default_terminal_in(&config, &data, &tokens, Path::new("/elsewhere/transom")).unwrap();
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

    register_default_terminal_in(&config, &data, &tokens, Path::new("/usr/bin/transom")).unwrap();
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
