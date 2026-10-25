<div align="center">
  <img src="./logo/transom-lockup.svg" alt="Transom" width="420" />

  <p>
    <a href="./README.md"><strong>English</strong></a> ·
    <a href="./README_zh-CN.md"><strong>简体中文</strong></a>
  </p>

  <p>
    <a href="https://github.com/mangerle/transom/blob/main/LICENSE"><img alt="License" src="https://img.shields.io/badge/license-Apache--2.0-blue" /></a>
    <img alt="Rust" src="https://img.shields.io/badge/rust-1.92%2B-orange" />
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
- 连接类型：SSH2（密码、键盘交互、公钥、OpenSSH CA 用户证书、Unix 下支持 `ssh-agent`）、本地 PTY Shell、以及面向嵌入式板卡与交换机管理口的串口连接。
- 只保留现代加密套件：AES-128/256-GCM 与 ChaCha20-Poly1305、SHA2-512 MAC、`zlib` 传输压缩，密钥支持 RSA / ECDSA / Ed25519。
- 连接层默认每 30 秒心跳、连续 3 次无应答断开，避免云厂商防火墙静默掐断空闲连接。
- 启动时自动恢复并重连上次打开的会话；会话库可导出/导入为明文 JSON/YAML，或导出为 Argon2id + XChaCha20-Poly1305 加密备份。
- 快速打开面板（`$app-k`）兼作免保存直连：输入 `deploy@10.0.0.8:2222` 即可直接发起连接，这条会话不会进入会话列表。

### 终端
- 标签内自由分屏：切分方向属于每一刀、可嵌套，分隔线可拖拽，支持单窗格最大化与拆出为独立标签。
- 缓冲区全文检索带正则开关，右侧刻度带显示整块滚动缓冲区的命中分布并支持点击跳转。
- 常驻高亮 `ERROR` / `WARN` / `INFO` / `DEBUG`、IPv4 与 UUID，另可加最多 32 个自定义关键词；颜色全部取自主题调色板，跟随配色切换。
- <kbd>Alt</kbd>+拖拽矩形列选（在 `vim`、`htop` 里同样可用）、双击选词分隔符可自定义、命令耗时超过阈值时通过 OSC 133 / OSC 777 发出桌面通知。
- 快捷命令条（Snippets）把常用单行命令填充到焦点窗格。

### 端口转发
- 对运行中的会话随时启停本地、远程与动态（SOCKS5）隧道，并有独立面板查看规则、连接数与通道状态。

### SFTP
- 本地/远程双栏文件浏览器，支持断点续传、传输队列置顶插队、失败一键重试与已完成清空，逐任务显示速率与百分比。
- 按大小与修改时间做目录差异对比与勾选式增量同步；传输后保留源文件 mtime，避免重复回传。
- 可选的传输后 SHA-256 校验（两端均按 64 KiB 分块流式计算并比对），以及上传目录时按通配符剪枝 `.git/`、`node_modules/` 的排除规则。
- 内置远程编辑器：只读保护、保存前统一 diff 预览、写回前自动备份；`chmod` 提供 9 宫格与八进制两种输入；图片与文本首数十行悬停预览；`Ctrl+F` 文件名即时过滤。

### 界面
- 内置 6 套主题（One Dark、Dracula、Monokai Pro、Nord、Solarized Dark、Gruvbox Dark），客户端窗口控制三键支持拖动与双击缩放，记忆窗口尺寸与位置，提供沉浸式全屏。
- 界面中英双语，可在设置中切换——本 README 的双语结构与应用一致。
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

| 平台 | 状态 |
| :--- | :--- |
| Linux（Wayland / X11） | 主要开发与验证平台。 |
| macOS、Windows | 已有跨平台代码路径（修饰键映射、钥匙串后端、在文件管理器中显示）并可编译，但**尚未经 CI 或发布产物验证**，请按实验性看待。 |

`ssh-agent` 认证目前仅支持 Unix；终端超链接通过 `xdg-open` 打开。

### 桌面集成（`ssh://` 链接）

Linux 上 Transom 可以接管浏览器或其他工具发出的 `ssh://[user@]host[:port]` 链接，并以单实例方式运行：第二次启动会把地址通过 `$XDG_RUNTIME_DIR/transom` 下的私有套接字交给已在运行的窗口，然后立即退出。

注册这一步刻意留给用户手动执行——多数发行版已把 `ssh://` 绑给某个终端模拟器，应用私自改写默认处理程序会影响到本项目之外的行为：

```bash
mkdir -p ~/.local/share/applications
cp packaging/linux/com.github.mangerle.transom.desktop ~/.local/share/applications
update-desktop-database ~/.local/share/applications
xdg-mime default com.github.mangerle.transom.desktop x-scheme-handler/ssh   # 可选：设为默认
```

深链会在既有窗口里新开一个标签并直接连接，不保存会话。把已最小化的窗口提到前台**尚未实现**：GPUI 在 Linux 没有提供窗口 raise/焦点接口。

## 源码构建

前置条件：Rust **1.92 或更新**（edition 2024），以及可供 GPUI 平台层使用的桌面会话。

```bash
git clone https://github.com/mangerle/transom.git
cd transom

cargo run --bin transom            # 开发构建
cargo build --release --bin transom  # 产物位于 target/release/transom
```

Release 构建剥离符号表并启用 thin LTO。日志走 `tracing` 的 `env-filter`，`RUST_LOG=debug ./target/release/transom` 可提高详细度，默认级别为 `info`。

### 数据存放位置

全部位于用户配置目录（Linux 为 `~/.config/transom`，macOS 为 `~/Library/Application Support/transom`，Windows 为对应已知文件夹）：

| 文件 | 内容 |
| :--- | :--- |
| `sessions.json` | 会话树与连接配置。 |
| `settings.json` | 偏好设置、主题、字体、快捷键与快捷命令。 |
| `known_hosts.json` | 主机密钥指纹（首次信任）。 |
| 系统钥匙串，服务名 `transom` | 口令与私钥 passphrase，绝不落盘。 |

密钥向导生成的密钥对默认写入 `~/.ssh`。

## 工程结构

```
apps/transom            可执行入口：装配存储层与异步连接控制器
crates/transom-core     存储契约、会话/设置模型、转发与高亮规则
crates/transom-net      russh 客户端、SFTP、端口转发、串口与 PTY 链路
crates/transom-store    JSON 存储、钥匙串、Argon2id/XChaCha20 加密备份、原子写
crates/transom-terminal alacritty_terminal 网格、渲染器、检索与 OSC 解析
crates/transom-ui       GPUI 应用、侧边栏、标签、分屏、SFTP 面板、设置与 i18n
```

约 4.7 万行 Rust 代码，含 260+ 单元与集成测试。

## 开发

```bash
cargo fmt --all
cargo clippy --all-targets -- -D warnings   # 零警告门禁
cargo check --all-targets
cargo test
```

仓库根目录的 `AGENTS.md` 记录了本工程遵循的工程约束：禁用无界通道、生产路径禁止裸 `unwrap()`、异步运行时内不得阻塞、队列必须有界且显式声明背压策略、大文件传输必须流式 I/O。

## 路线图

[`docs/FEATURES_ROADMAP.md`](./docs/FEATURES_ROADMAP.md) 逐项标注了 116 项评估过的能力及其完成状态。推进中的项目：跳板机 ProxyJump 链式隧道、跨窗格广播输入、撰写窗格、以及会话环境色标识。已明确记录的缺口：SOCKS5 目前仅支持无认证代理，且尚未提供签名安装包。

## 参与贡献

欢迎 Issue 与 Pull Request。较大的改动请先开 Issue 对齐范围。提交信息请遵循 Conventional Commits，中文或英文描述均可。

## 开源协议

本项目采用 **Apache License, Version 2.0** 授权，详见 [`LICENSE`](./LICENSE)。第三方依赖各自遵循其原始协议，主要包括 GPUI（MIT/Apache-2.0）、russh（MIT）、russh-sftp（MIT）、alacritty_terminal（Apache-2.0）。
