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

- 活动栏新增「容器」，侧栏在该模式下显示引擎连接列表、新建按钮与「发现本机
  引擎」入口；连接行首用状态点表示该连接的工作区是否连通（连通/连接中/失败），
  发现结果面板可一键添加并打开，同种类同端点不重复入库；
- 容器表单：名称、引擎种类、端点（自动/Unix/命名管道/TCP/SSH/WSL/WSLC/
  自定义 CLI）、测试连接；保存到目录；
- 工作区标签：三栏布局，菜单与列表宽度可拖拽（菜单 150-320、列表 240-620），
  全屏按钮打开同一个状态的模态视图；
- 容器行首有复选框，勾选后出现批量条：启动/停止/删除。批量删除在同一批勾选下
  必须连点两次（按钮文案变成「确认删除 N 项」）才下发，勾选变化自动解除武装；
  每个容器发一条独立命令，失败互不牵连，不做假事务；
- 卷与镜像各有一个「清理视角」快捷档：仅未使用卷、仅无标签镜像，用已拉到本地的
  容器挂载与标签计算，不再多发请求；CLI 兜底后端拿不到挂载明细时，该档位仍然显示
  但不绑事件，并在悬停时说明原因；
- 卷行的「在用 / 空闲」徽章同样由容器挂载算出：切到卷分类时顺带拉一次容器列表，
  否则在用卷会被算成空闲，用户据此删除就是在删有数据的卷；容器列表事件回来时也只
  在容器/编排分类校验选中项，不清其他分类的选中；
- 列表列（中栏）：搜索命中多个字段（容器名/镜像/状态/项目名），状态三档筛选
  （全部/运行中/已停止），排序键按分类给可用档位（名称/状态/创建时间/大小）
  并可切换升降序；行首状态点、行内快捷按钮（启停/日志/终端，静态资源为复制与
  删除），右键菜单收全部动作并在破坏性动作前加分隔线；筛选生效时页脚报告
  「匹配 x / y 条」；动作分发由 `run_row_action` 单点实现；
- 容器行右键菜单含「重命名」，点击后在详情头部展开一条改名输入条（与文件重命名
  同形态，不再多弹一层模态）；名称留空或与原名相同时不下发，避免无意义请求；
- 「运行容器」模态：镜像、名称、命令、端口（`8080:80[/tcp]`，逗号分隔）、挂载
  （`源:容器路径[:ro]`）、环境变量、健康检查命令与间隔、内存上限（MB）、CPU 上限
  （核，界面按小数填、换算成 `NanoCpus` 整数进契约）与重启策略档位；文本按 `docker run` 的常见写法
  收集，先解析成强类型 spec 再由 `ContainerCreateSpec::validate` 校验，镜像行右键
  直接进入并预填镜像名。CLI 兜底后端明确返回「不支持」，因为 `docker run` 拼参数
  有引号风险且拿不到新容器 ID；
- 健康状态：inspect 的 `State.Health.Status` 解析为 `ContainerDetails.health`，
  详情标题行给一个带颜色的状态点（healthy 绿、unhealthy 红、starting 弱色），
  基础信息里单列一行；运行容器表单可填健康检查命令与间隔，命令留空时显式下发
  `NONE`，避免继承镜像自带的 HEALTHCHECK 被误判；
- 容器详情页签：基础信息、日志、终端、统计、文件预览；日志页签可切换尾部行数
  档位（100/500/5000/全量）、跟随、引擎时间戳、「最新在前」与「复制全部」；
  最新在前用于跟随场景：纯文本块没有滚到底的自动滚动能力，反转顺序让最新日志
  始终在视野内（与 Portainer 的 reverse 选项同思路）；文件页签按目录优先
  排序，行内显示权限位、修改时间与大小，支持进入/返回/刷新/上传/下载/重命名/
  删除/新建目录；统计页签可开关引擎侧自动刷新；终端页签内嵌 `TerminalView`；
- 跨分类跳转：编排成员、卷的占用容器、网络的连接容器都渲染成可点击行，点击后由
  `reveal_container` 切到容器分类并在新列表到达时按名字选中，避免用户回到列表再搜；
- 分类切换时排序键自动回落到该分类可用档位，详情页签收回不可用项，选中项与缓存的
  详情一起清空，不给「按大小排数据卷」这类无效组合留入口；
- 两个容器模态（引擎连接、运行容器）支持点遮罩关闭，对话框本体 `occlude` 挡住
  命中测试，点在里面不会误关；Esc 依次关「运行容器」→「连接表单」→「全屏」，
  但全屏视图正处于终端页签时 Esc 不放行给界面，仍然交给容器内的程序（vim 等把
  Esc 当正常输入）；按键回调挂在浮层自身的 Overlay 上下文里，不做全局绑定，
  避免没有浮层时也吃掉这一次按键；
- 编排项目不再只读：详情头部提供「按文件重新部署」（compose up -d）与「停止并删除
  项目」（compose down，走确认条）。引擎没有 Compose 的 REST 接口，只能起宿主机
  CLI 子进程：工作目录与配置文件取容器标签且必须是已存在的绝对路径，参数以 argv
  传递不经 Shell；`docker compose` 失败时回落独立版 `docker-compose`；子进程
  `kill_on_drop` 加显式 `kill`，5 分钟超时与取消都收得回来，输出只回 8KB 内的
  尾部摘要；
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

## 13. 真机验收（Linux，2026-10-30）

模拟引擎只能证明协议解析，真实守护进程上的归档路径、HTTP 升级、统计差分、
标签聚合与响应里的空值习惯必须由 `crates/transom-container/examples/docker_smoke.rs`
跑一遍。它是人工验收工具（会真的启停容器、在容器 `/tmp` 读写临时文件），
不进 CI；每一步都有 20 秒兜底超时，失败只记录不中断，末尾按失败数决定退出码。

夹具（需要能访问引擎端点的身份）：

```sh
docker pull docker.m.daocloud.io/library/busybox:1.36
docker volume create transom-smoke-vol
docker network create transom-smoke-net
# 主进程持续产出日志，跟随验证才有意义：docker logs 只收 PID 1 的输出，
# exec 的标准输出不进日志流。
docker run -d --name transom-smoke -v transom-smoke-vol:/mnt/data \
  --network transom-smoke-net docker.m.daocloud.io/library/busybox:1.36 \
  sh -c 'echo hello-stdout; echo hello-stderr >&2; i=0; while true; do i=$((i+1)); echo "tick-$i"; sleep 1; done'
# 编排项目：工程名 transom-demo，服务定义见夹具文件
docker compose -p transom-demo -f crates/transom-container/examples/fixtures/docker-compose.yml up -d
cargo run -p transom-container --example docker_smoke
```

结论：Docker 29.1.3 / API 1.52（Ubuntu 26.04，systemd cgroup + overlayfs）
87 步全部通过（含「列表接口必须带挂载明细」的断言，界面靠它算在用卷；以及
创建时给 2 秒间隔、最终跑到 `healthy` 的健康检查闭环），
覆盖发现与协商、容器列表/详情、创建并启动容器（含端口、挂载、
环境变量、同名 409 冲突与重命名，非法名称单独用 `InvalidName` 报错而不是复用
路径错误）、统计两次采样、日志全量与
stderr 分流、日志时间戳、实时跟随与取消、exec 建立/尺寸/回显/结束通知、归档
列目录与预览与上传下载与重命名删除、新建目录与递归建目录与递归删除、镜像列表
/详情/重复拉取、卷与网络的创建详情删除、Compose 标签聚合与项目级 down/up 闭环（down 后
项目从列表消失、up 后容器与网络按 compose 文件恢复）、容器路径防线、启停状态闭环。

真机暴露的 7 个问题（均已修在 `transom-container`，各带回归测试）：

| 现象 | 原因 | 处置 |
| :--- | :--- | :--- |
| 数据卷详情解析失败 | Docker 对无标签卷返回 `"Labels": null`，而 `#[serde(default)]` 只兜底「键缺失」 | `null_as_empty` 反序列化器覆盖 Labels/Ports |
| 平台列一片空白 | Docker 29 的 `/version` 把 `Platform.Name` 返回成空串 | 空白按缺失处理，回落到 `os/arch` |
| 要全量日志却拿到 0 字节 | REST 与 CLI 都把 `tail=0` 理解成「最后 0 行」 | `tail_token`：0 翻译成 `all` |
| 一次性 exec 打出 500 告警 | 无 TTY 的 exec 不支持 resize；短命令在 resize 到达前已退出 | 仅 TTY 设初始尺寸、控制任务丢弃无 TTY 的 Resize、日志降为 debug |
| 侧栏出现两条等价 Docker | `/run` 是 `/var/run` 的符号链接；且 CLI 兜底与 socket 同时可用 | unix 端点按真实路径去重；同种类有 REST 就剔除 CLI |
| `Healthcheck` 写在 `HostConfig` 里会被守护进程静默丢弃：创建成功但 `Config.Healthcheck` 为空 | 该字段属于顶层 Config 而不是 HostConfig，且超时必须落在 `[1ms, 间隔]` 区间内，否则创建返回 400 | 移到顶层并按间隔一半夹取超时；真机断言最终 `healthy` |
| 列目录偶发「connection closed before message completed」 | 归档泵任务只搬走 `response.body`，连接守卫随 `RawResponse` 在函数返回时析构并中止驱动任务，分块响应被截断 | `http::pump_body` 整体接管响应，守卫与响应体同生命周期；模拟引擎用 40 条目分块 tar 做回归 |

仍未覆盖：三栏界面、全屏模态、容器表单的保存与测试连接、exec 终端的交互
手感——这些要以能访问 socket 的用户身份运行应用本体后在真实窗口里验收。
