# 架构说明（Architecture）

本文描述 Transom 当前的模块边界、依赖方向与关键数据流，以及若干"为什么
这样设计"的决策记录。README 面向使用者，本文面向改动代码的人。

## 一句话定位

单进程桌面应用：GPUI 负责界面渲染，一个 tokio 控制平面负责全部连接与
后台任务，两者之间只通过三条有界通道通信；终端字节流不走事件总线，
由终端视图独占消费。

## crate 与依赖方向

```
apps/transom (组合根 + 控制平面)
  ├─ transom-ui (GPUI 界面与全部应用状态)
  │    ├─ transom-terminal (VTE 网格 / 渲染 / 高亮引擎 / 检索 / OSC / Kitty)
  │    │    └─ alacritty_terminal
  │    ├─ transom-store (JSON 存储 / 钥匙串 / 备份文件读写)
  │    │    ├─ transom-crypto
  │    │    └─ transom-core
  │    ├─ transom-desktop (默认终端 / 桌面条目 / 启动器集成)
  │    │    ├─ transom-platform
  │    │    └─ transom-core
  │    ├─ transom-platform (用户目录 / 私有文件权限 / 发行版识别)
  │    └─ transom-core
  ├─ transom-updater (shipup 封装：签名清单 / 下载验签 / 崩溃回滚)
  ├─ transom-net (SSH / SFTP / 转发 / 串口 / Telnet / 统计解析)
  │    └─ transom-core
  ├─ transom-container (容器引擎发现 / Docker 兼容 REST / exec 桥 / 归档文件)
  │    └─ transom-core
  ├─ transom-zmodem (ZMODEM 协议状态机与终端挂载层)
  │    └─ transom-core
  ├─ transom-sync (云同步 WebDAV / S3 / Git 编排)
  │    ├─ transom-crypto
  │    ├─ transom-store
  │    ├─ transom-platform
  │    └─ transom-core
  ├─ transom-store
  │    ├─ transom-crypto
  │    ├─ transom-platform
  │    └─ transom-core
  ├─ transom-terminal
  │    └─ transom-platform
  └─ transom-crypto (TSYN 二进制信封与 JSON/YAML 加密信封)
       └─ transom-core

全部 crate ──> transom-core（契约层：领域模型、协议契约、命令事件总线、错误定义）
需要本地目录或私有权限的 crate ──> transom-platform（宿主平台约定）
```

规则：**依赖只向内**。`transom-net` 不依赖 `transom-store`；`transom-sync` 依赖 `transom-store` 与 `transom-crypto`；加解密信封由独立的 `transom-crypto` 提供；UI 不直接做网络协议；与操作系统打交道的路径与权限经 `transom-platform`，桌面注册经 `transom-desktop`。

自更新链路同样遵守该方向：`transom-updater` 只封装 `shipup` 并暴露稳定的信息/事件模型，不依赖 UI；`transom-ui` 的“关于”页在 `SettingsView` 内维护互斥状态机，检查/下载/安装经 `spawn_blocking_local` 执行，下载进度走容量 1 的有界通道；安装完成后的重启意图经 `SettingsEvent::RestartForUpdate` 交回 `AppShell`，外壳置位 `AppState::restart_requested` 并退出事件循环，`transom_ui::run` 把该标志返回给 `main`，确保控制器优雅停机后才拉起新进程。

## UI 内部结构

`transom-ui` 的功能模块通过 `#[path]` 挂在 `app` 模块之下，`AppShell` 的字段只对 `app` 及其后代可见，crate 其它模块无法绕过外壳直接改状态。应用状态集中在 `app::state::AppState`，`AppShell` 只保留一个 `state` 字段；状态访问一律显式写 `self.state.xxx`，不用 `Deref` 继承方法。

可复用的表现层组件在 `transom-ui/src/widgets/`：图标按钮、弹窗动作按钮与动作条、设置/表单行、空状态、菜单项（含禁用态）、列表行与表头、状态徽标、工具栏。组件只接收数据与回调，不依赖 `AppShell`、不直接访问 store；字号、间距、控件尺寸与弹窗宽度统一取 `theme::tokens`。

方法是按“是否需要 GPUI 上下文”划分的：不依赖 `Window`/`Entity`/`Context` 的状态查询与变更写在 `impl AppState`，其余渲染、事件路由与生命周期入口留在 `AppShell`。抽取时优先判断能否落到 `AppState`，再考虑是否真的需要外壳。

三条回归闸门固化上述约定：`app_shell_keeps_single_state_field` 禁止外壳重新平铺字段；`widgets_use_design_tokens` 禁止组件层写裸字号与间距；`menu_items_use_widget_constructor` 禁止手写菜单项链。

新增功能时：状态放 `AppState` 或对应 feature 实体，行为与状态同处；纯展示结构先找 `widgets`，不要继续往 `AppShell` 上堆固有方法。新增模态弹窗时，先在 `dialog_host/registry` 的 `MODALS` 表登记一行（层序即表序），再补 `checks` 的打开判定与文本项函数；渲染、取消与提交仍按领域分发。

### 弹窗注册表

`dialog_host/registry` 是弹窗元数据的唯一来源：

```
ModalEntry { kind, slug, width, is_open, field_values }
MODALS: &[ModalEntry]   // 顺序即层叠顺序
```

`ModalWidth` 分三种形态：`Token(rem 档位)`、`Max`（全局上限）、`Viewport(比例)`
（差异对比专用）。新增弹窗的改动点收敛为“枚举加变体、注册表加一行、checks
补函数”，不再同时修改 `ORDER`、`spec`、`is_open`、`field_values` 四处大 `match`。
守卫测试 `registry_covers_every_kind_exactly_once` 保证表与枚举不漏不重。

### 面板实体化

面板独立为 `Entity<面板>`，外壳只持有句柄；状态、动作与渲染都归实体：

```
AppShell { monitor: Entity<MonitorView>, editors: Entity<EditorPanels>, ... }
MonitorView  { state: MonitorState }          // 不持有外壳句柄
EditorPanels { manager: EditorManager }       // 不持有外壳句柄
SettingsView { state: SettingsModalState, services: SettingsServices, ... }
SftpPanelView { state: SftpPanelState, tab_id: TabId }   // 每窗格一份
impl Render for 面板                            →  面板自己渲染
impl 面板                                       →  面板自己的动作
```

约定：

- 面板**不得**持有 `WeakEntity<AppShell>` 等父实体句柄，避免双向引用与双重租借；
- 数据向下：外壳在变更点调用 `view.update(|v, cx| v.set_*(...))` 注入；
- 意图向上：面板 `cx.emit(PanelEvent)`，事件自带全部数据，外壳订阅一次并消费；
- 需要 `Window` 的动作排进 `window_requests`，由外壳在渲染期统一执行；
- 外壳处理实体事件期间不得回读该实体，否则触发
  “cannot read/update ... while it is already being updated”。

已完成实体：监控面板（`MonitorView`）、编辑器面板（`EditorPanels`）、SFTP 面板
（`SftpPanelView`）、设置面板（`SettingsView`）、已知主机弹窗（`KnownHostsView`）、
备份弹窗（`BackupPromptView`：口令校验、文件选择与后台任务归实体，目录合并与
状态栏仍由外壳消费事件完成）、审计查看器（`AuditViewerView`：扫描、选中、删除、
复制与渲染归实体，状态栏与“打开设置”由外壳消费事件）、密钥向导
（`KeyWizardView`：生成任务、公钥清单、复制/导出/部署与渲染归实体，状态栏与
部署回执由外壳消费事件）、回放选择弹窗（`ReplayModalView`：录制目录扫描、
选择与渲染归实体，打开播放器/终端回放由外壳消费事件）、会话表单
（`SessionFormView`：表单状态、校验、保存/删除意图与渲染归实体，目录/钥匙串/
串口由外壳消费事件）。

容器与 Git 的**每标签页状态**已收进 `Entity<ContainerWorkspace>` /
`Entity<GitWorkspace>`：标签内容只存句柄，外壳读写都经 `read`/`update`。

容器侧的在途请求登记、命令跟踪投递、列表刷新（防抖/单分类/全量预热）、
选择项、生命周期与清理动作已迁入 `impl ContainerWorkspace`；请求按标签归属，
事件用 `request_id` 在实体上反查标签，迟到结果自然丢弃。渲染编排仍在
`AppShell`，通过 `with_container` 迁移期桥只读状态，下一步是迁渲染并删除桥。

Git 侧已迁 `GitWorkspace::load_log` 与实体命令通道；其余 Git 动作仍在外壳。

## 数据流

### 终端字节流（热路径，不走事件总线）

```
SSH 通道 / PTY / 串口 / TCP
  └─ TerminalIo.reader（含 ZMODEM 监督层）
       └─ 读线程（每块 64 KiB）
            └─ 有界通道（32 深）
                 └─ TerminalView 读任务：engine.process(bytes) → cx.notify()
```

终端输出的每一块只经过"过滤层拷贝 → 通道 → 引擎"，不进入 AppEvent
总线，避免把高频字节流与低频控制事件混在同一队列。渲染侧按损伤集合
（damage）只重画变化行。

### 控制面（低频，三条有界通道）

```
UI（try_send，满则提示）
  └─ 命令通道 256 ──> 控制平面主循环
                          ├─ commands.rs：连接 / SFTP / 转发 / 云同步 / 进程 / 停机
                          ├─ internal/：连接接入 / 断开 / SFTP-转发提交（按事件类型分成子模块）
                          └─ state.rs：ControllerState（全部状态表）
  └─ 事件通道 1024 <── 控制平面（状态迁移用 send_async 反压；可覆盖样本 try_send）
  └─ 内部通道 64   （建连任务与主循环之间的完成/断开通知）
```

背压口径：可覆盖状态样本（流量、系统统计、SFTP 进度）由生产者
`try_send`，通道满即丢弃、不占位；状态迁移事件（连接建立/断开、转发
变更、失败）保持 `send_async` 反压，UI 停滞时拖慢发送方而非内存无限
积压。

### 连接接入（单一路径）

```
AppCommand::Connect
  └─ pty::dispatch_connect 派生建连任务（SSH 握手 / PTY / 串口 / Telnet）
       └─ Internal::Connected { kind: ReadyConnection, zmodem }
            └─ internal::handle_connected
                 ├─ admit_connection：与"最新连接意图"比对，过期结果拒收
                 └─ adopt_stream_connection：登记连接、停旧任务、广播 TerminalReady
```

新增连接类型 = 扩展 `ReadyConnection` 一个变体 + 差异化的收尾步骤，
不再复制整段接入逻辑（历史上四类连接各有一份复制）。

### 存储（原子写 + 异步落盘）

- 会话/设置：内存态立即生效，`app::persistence::SnapshotWriter` 以覆盖式
  有界单槽在后台线程写盘（唯一临时名 → 写入 → `sync_all` → rename），
  Drop 时停机并 join；退出路径仍同步保存窗口状态，保证最后一次快照落定；
- 历史：`file_io::write_atomic`（唯一临时名 → 写入 → `sync_all` → rename）；
- known_hosts：内存态权威，常驻写盘线程异步落盘（覆盖式单槽，Drop 时
  join），`record` 返回 Ok 表示"已进入内存与待写槽"而非"已落盘"；
- 凭据（口令/口令短语）：只进系统钥匙串，文件里只有 `save: bool` 开关；
- 备份/同步文档：两套信封（`transom-crypto::envelope` 的 TSYN 二进制对象、
  `transom-crypto::backup` 的 JSON/YAML 内嵌字段），Argon2id + XChaCha20-Poly1305。

### Git 配置

Git 可执行文件路径与提交作者是进程级配置，只能由控制平面写入：界面在启动与
设置保存时投递 `GitCommand::Configure`，控制器调用 `transom-git` 的配置入口。
界面源码不得直接调用 `set_git_program` / `set_git_author`（守卫：
`git_config_is_controller_owned`），避免“谁能改全局状态”不可追踪。

### Git 搁置项（Shelf）

搁置项不再由界面直接操作：`GitCommand::{LoadShelves, Shelve, ApplyShelf, DropShelf}`
经命令总线交给控制器，补丁导出/应用/工作区丢弃走 `transom-git`，补丁列表的
读写走 `transom-store` 且统一 `spawn_blocking`；结果经 `GitAppEvent::Shelf*`
回到界面。`Shelve` 的顺序是“导出补丁 → 落盘条目 → 丢弃工作区”，落盘失败
时保留工作区改动，避免不可恢复的数据丢失。

## 目录结构（2026-10 整理后）

```
crates/transom-core/src/
  model/     领域模型与算法：session / settings / catalog / merge / appearance ...
  protocol/  线格式与契约：ssh_config / ssh_url / sftp / sync / system_stats(types) ...
  根        commands（命令/事件）、error、ids、prompt、secrets、storage、backup、transport（终端字节流契约）
  注：模块路径即规范路径，不再保留历史别名；
  仅界面队列使用的 TransferTask 系列模型在 transom-ui/sftp/task.rs。

crates/transom-platform/src/
  paths.rs       用户目录与文件命名约定
  private_fs.rs  私有目录 0700 / 私有文件 0600 的创建与收紧
  os_release.rs  /etc/os-release 发行版识别

crates/transom-crypto/src/
  envelope.rs  TSYN 二进制端到端加密信封
  backup.rs    JSON/YAML 内嵌加解密信封

crates/transom-desktop/src/
  linux/       freedesktop 默认终端注册、桌面条目与图标安装
  unsupported.rs  macOS/Windows 同名入口的占位实现

crates/transom-terminal/src/
  highlight/ 关键词高亮扫描与编译规则引擎

crates/transom-ui/src/
  app/        AppShell、AppState、持久化、事件分发与各功能模块
  widgets/    跨页面复用的表现层组件
  theme/      主题、设计令牌与外观解析
  palette.rs  主题调色板快照与本地化辅助

crates/transom-zmodem/src/
  frames / scanner / receiver / sender / fsm / attach：字节级协议与会话驱动

crates/transom-updater/src/
  lib.rs  shipup 封装：签名清单检查、下载验签、安装、崩溃回滚与重启入口

crates/transom-net/src/
  system_stats/ Linux 宿主与远端资源采样输出解析器与状态机

crates/transom-container/src/
  api/          Docker/Podman REST 客户端：按资源域拆分（容器/镜像/卷/网络/系统/exec）
  models/       线格式结构体：按资源域拆分，转换逻辑随类型就近放置
  cli/          CLI 兜底后端：按子命令拆分，共享进程执行管线留在 mod.rs
  files/        容器文件传输：read / write / archive / listing 分层
  engine/       引擎实例：mod.rs 核心与分派宏、operations.rs 操作面、
                discovery.rs 发现/显式探测
  discovery/    候选发现：parse/unix/cli/windows 分平台
  registry/     仓库检索与鉴权：search/auth/tags/io
  volume_files/ 数据卷文件访问：probe/backup/直接读写
  transport.rs / http/（请求入口、连接池、响应体读取）
  build/（ignore/stream/http/cli）/ logs.rs / exec.rs
                通道、归档与长任务

apps/transom/src/controller/
  mod.rs       接线与主循环（约 300 行：收事件、委托处理、判定停机）
  state.rs     ControllerState（全部状态表）与 ControllerCtx（共享句柄）
  commands.rs  用户命令处理
  internal/    内部完成事件处理与单一接入路径：
               mod.rs 只做分发，connected / disconnected / sftp / forward / container
               各处理一类事件
  lifecycle.rs 连接接入/清理/停机的公共流程
  pty.rs / samplers/ / sftp_exec/（dispatch + 按域拆分的 ops）/ sftp_finish.rs / sync_exec.rs / host_exec.rs
```

## 关键决策记录

| 决策 | 理由 | 代价 |
| :--- | :--- | :--- |
| 终端字节绕开事件总线 | 高频字节流与低频控制事件隔离 | 终端视图自己管读取任务与背压 |
| 可覆盖样本 try_send、迁移事件反压 | 样本丢了下一轮补，迁移事件不能丢 | UI 停滞时采样频率下降（可自愈） |
| 加密信封独立为专用 crate（transom-crypto） | net/sync 不再反向依赖 store，同时 core 不再背负加解密重依赖 | 多维护一个轻量 crate |
| 平台约定独立为 transom-platform | 路径与权限被存储/终端/同步/界面共同使用，留在 core 会把它变成 IO 实现层 | 多维护一个轻量 crate |
| ZMODEM 独立为 transom-zmodem | 协议只依赖终端 IO 契约，抽离后 net 不再背负 4 千行编解码与真机测试 | 多维护一个轻量 crate |
| 桌面集成独立为 transom-desktop | 存储层只做持久化；启动器/注册表逻辑有独立平台边界，macOS/Windows 可分别扩展 | 多维护一个轻量 crate；非 Linux 暂为占位实现 |
| 连接参数随命令携带（ZMODEM/SHA1 策略） | 控制循环零同步 IO，磁盘慢不拖控制平面 | 命令载荷变宽，UI 有责任携带最新快照 |
| known_hosts 异步落盘 | 握手路径不带写盘 IO | record 成功 ≠ 已落盘；极端崩溃丢最后若干条 |
| 会话/设置覆盖式异步写盘 | UI 点击不承受 fsync；单槽天然幂等，内存不会积压 | 成功入队 ≠ 已落盘；后台失败由渲染帧提示 |
| 原子写必带 fsync | rename 只保证"旧或新"，不保证已持久 | 每次保存多一次 fsync（毫秒级） |
| 单实例 + 深链 socket（0600/0700） | 二次启动转交请求，不堆进程 | 多实例并行调试需伪装环境变量 |
| 统计采样按机器共享（本地标签） | 标签一多避免 N 倍 /proc 与 df | 新标签等下一轮（3-5 秒）才见数据 |
| 远端统计 5 秒、df 每 3 轮 | 降低远端负载与子进程开销 | 磁盘指标最多陈 15 秒 |

## 改动指引

- **新增一条 AppCommand**：`crates/transom-core/src/commands/mod.rs` 加变体 → `controller/commands.rs`
  加处理分支（UI 侧构造点按其语义选择 try_send/send_command）；
- **新增连接类型**：`ReadyConnection` 加变体 → `internal/connected.rs` 的 `adopt_*` 加差异化收尾
  → `pty.rs` 的派发点构造新变体；
- **新增持久化文件**：走 `file_io::write_atomic`（禁止自写临时文件流程）；
- **新增周期任务**：必须持有 `JoinHandle` 并纳入 `ControllerState` 的对应
  表（断开/关标签/停机三处都能被清理），通道一律有界。