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
  └─ transom-ui (GPUI 界面与全部应用状态)
       └─ transom-terminal (VTE 网格 / 渲染 / 检索 / OSC / Kitty)
            └─ alacritty_terminal
       └─ transom-store (JSON 存储 / 钥匙串 / 备份文件读写 / 桌面集成)
            └─ transom-core
       └─ transom-core

apps/transom ──> transom-net (SSH / SFTP / 转发 / 串口 / Telnet / 云同步 / ZMODEM)
  └─ transom-net ──> transom-core（含 crypto 信封）
  └─ transom-store ──> transom-core

全部 crate ──> transom-core（契约层，唯一被所有人依赖的底座）
```

规则：**依赖只向内**。`transom-net` 不依赖 `transom-store`（加密信封上提
到 `transom-core::crypto` 之后成立，`cargo tree -p transom-net --edges normal`
可核验）；`transom-store` 不依赖 `transom-net`；UI 不直接做网络协议。

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
                          ├─ internal.rs：连接接入 / 断开 / SFTP-转发提交
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

- 会话/设置/历史：`file_io::write_atomic`（唯一临时名 → 写入 → `sync_all`
  → rename），均在工作区确认后写出；
- known_hosts：内存态权威，常驻写盘线程异步落盘（覆盖式单槽，Drop 时
  join），`record` 返回 Ok 表示"已进入内存与待写槽"而非"已落盘"；
- 凭据（口令/口令短语）：只进系统钥匙串，文件里只有 `save: bool` 开关；
- 备份/同步文档：两套信封（`crypto::envelope` 的 TSYN 二进制对象、
  `crypto::backup` 的 JSON/YAML 内嵌字段），Argon2id + XChaCha20-Poly1305。

## 目录结构（2026-10 整理后）

```
crates/transom-core/src/
  model/     领域模型与算法：session / settings / catalog / appearance / keys ...
  protocol/  线格式与同步：ssh_config / ssh_url / sftp / sync / system_stats ...
  platform/  用户目录与私有文件权限：paths / private_fs
  crypto/    加解密信封：envelope（TSYN）/ backup（JSON 信封）
  根        commands（命令/事件）、error、ids、prompt、secrets、storage、backup
  注：历史路径（crate::session 等）经 lib.rs 别名 re-export 保持可用。

apps/transom/src/controller/
  mod.rs       接线与主循环（约 300 行：收事件、委托处理、判定停机）
  state.rs     ControllerState（全部状态表）与 ControllerCtx（共享句柄）
  commands.rs  用户命令处理
  internal.rs  内部完成事件处理与单一接入路径
  lifecycle.rs 连接接入/清理/停机的公共流程
  pty.rs / samplers.rs / sftp_exec.rs / sftp_finish.rs / sync_exec.rs / host_exec.rs
```

## 关键决策记录

| 决策 | 理由 | 代价 |
| :--- | :--- | :--- |
| 终端字节绕开事件总线 | 高频字节流与低频控制事件隔离 | 终端视图自己管读取任务与背压 |
| 可覆盖样本 try_send、迁移事件反压 | 样本丢了下一轮补，迁移事件不能丢 | UI 停滞时采样频率下降（可自愈） |
| 加密信封放契约层（core::crypto） | net 不再反向依赖 store；备份与云同步共用同一线格式 | core 依赖面变宽（加密四件套） |
| 连接参数随命令携带（ZMODEM/SHA1 策略） | 控制循环零同步 IO，磁盘慢不拖控制平面 | 命令载荷变宽，UI 有责任携带最新快照 |
| known_hosts 异步落盘 | 握手路径不带写盘 IO | record 成功 ≠ 已落盘；极端崩溃丢最后若干条 |
| 原子写必带 fsync | rename 只保证"旧或新"，不保证已持久 | 每次保存多一次 fsync（毫秒级） |
| 单实例 + 深链 socket（0600/0700） | 二次启动转交请求，不堆进程 | 多实例并行调试需伪装环境变量 |
| 统计采样按机器共享（本地标签） | 标签一多避免 N 倍 /proc 与 df | 新标签等下一轮（3-5 秒）才见数据 |
| 远端统计 5 秒、df 每 3 轮 | 降低远端负载与子进程开销 | 磁盘指标最多陈 15 秒 |

## 改动指引

- **新增一条 AppCommand**：`core/commands.rs` 加变体 → `controller/commands.rs`
  加处理分支（UI 侧构造点按其语义选择 try_send/send_command）；
- **新增连接类型**：`ReadyConnection` 加变体 → `internal.rs` 加差异化收尾
  → `pty.rs` 的派发点构造新变体；
- **新增持久化文件**：走 `file_io::write_atomic`（禁止自写临时文件流程）；
- **新增周期任务**：必须持有 `JoinHandle` 并纳入 `ControllerState` 的对应
  表（断开/关标签/停机三处都能被清理），通道一律有界。