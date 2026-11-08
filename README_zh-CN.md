<div align="center">
  <img src="./logo/transom-lockup.svg" alt="Transom" width="420" />

  <p>
    <a href="./README.md"><strong>English</strong></a> ·
    <a href="./README_zh-CN.md"><strong>简体中文</strong></a>
  </p>

  <p>
    <a href="https://github.com/mangerle/transom/blob/main/LICENSE"><img alt="License" src="https://img.shields.io/badge/license-Apache--2.0-blue" /></a>
    <img alt="Rust" src="https://img.shields.io/badge/rust-1.96%2B-orange" />
    <img alt="Platform" src="https://img.shields.io/badge/platform-Linux%20(主)%20%7C%20macOS%20%7C%20Windows-informational" />
    <img alt="Status" src="https://img.shields.io/badge/status-alpha-red" />
  </p>

  <p>基于 Rust + GPUI 的原生 SSH、串口与终端工作台，内置 SFTP 文件传输。</p>
</div>

---

**Transom** 是一个桌面运维客户端：一个窗口同时管住远端主机的 Shell、开发板与交换机 Console 口的串口会话，以及与之配套的远程文件。界面通过 [GPUI](https://github.com/zed-industries/zed) 渲染，是真正走 GPU 的终端网格，而不是内嵌网页视图；连接链路全部为原生 Rust——SSH2 用 [`russh`](https://crates.io/crates/russh)、文件传输用 [`russh-sftp`](https://crates.io/crates/russh-sftp)、终端仿真内核用 [`alacritty_terminal`](https://crates.io/crates/alacritty_terminal)。

关于名字：*transom* 是门框上的横档，而船上的横档就是舷窗。当你在意的机器隔着网络时，这个工具就是你瞭望用的那道横梁。

## 设计取向

- **原生渲染，不嵌网页。** 没有 Electron，也不打包 Chromium。布局走 Taffy/GPUI，文字排版由真实字体引擎完成，debug 构建下 50×120 全可见区重绘实测约 1.7 ms/帧。
- **终端与文件同生命周期。** SFTP 自动跟随 Shell 的当前工作目录，每个标签页各自持有自己的远程会话状态。
- **凭据不出本机。** 口令与私钥 passphrase 存放在操作系统钥匙串；会话文件以原子写落盘并置 `0600` 权限，日志中不出现明文凭证。
- **中文优先的排版。** 拉丁与中日韩字形使用两套可独立配置的字体，中文字号按探针字形反推到"恰好两格宽"，因此换任意中文字体都不会错位。

## 功能特性

### 会话与协议
- 会话树支持多层嵌套文件夹、拖拽归类、实时过滤与多选批量改属性。
- 连接类型：SSH2（密码、键盘交互、公钥、OpenSSH CA 用户证书、Unix OpenSSH Agent、Windows OpenSSH Agent 或 Pageant）、本地 PTY Shell、以及面向嵌入式板卡与交换机管理口的串口连接。
- 只保留现代加密套件：AES-128/256-GCM 与 ChaCha20-Poly1305、SHA2-512 MAC、`zlib` 传输压缩，密钥支持 RSA / ECDSA / Ed25519。
- 连接层默认每 30 秒心跳、连续 3 次无应答断开，避免云厂商防火墙静默掐断空闲连接。
- 右键单个会话或整个文件夹即可对本组主机批量拨测（逐行显示 TCP 握手耗时，同时在途 16 路），并向会话配置的网卡 MAC 发送 Wake-on-LAN 魔法包。
- 启动时自动恢复并重连上次打开的会话；会话库可导出/导入为明文 JSON/YAML，或导出为 Argon2id + XChaCha20-Poly1305 加密备份。
- 快速打开面板（`$app-k`）兼作免保存直连：输入 `deploy@10.0.0.8:2222` 即可直接发起连接，这条会话不会进入会话列表。
- 凭据模板（Identity）：一个账号只存一份用户名与密码/私钥，任意多个会话引用它，改密只改一处即全量生效；模板与其钥匙串槽位随会话库一起走备份与云同步。

### 终端
- 标签内自由分屏：切分方向属于每一刀、可嵌套，分隔线可拖拽，支持单窗格最大化与拆出为独立标签。
- 缓冲区全文检索带正则开关，右侧刻度带显示整块滚动缓冲区的命中分布并支持点击跳转。
- 常驻高亮 `ERROR` / `WARN` / `INFO` / `DEBUG`、IPv4 与 UUID，另可加最多 32 个自定义关键词；颜色全部取自主题调色板，跟随配色切换。
- <kbd>Alt</kbd>+拖拽矩形列选（在 `vim`、`htop` 里同样可用）、双击选词分隔符可自定义、命令耗时超过阈值时通过 OSC 133 / OSC 777 发出桌面通知。
- 快捷命令条（Snippets）把常用单行命令填充到焦点窗格。
- 命令历史与输入建议：命令取自 shell 集成标记（OSC 133 上报的文本，取不到时按"提示符结束点到光标"的网格快照补采），存在 `~/.config/transom/history.json`，边打字边提示；<kbd>Tab</kbd> 补全第一条建议，回车仍按你敲的内容执行。

### 端口转发
- 对运行中的会话随时启停本地、远程与动态（SOCKS5）隧道，并有独立面板查看规则、连接数与通道状态。

### SFTP
- 本地/远程双栏文件浏览器，支持断点续传、传输队列置顶插队、失败一键重试与已完成清空，逐任务显示速率与百分比。
- 文件直接拖到终端网格即上传到该会话的当前远端目录：与上传按钮走同一套续传/校验/队列调度，无需先打开文件面板。
- 按大小与修改时间做目录差异对比与勾选式增量同步；传输后保留源文件 mtime，避免重复回传。
- 可选的传输后 SHA-256 校验（两端均按 64 KiB 分块流式计算并比对），以及上传目录时按通配符剪枝 `.git/`、`node_modules/` 的排除规则。
- 内置远程编辑器：只读保护、保存前统一 diff 预览、写回前自动备份；`chmod` 提供 9 宫格与八进制两种输入；图片与文本首数十行悬停预览；`Ctrl+F` 文件名即时过滤。

### 界面
- 内置 6 套主题（One Dark、Dracula、Monokai Pro、Nord、Solarized Dark、Gruvbox Dark），客户端窗口控制三键支持拖动与双击缩放，记忆窗口尺寸与位置，提供沉浸式全屏。
- 界面中英双语，可在设置中切换——本 README 的双语结构与应用一致。
- 可选的会话输出审计：每次连接的原始输出字节流可落盘到 `~/.config/transom/audit`，按会话名、主机与开始时间一连接一文件。默认关闭——日志会包含屏幕上出现过的全部内容。
- 所有快捷键均可自定义并带冲突检测。下表中的 `$app` 在 macOS 为 <kbd>Cmd</kbd>，其他平台为 <kbd>Ctrl</kbd>+<kbd>Shift</kbd>。

| 动作 | 默认键 | 动作 | 默认键 |
| :--- | :--- | :--- | :--- |
| 新建会话 | `$app-n` | 向右/向下分屏 | `$app-d` / `$app-e` |
| 新建本地终端 | `$app-t` | 关闭窗格 / 窗格最大化 | `$app-x` / `$app-enter` |
| 快速打开 | `$app-k` | 窗格焦点移动 | `$app-alt-←↑→↓` |
| 关闭标签页 | `$app-w` | 侧边栏 / SFTP / 转发面板 | `$app-b` / `$app-s` / `$app-p` |
| 下一个 / 上一个标签 | `ctrl-tab` / `ctrl-shift-tab` | 快捷命令条 | `$app-j` |
| 重新连接 | `$app-r` | 终端检索 | `$app-f` |
| 沉浸式全屏 | `f11` / `alt-enter` | | |

## 平台支持

| 平台 | 支持等级 | 已验证能力 |
| :--- | :--- | :--- |
| Linux（Wayland / X11） | 主要交互开发平台 | 完整原生 CI、deb 与 AppImage 发布构建、XDG 默认终端集成、`ssh://` 单实例转交。 |
| macOS（Intel / Apple silicon） | 支持 | 原生 CI 测试与 Clippy、Developer ID 签名并公证的 DMG、已签名更新二进制、LaunchServices `ssh://` 注册、系统原生打开能力。 |
| Windows（x64 / arm64） | 支持 | 原生 CI 测试与 Clippy、Authenticode 签名的主程序与 NSIS 安装包、命名管道/WSL/WSLC 容器发现、`ssh://` 候选注册、系统原生打开能力。 |

三个平台的 CI 都会真实运行，但属于无头测试；具体硬件、显卡或桌面环境仍应在代表性机器上做发布前冒烟验证。

明确的平台差异：

- 默认终端接管遵循 freedesktop XDG 规范，因此只在 Linux 提供。macOS 没有 XDG 式系统默认终端 API，Windows 也没有等价的全系统默认终端注册；两端只登记 `ssh://` 应用候选，最终默认项由用户选择。
- macOS 自更新压缩包只替换 `Transom.app/Contents/MacOS/transom`，`Info.plist` 与包内资源保持安装时版本；需要独立更新资源时必须改为 `.pkg` 或整包替换路径。
- Windows 的 WSL 容器发现、WSL Mosh、OpenSSH Agent/Pageant、Docker Desktop、WSLC 与 Podman machine 集成依赖对应外部工具已安装并配置。Linux 终端超链接依赖 `xdg-open`，macOS 与 Windows 使用系统原生打开接口。

### 桌面集成（默认终端、`ssh://` 链接）

Linux 上 Transom 可以接管浏览器或其他工具发出的 `ssh://[user@]host[:port]` 链接，并以单实例方式运行：第二次启动会把请求通过 `$XDG_RUNTIME_DIR/transom` 下的私有套接字交给已在运行的窗口，然后立即退出。

**设为默认终端**——设置 → 通用 → 桌面集成（仅 Linux）。这一步刻意做成显式开关：多数发行版已把终端快捷键与 `ssh://` 绑给既有终端，私自抢占会影响本项目之外的行为。打开后只往当前用户目录写三样东西：

- `~/.local/share/applications/com.github.mangerle.transom.desktop`（`Exec` 写绝对路径，任何安装位置都能解析）；
- `~/.local/share/icons/hicolor/scalable/apps/com.github.mangerle.transom.svg`——图标只随一份矢量 SVG，桌面环境用 librsvg 按目标尺寸出图，任何大小与 DPI 都不会出现放大颗粒感；
- 把本应用的条目 ID 置顶写入 `~/.config/xdg-terminals.list`，其余偏好行原样保留。

这套做法遵循 freedesktop 的默认终端执行规范，GNOME（含 Ubuntu）的终端快捷键与文件管理器"在终端打开"都走该规范（实测 `nautilus 50.2.2` 调用 `xdg-terminal-exec --dir=%f`）。条目声明 `X-TerminalArgDir=--dir=`，目标目录以显式选项传入，因此两条路径都成立：冷启动直接在该目录开终端；已有实例在跑时，第二个进程把目录经单实例套接字转交，既有窗口新开一个本地终端标签。

关闭开关只摘除我们自己的偏好行、把桌面条目改名为 `.disabled`（可逆，不销毁用户数据）并删除图标；不"恢复注册前的默认终端"——规范里没有旧优先级快照，伪造恢复反而危险。

`ssh://` 的注册在不使用开关时仍留给手动步骤：

```bash
mkdir -p ~/.local/share/applications ~/.local/share/icons/hicolor/scalable/apps
cp packaging/linux/com.github.mangerle.transom.desktop ~/.local/share/applications
cp logo/transom-logo.svg ~/.local/share/icons/hicolor/scalable/apps/com.github.mangerle.transom.svg
update-desktop-database ~/.local/share/applications
xdg-mime default com.github.mangerle.transom.desktop x-scheme-handler/ssh   # 可选：设为默认
```

深链会在既有窗口里新开一个标签并直接连接，不保存会话。唤起时 transom 会同时请求两级激活：平台级 `App::activate`（Linux 只有日志，真正生效的是 macOS/Windows）与窗口级 `activate_window`——X11 后端发 `_NET_ACTIVE_WINDOW`，Wayland 后端经 `xdg_activation` 申请 token。焦点归属由合成器裁定，效果按平台分别核实：Ubuntu GNOME 的 Wayland 会话实测能把已在运行的窗口调到前台。窗口可以最小化（`Window::minimize_window`，标题栏按钮就用它）；Wayland 没有"取消最小化"的原语（`xdg_toplevel.set_minimized` 是单向请求），实测恢复走的就是上面同一个激活请求——已最小化的窗口能被重新调到前台。X11 侧发 `_NET_ACTIVE_WINDOW`，未在本机单独实测。

## 源码构建

前置条件：Rust **1.96 或更新**（edition 2024），以及可供 GPUI 平台层使用的桌面会话。

```bash
git clone https://github.com/mangerle/transom.git
cd transom

cargo run --bin transom            # 开发构建
cargo build --release --bin transom  # 产物位于 target/release/transom
```

Release 构建剥离符号表并启用 thin LTO。日志经 `log` 门面落到 `~/.config/transom/logs/transom.log`，按大小轮转（单文件 5 MiB，另留 4 份），崩溃后记录仍在，可以直接附到 issue 里。级别在「偏好设置 → 通用 → 运行日志」里调，改完立即生效、无需重启；也可以用启动时的 `TRANSOM_LOG=debug` 或 `RUST_LOG=debug ./target/release/transom` 覆盖，环境变量优先于配置值。默认级别为 `info`；标准错误输出只镜像警告及以上，从终端拉起时不会被细节刷屏。

## 发布与自动更新

推送 `v*` 标签会触发 `.github/workflows/release.yml`，为 Linux、Windows、macOS 的 x86_64 与 arm64 构建并发布 GitHub Release：

| 平台 | 更新通道 | 手动分发产物 |
| :--- | :--- | :--- |
| Linux | `.deb` 安装器 | `.AppImage` |
| Windows | Authenticode 签名的 NSIS 每用户安装包 | 内含已签名主程序的绿色便携 `.zip` |
| macOS | 内含已签名可执行文件的 `.zip`（由 shipup 解压后替换 `.app` 内可执行文件） | Developer ID 签名、Apple 公证并 stapling 的 `.dmg` |

发布任务先用 Ed25519 对每个更新包与合并后的 `latest.json` 签名，再应用平台原生签名：Windows 使用带 RFC 3161 时间戳的 Authenticode，macOS 使用 Developer ID、Hardened Runtime、Apple 公证与 stapling。全部签名凭据均为必需的 GitHub Secrets；任一缺失时发布前门禁会直接拒绝构建。应用内「偏好设置 → 关于 → 软件更新」负责检查清单、下载验签、安装与优雅重启；连续两次启动失败会触发上一版本回滚。

密钥管理、产物矩阵、本地验证命令与三端更新路径见 [`docs/release.md`](./docs/release.md)。

### 数据存放位置

全部位于用户配置目录（Linux 为 `~/.config/transom`，macOS 为 `~/Library/Application Support/transom`，Windows 为对应已知文件夹）：

| 文件 | 内容 |
| :--- | :--- |
| `sessions.json` | 会话树与连接配置。 |
| `settings.json` | 偏好设置、主题、字体、快捷键与快捷命令。 |
| `known_hosts.json` | 主机密钥指纹（首次信任）。 |
| `logs/` | 应用运行日志，按大小轮转。不含口令与私钥材料；`debug` 与 `trace` 档会记录主机名、路径与命令行。 |
| `audit/` | 可选的每连接原始输出留档，默认关闭。 |
| 系统钥匙串，服务名 `transom` | 口令与私钥 passphrase，绝不落盘。 |

密钥向导生成的密钥对默认写入 `~/.ssh`。

## 工程结构

```
apps/transom            可执行入口：装配存储层与异步连接控制器
crates/transom-core     存储契约、会话/设置模型、命令事件契约与转发/高亮规则
crates/transom-platform 用户目录、私有文件权限与发行版识别
crates/transom-crypto   TSYN 二进制与 JSON/YAML 加密信封
crates/transom-desktop  平台桌面集成：Linux 默认终端、macOS/Windows URI 注册
crates/transom-net      russh 客户端、SFTP、端口转发、串口与 PTY 链路
crates/transom-sync     WebDAV/S3/Git 云同步编排
crates/transom-store    JSON 存储、钥匙串、加密备份与原子写
crates/transom-terminal alacritty_terminal 网格、渲染器、检索与 OSC 解析
crates/transom-zmodem   ZMODEM 协议状态机与终端挂载层
crates/transom-updater  shipup 封装：签名清单检查、下载、安装与崩溃回滚
crates/transom-ui       GPUI 应用、侧边栏、标签、分屏、SFTP 面板、设置与 i18n
```

约 9 万行 Rust 代码，含 600+ 单元与集成测试。

## 开发

```bash
cargo fmt --all
cargo clippy --all-targets -- -D warnings   # 零警告门禁
cargo check --all-targets
cargo test
```

仓库根目录的 `AGENTS.md` 记录了本工程遵循的工程约束：禁用无界通道、生产路径禁止裸 `unwrap()`、异步运行时内不得阻塞、队列必须有界且显式声明背压策略、大文件传输必须流式 I/O。

## 路线图

[`docs/FEATURES_ROADMAP.md`](./docs/FEATURES_ROADMAP.md) 逐项标注了 117 项评估过的能力及其完成状态，清单内项目均已实现或由终端内核原生支持。清单之外仍有运行前提：Windows 的 WSL 能力依赖外部 WSL 工具，Linux 默认终端接管基于 XDG 且按平台设计不适用于 macOS/Windows，macOS 自更新只替换可执行文件。

## 参与贡献

欢迎 Issue 与 Pull Request。较大的改动请先开 Issue 对齐范围。提交信息请遵循 Conventional Commits，中文或英文描述均可。

## 开源协议

本项目采用 **Apache License, Version 2.0** 授权，详见 [`LICENSE`](./LICENSE)。第三方依赖各自遵循其原始协议，主要包括 GPUI（MIT/Apache-2.0）、russh（MIT）、russh-sftp（MIT）、alacritty_terminal（Apache-2.0）。
