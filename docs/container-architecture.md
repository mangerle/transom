# 容器管理工作区架构（Docker / Podman）

本文描述 Transom 的容器管理功能：连接一个容器引擎后，进入类似 Docker
Desktop 的**管理工作区**——左侧功能菜单（编排 / 容器 / 镜像 / 数据卷 /
网络），中间是当前分类的列表，右侧是选中项的详情与操作页签。实现继续
遵循现有规则：依赖只向内、控制器独占网络与子进程、通道有界、终端字节
流不走事件总线。

## 1. 定位与入口

- **容器表单**：与会话表单并列的一类表单，用于新建/编辑“引擎连接”，
  可保存多个引擎（本机 Docker、本机 Podman、自定义 socket、命名管道、
  远端端点、WSL 发行版内 Docker、WSLC）。
- **容器侧栏**：活动栏新增「容器」入口，侧栏列出已保存的引擎连接；
  点击一个连接即打开（或聚焦）该引擎的管理工作区标签。
- **管理工作区**：一个特殊标签页，承载三栏界面与全屏模态。
- 引擎连接的在线状态、标签页生命周期与终端会话解耦：关闭工作区标签
  即断开该引擎的订阅与 exec 会话，但不会停止容器。

## 2. 工作区页面结构

```
┌ 菜单（编排/容器/镜像/数据卷/网络）┬ 列表（当前分类）┬ 详情（选中项）┐
│ 编排  ● 3 个项目                 │ web   running  │ 基础信息 | 日志 |   │
│ 容器  ● 12 个容器                │ db    exited   │ 终端 | 统计 | 文件   │
│ 镜像  ● 8 个镜像                 │ cache running  │                    │
│ 数据卷 ● 4 个卷                  │ ...            │  （页签内容区）      │
│ 网络  ● 5 个网络                 │                │                    │
└──────────────────────────────────┴────────────────┴────────────────────┘
```

- 菜单固定五项，显示数量徽标；选中分类决定中间列表的数据源。
- 中间列表支持搜索、按状态过滤、排序与手动刷新；选中行决定右侧详情。
- 右侧详情按分类提供固定页签：
  - 容器：基础信息、日志、终端、统计、文件预览；
  - 镜像：基础信息、标签、层信息；
  - 数据卷：基础信息、使用它的容器；
  - 网络：基础信息、已连接容器、子网/网关；
  - 编排：项目容器列表、项目级操作。
- 标签页右上角提供「全屏」按钮，将工作区提升为全屏模态；退出后回到
  原标签位置，页面状态不丢失。

## 3. 各分类的能力

### 3.1 编排（Compose 项目）

- 数据来源：容器标签
  `com.docker.compose.project`、`...project.working_dir`、
  `...project.config_files`；按项目名聚合容器，得到项目状态（运行数/
  总数）与配置文件路径。Docker Engine 没有 Compose 专有 API，这种聚合
  方式对 Docker 与 Podman 一致。
- 列表项：项目名、运行/总数、配置目录、状态。
- 详情：项目内容器列表；操作“启动/停止/重启”默认对项目内全部容器逐个
  执行（跨平台可用）；“compose up/down/build”需要本机 compose CLI 与
  可访问的项目目录，作为 P1 能力，界面上明确标注依赖。
- 不提供 Compose 文件编辑；P2 可考虑打开项目目录。

### 3.2 容器

- 列表：名称、镜像、状态、状态文本、端口映射、创建时间。支持全部/运行中
  过滤与搜索。
- 操作：启动、停止、重启、删除（可强制），以及“打开终端”。
- 详情页签：
  - 基础信息：ID、镜像、命令、入口、用户、工作目录、环境变量、端口、
    挂载、重启策略、创建/启动时间、原始 inspect（可折叠）；
  - 日志：最近 N 行 + 跟随开关，stdout/stderr 合并展示；
  - 终端：在该容器内 exec 一个交互式 TTY，页签关闭即结束 exec；
  - 统计：CPU、内存、网络 IO、块 IO、进程数，工作区可见时 3 秒轮询；
  - 文件预览：容器文件系统浏览、文本预览/下载/上传、删除/重命名。

### 3.3 镜像

- 列表：仓库标签、短 ID、大小、创建时间；支持拉取（输入引用）与删除。
- 详情：完整 ID、标签、架构/系统、层数与大小、创建时间。

### 3.4 数据卷

- 列表：名称、驱动、挂载点、创建时间。
- 详情：驱动、挂载点、标签、正在使用它的容器（从容器 inspect 的 Mounts
  反查）。
- 操作：新建、删除（使用中拒绝并提示占用容器）。

### 3.5 网络

- 列表：名称、驱动、作用域、子网。
- 详情：网络 ID、驱动、作用域、子网、网关、已连接容器。
- 操作：新建（名称 + 驱动）、删除（内置网络与使用中的网络拒绝）。

## 4. 分层与依赖

```
apps/transom (控制平面：引擎工作区会话、订阅任务、exec 生命周期)
  ├─ transom-container (发现、传输、Docker/Podman REST、CLI 兜底、exec 桥、归档文件)
  │    └─ transom-core
  └─ transom-ui (容器侧栏、容器表单、管理工作区三栏与详情页签)
       └─ transom-core
```

`transom-container` 不依赖 GPUI，不读写配置；UI 不直接发 HTTP。

## 5. 领域模型与持久化

### 5.1 可保存的引擎连接

```rust
pub struct ContainerEngineProfile {
    pub id: EngineProfileId,          // UUID 新类型
    pub name: String,                 // 用户可读名称
    pub engine: ContainerEngineRef,   // 种类 + 端点（Auto/Unix/NamedPipe/Tcp/Ssh/Wsl/Wslc/Cli）
    pub updated_at: u64,              // 多端合并裁决
}
```

存放在 `sessions.json` 所在目录的**会话目录快照**中（`SessionCatalog.engines`），
随现有备份、云同步与三路合并一起流转；老文件缺字段时按空列表降级。
合并规则与会话相同：按 ID 对齐，`updated_at` 较新者胜，时间戳相等按操作
方向裁决。

### 5.2 工作区运行态（不落盘）

控制器为每个打开的工作区标签维护一份 `EngineWorkspaceSession`：

```rust
struct EngineWorkspaceSession {
    tab_id: TabId,
    engine: Engine,                 // 已探测的引擎实例
    exec_terminals: HashMap<TerminalRequestId, ExecHandle>,
    log_subscriptions: HashMap<ContainerRequestId, Arc<AtomicBool>>,
    stats_task: Option<JoinHandle<()>>,
}
```

标签关闭、窗口退出或引擎失联时统一中止订阅与 exec，任务句柄全部可追踪。

### 5.3 管理面数据类型（transom-core）

容器列表/详情、镜像、数据卷、网络、Compose 项目、统计快照均定义强类型；
外部 JSON 只在 `transom-container` 内部出现一次，立即收敛。

## 6. 引擎接入与平台矩阵

### 6.1 抽象

- `EngineTransport`：把「连上守护进程」收敛为一条双向字节流——Unix
  socket、Windows 命名管道、TCP、子进程 stdio；
- `EngineBackend`：HTTP（Docker 兼容 REST）或 CLI（docker/podman/wslc）；
- 能力矩阵显式暴露差异：CLI 后端无 resize、无归档文件 API、统计粒度低。

### 6.2 平台矩阵

| 宿主 | 形态 | 首选通道 | 兜底 |
| :--- | :--- | :--- | :--- |
| Linux | Docker（root） | `/var/run/docker.sock`、`/run/docker.sock` | CLI `docker` |
| Linux | Docker rootless | `$XDG_RUNTIME_DIR/docker.sock`、`DOCKER_HOST` | CLI |
| Linux | Podman rootful/rootless | `/run/podman/podman.sock`、`$XDG_RUNTIME_DIR/podman/podman.sock`、`CONTAINER_HOST` | CLI `podman` |
| macOS | Docker Desktop | `~/.docker/run/docker.sock`、`/var/run/docker.sock` | CLI |
| macOS | Podman machine | `podman machine inspect` 报告的 socket（P1） | CLI |
| Windows | Docker Desktop | 命名管道 `//./pipe/docker_engine`、`dockerDesktopLinuxEngine` | CLI |
| Windows | WSL 内 docker-ce | `wsl.exe -d <distro> -e docker system dial-stdio` | `wsl.exe ... docker` |
| Windows | WSLC | `wslc.exe`（别名 `container.exe`）Docker 语法 | 无 |
| 任意 | 远端 daemon | `ssh://` + `docker system dial-stdio`（P1） | 无 |

发现顺序：显式配置 → `DOCKER_HOST`/`CONTAINER_HOST` → 平台已知 socket →
Docker Desktop/命名管道 → CLI。每个候选以 `GET /_ping`（1 秒超时）确认，
并记录 `API-Version` 做版本协商。

## 7. 数据通路

### 7.1 交互式终端（exec）

```
容器详情「终端」页签
  └─ ContainerCommand::ExecTerminal { request_id, options, size }
       └─ 控制器解析引擎 → POST /containers/{id}/exec（Tty=true）
            → POST /exec/{id}/start（HTTP Upgrade）
            → 读/写任务 ⇄ 有界通道 ⇄ 阻塞式 TerminalIo
                 └─ ContainerAppEvent::ExecReady { request_id, io }
                      └─ UI 在页签内创建 TerminalView
```

- resize：`POST /exec/{id}/resize`；
- 关闭页签：销毁 `TerminalView` → `SessionCleanup` 中止任务并关闭连接；
- 退出码：流 EOF 后 `GET /exec/{id}/json`，只用于日志与状态提示；
- CLI 兜底：`docker exec -it`，无 resize，能力矩阵标注。

### 7.2 日志

- 非 TTY 容器按 8 字节头解复用 stdout/stderr，TTY 容器按裸流；
- 跟随模式只在该详情页签可见时运行，切换页签/关闭工作区即取消；
- 输出走 `ContainerAppEvent::LogsChunk`，界面持有滚动缓冲（上限固定，
  超出丢弃最旧行）。

### 7.3 统计

- `GET /containers/{id}/stats?stream=0` 每 3 秒采样一次（仅统计页签可见）；
- 解析 CPU 百分比、内存用量/上限、网络收发、块 IO、进程数；
- CLI 后端用 `docker stats --no-stream --format '{{json .}}'` 降级。

### 7.4 文件

- 归档 API：`GET/HEAD/PUT /containers/{id}/archive`，tar 流式解析与生成；
- 路径必须为容器内绝对路径，拒绝 `..` 与控制字符；下载经安全文件名校验；
- 删除/重命名走 `exec` 的 `rm --` / `mv --`（argv 传递，无 Shell 拼接）。

## 8. 命令与事件

工作区以 `tab_id` 作为引擎会话句柄；请求用 `ContainerRequestId` 关联。

```rust
AppCommand::Container(ContainerCommand)
AppEvent::Container(ContainerAppEvent)

enum ContainerCommand {
    Connect { tab_id, profile: ContainerEngineProfile },
    Disconnect { tab_id },
    TestConnection { request_id, profile },
    ListContainers { tab_id, request_id },
    InspectContainer { tab_id, request_id, container },
    Lifecycle { tab_id, request_id, container, action },
    ExecTerminal { tab_id, request_id, options: ContainerConnection, size },
    ListImages / InspectImage / PullImage / RemoveImage { .. },
    ListVolumes / InspectVolume / CreateVolume / RemoveVolume { .. },
    ListNetworks / InspectNetwork / CreateNetwork / RemoveNetwork { .. },
    ListComposeProjects { tab_id, request_id },
    Logs { tab_id, request_id, container, tail, follow },
    Stats { tab_id, request_id, container, enabled },
    ListDir / ReadFile / Download / Upload / RemovePath / Rename { .. },
    Cancel { tab_id, request_id },
}
```

事件覆盖：`Connected`、`ConnectFailed`、`TestFinished`、各列表与详情节、
`ExecReady`（携带 `TerminalIo` 与退出通知）、`LogsChunk`、`LogsClosed`、
`StatsUpdated`、`DirListed`、`FilePreview`、`TransferProgress`、
`TransferFinished`、`ActionFinished`、`Failed`。容器详情、镜像/卷/网络详情
与 `TerminalIo` 一律装箱，避免事件枚举出现尺寸倾斜。

## 9. UI 组织

- 活动栏新增「容器」，侧栏在该模式下显示引擎连接列表与新建按钮；
- 容器表单：名称、引擎种类、端点（自动/Unix/命名管道/TCP/SSH/WSL/WSLC/
  自定义 CLI）、测试连接；保存到目录；
- 工作区标签：三栏布局（菜单 200px、列表 320px、详情自适应），全屏按钮
  打开同一个状态的模态视图；
- 容器详情页签：基础信息、日志、终端、统计、文件预览；终端页签内嵌
   `TerminalView`；
- 图标全部使用 `gpui-kit` 的 `IconName`，容器默认图标复用
  `SessionIcon::Container`。

## 10. 安全与可观测性

- 引擎 socket 权限即权限边界；不保存引擎口令、不调用 `sudo`；
- 明文 TCP 需显式配置并二次确认；TLS 证书三件套 P1；
- 日志全中文、无凭证；容器名、操作、耗时进入日志上下文；
- 新增错误码：`ContainerUnavailable`、`ContainerApi`、`ContainerNotFound`、
  `ContainerConflict`；
- 外部输入（容器 ID、路径、镜像引用、卷名、网络名）进入 API 前校验；
- 大文件传输流式处理，禁止无上限 `read_to_end`。

## 11. 分期

**P0（Linux，本次）**

1. crate：发现与探测、HTTP over Unix socket、版本协商；容器列表/详情/
   启停删、exec、日志、统计、归档文件；镜像列表/拉取/删除；卷与网络
   列表/详情/删除/新建；Compose 项目聚合；
2. core：引擎连接配置、管理面模型、命令/事件、错误码、目录合并；
3. 控制平面：工作区会话、订阅任务、exec 生命周期、取消；
4. UI：容器侧栏、容器表单、三栏工作区、容器五个详情页签、全屏模态；
5. 门禁：fmt/clippy/check/test 全绿；`crates/transom-container/tests/mock_engine.rs`
   用 Unix socket 上的最小 HTTP 服务端覆盖「探测 + 版本协商 + 容器列表 +
   Compose 聚合」与「exec 升级 + 阻塞读写桥」两条链路；真机 Docker/Podman
   冒烟在装有运行时的机器上补做。

**P1**：macOS 与 Windows 通道（命名管道、Docker Desktop、WSL dial-stdio、
WSLC CLI、Podman machine）、TLS/SSH 端点、Compose CLI 级操作。

**P2**：镜像构建、Compose 文件编辑、容器指标历史、容器间文件对比。

## 12. 关键决策

| 决策 | 理由 | 代价 |
| :--- | :--- | :--- |
| 工作区标签而非独立窗口 | 与终端标签并列，符合现有标签/分屏习惯 | 标签状态需独立建模 |
| 引擎连接保存进会话目录 | 复用备份/同步/合并 | 需扩展目录合并类型 |
| Compose 项目按容器标签聚合 | 引擎无 Compose API，Docker/Podman 一致 | 项目级 up/down 依赖 CLI |
| REST 为主、CLI 兜底 | 结构化、resize、流式日志 | 需自建 HTTP 客户端与桥接 |
| 详情页签内嵌 TerminalView | 符合“终端/日志/统计/文件”并列的交互 | 终端生命周期要跟随页签 |
| 统计 3 秒轮询且仅可见时运行 | 避免后台打满守护进程 | 切回页签首帧可能稍旧 |
