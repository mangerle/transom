# API 客户端迁移

本文记录 CyberAPI 桌面核心能力迁入 Transom 的范围、架构、存储和验收边界。本阶段只做桌面端本地 API 客户端，不增加账号、团队、邀请、权限、服务端同步或云端项目，也不直接引入 CyberAPI 的 crate。

## 结论与边界

当前已完成首版能力闭环，覆盖以下功能：

- HTTP、GraphQL、原始 gRPC 和 WebSocket 请求。
- Bearer、Basic、OAuth 2.0 client credentials 和 AWS SigV4。
- Cookie、环境变量与项目内请求历史。
- cURL、Postman 2.1、OpenAPI 3 JSON/YAML 的基础导入导出。
- 文本与 JSON 路径断言。
- cURL、JavaScript fetch、Python requests、Rust reqwest 代码生成。
- API 项目标签跨重启恢复。

这里的“完成”指 Transom 迁移首版可用，不等同于 CyberAPI 功能的逐项全量复制。高级补全、脚本执行和完整协议生态仍按下方边界保留。

## 产品入口

API 客户端沿用 Git、容器和会话目录的导航模型：

```text
左侧活动栏 ApiClient
  ├─ 已有活动项目：直接显示对应 API 工作区标签
  └─ 无活动项目：主区域显示项目启动页
       ├─ 选择已有项目
       ├─ 新建项目
       └─ 导入 cURL、Postman、OpenAPI 或 Transom 项目
```

API 项目不再占用常驻侧栏，多个项目通过请求树顶部的下拉菜单切换。终端、Git、容器和 API 标签可以同时存在，API 项目不会伪造终端会话或占用 `TerminalTab` 的会话字段。

## UI 信息架构

UI 采用“Transom 视觉语言 + CyberAPI 信息架构”的原则，不复制 CyberAPI 的 Web 页面外观，但保留其对象层级和操作路径。

```text
API 项目标签
  ├─ 左侧接口树
  │    ├─ 项目下拉：切换、新建、导入、重命名、删除和导出项目
  │    ├─ 新建、重命名和删除目录
  │    └─ 新建、重命名和复制请求
  ├─ 项目工具条
  │    ├─ HTTP / GraphQL / gRPC / WebSocket 协议
  │    ├─ 环境选择与环境变量
  │    ├─ Cookie 管理
  │    └─ 代码生成
  ├─ 请求编辑区
  │    ├─ 请求行：方法/协议、环境、URL、粘贴 cURL、发送
  │    └─ 请求页签：Query、Body、Header、Auth、Assertions、Settings、Docs
  └─ 响应面板
       ├─ 状态、耗时、大小和断言摘要
       └─ 响应页签：Body、Headers、Cookies、Assertions、History
```

请求页签按方法和协议动态出现：

- Query：名称、值、说明三列表格，启用项合并到发送 URL。
- 接口树支持单层目录管理，请求可通过右键菜单移动到任意目录或移回根层；删除目录时，目录内请求会移回项目根层，不会连带删除。
- Body：HTTP 请求体、GraphQL Query/Variables、gRPC 服务/方法/消息、WebSocket 消息。
- Header：名称、值、说明三列表格。
- Auth、Assertions：当前请求的认证与断言。
- Settings：请求级超时，0 或留空使用默认值。
- Docs：请求说明文档。
- 地址框粘贴完整 cURL 命令时，自动填充方法、URL、Query、Header 和 Body。

项目级动作与请求级页签分离，避免把环境或 Cookie 当作当前请求字段。不同功能使用不同语义图标，禁止所有工具复用同一个齿轮图标。

当前已完成信息架构骨架、结构化键值编辑器、表单化认证/断言/环境/Cookie 和导入预览；独立环境/Cookie 弹窗与历史详情仍按后续阶段继续完善。

## 类型与状态边界

- `ApiSidebarView`：窗口单例项目管理实体，保存项目登记表、内联编辑器、删除确认和导入预览，不再渲染常驻侧栏。
- `ApiWorkspace`：每个项目标签独立实体，保存请求树选择、请求草稿、环境、Cookie、响应、断言、历史和协议运行状态。
- `ApiHomeTab`：无项目启动标签，只承载项目列表、新建、导入和导入预览，不保存项目状态。
- `ApiClientTab`：`TabContent` 的类型化内容，只持有 `ApiProjectId` 与 `Entity<ApiWorkspace>`。
- 关闭 API 标签会释放工作区实体；发送任务和 WebSocket 连接随对应句柄释放而取消或停止。
- 项目管理实体和工作区只通过事件把完整意图交给外壳，外壳不回读派发事件的实体。
- `AppShell` 仍只持有 `AppState`；项目登记表、工作区与服务分别归属项目管理实体、标签实体和窗口服务集合。

## 数据与存储

`transom-api` 定义项目、目录、请求、环境、Cookie 和历史模型，以及 `ApiProjectStore` 接口。

`transom-store::JsonApiProjectStore` 是首个实现，文件路径为：

```text
<config_dir>/transom/api_projects.json
```

存储使用原子写路径：临时文件、`fsync`、原子替换和损坏文件隔离。UI 通过 `Persistence` 的单槽异步写入器保存项目，避免在渲染和点击回调中同步写盘。

当前项目 schema 为 v3，登记表包含：

- 项目、目录、请求和请求头；
- 环境、活动环境标识与 `{{变量}}`；
- 全局变量与全局 Query/Header 参数；
- Cookie 及域名、路径、Secure、HttpOnly、过期时间；
- 最近 100 条请求历史与响应摘要；

旧版项目文件仍可加载；高于当前程序支持版本的 schema 会拒绝加载，避免静默丢失新字段。

## 协议与执行

### HTTP 与 GraphQL

- HTTP Body 使用与 APIPOST 类似的“模式 + raw 语言”结构：None、form-data、urlencoded、binary 和 raw；raw 支持 JSON、XML、JavaScript、Text、HTML。
- `form-data` 字段可逐行选择 Text 或 File；文件值与文本值分开保存，选择后在后台检查文件是否存在、是否为普通文件以及是否超过 64 MiB，并支持替换和清除。
- Binary Body 通过系统文件选择器选择，发送时使用文件扩展名推测 MIME，缺失时回退为 `application/octet-stream`。
- Query 与 Header 的值始终是文本；文件上传不属于 Query 或 Header。
- 全局复用单个 `reqwest::Client`，保留连接池和 Keep-Alive。
- HTTP 支持 GET、POST、PUT、PATCH、DELETE、HEAD、OPTIONS。
- GraphQL 固定以 POST 发送 JSON，包含 `query` 与 `variables`。
- GraphQL 查询和变量都支持环境变量替换。
- 连接超时为 15 秒，单次请求总超时为 60 秒。
- 响应体流式读取，超过 4 MiB 立即停止并返回中文错误。
- JSON 响应自动格式化，非 JSON 文本原样展示。
- 请求任务由可取消句柄持有；关闭标签或重新发送会中止旧任务。
- 日志不输出请求头、请求体或包含查询参数的完整敏感 URL。

### 原始 gRPC

- 使用 HTTP/2 POST 到 `/{服务名}/{方法名}`。
- 设置 `application/grpc`、`te: trailers` 与 `grpc-accept-encoding: identity`。
- 请求体按 Base64 输入，执行时解码并添加 5 字节 gRPC framing。
- 响应先读取完整 HTTP 响应，再提取 gRPC 数据帧并以 Base64 展示。
- 当前不解析 proto 文件，不生成消息，不做 schema 补全。

### WebSocket

- 支持连接、发送文本、接收文本和关闭连接。
- 文本消息写入工作区日志；二进制消息显示字节数，避免伪造文本内容。
- 自定义请求头和环境变量可参与握手。
- Bearer 与 Basic 可用于握手认证。
- 握手超时 15 秒，发送与关闭 IO 超时 10 秒。
- 命令与事件均使用有界通道；队列满时快速失败或由接收端形成背压。
- OAuth 与 AWS SigV4 当前只用于请求型协议，不用于 WebSocket 握手。

## 认证、环境与 Cookie

- Auth 页签使用结构化表单选择 None、Bearer、Basic、OAuth 2.0 或 AWS SigV4。
- Bearer 与 Basic 从环境变量读取凭据。
- OAuth 2.0 支持 client credentials，从令牌端点获取 `access_token` 后设置 Bearer 头。
- AWS SigV4 支持 canonical query、canonical headers、HMAC 派生密钥和会话令牌。
- GraphQL 和 gRPC 的签名使用实际发送方法，不使用界面上残留的旧方法字段。
- 环境支持切换、重命名、新增和删除；环境变量使用名称、值、启用和密钥标记表格编辑。
- 全局变量作为基础变量参与 `{{变量}}` 替换，同名环境变量优先。
- 全局参数支持 Query 和 Header；请求自身同名项优先，全局参数只补缺。
- URL、请求头、请求体、GraphQL 查询与变量统一使用合并后的有效环境解析。
- Cookie 使用名称、值、Domain、Path、Secure、HttpOnly 表格编辑；响应 `Set-Cookie` 会自动更新项目 Cookie。
- 发送时按域名、路径、Secure 和过期时间筛选 Cookie，再与手工 Cookie 合并。

## 导入导出

导入时按扩展名和文件内容共同识别格式：

- cURL：基础命令、方法、URL、多个请求头、请求体、`--data-urlencode`、`-F` 文件字段和 `-T` 文件上传。
- Postman 2.1：集合名称、嵌套目录、请求方法、URL、请求头、raw、urlencoded、formdata 和 file Body。
- OpenAPI 3 JSON/YAML：`servers`、`paths`、HTTP 操作、摘要和描述。
- Transom：单个项目或本地项目登记表。

导出支持：

- cURL 脚本，包含表单字段和文件上传；
- Postman 2.1 集合，保留 formdata/file Body 类型；
- OpenAPI 3 JSON 文档；
- Transom 项目 JSON。

当前转换是基础语义映射：不完整校验 OpenAPI，不透传所有安全方案、示例、Schema、测试脚本、变量作用域或 Postman 鉴权脚本；Postman 导出会把请求扁平化。外部文件在进入导入解析前有 4 MiB 上限，读取和解析在线程池中执行；解析完成后先在项目工作区展示文件、格式、项目数、请求数和项目名称，用户确认后才写入登记表与磁盘。

## 历史、断言与代码生成

### 请求历史

- 每次成功收到 HTTP 响应后追加一条历史。
- 记录请求 ID、名称、方法、URL、状态码、耗时、时间、断言统计和最多 500 字符的响应摘要。
- 每个项目最多保留最近 100 条记录，并随项目 JSON 保存。
- 历史列表支持选中对应请求并显示该次状态、URL、耗时和响应摘要，也可单条重放；批量运行与定时运行仍未包含。

### 断言

支持以下文本语法：

```text
status == 200
status != 500
body contains 关键字
text contains 关键字
header Content-Type contains application/json
json data.id == 123
json data.token exists
```

断言在请求页以逐行表格编辑，支持启用、禁用、添加和删除；执行时只计算启用中的断言。结果是逐条通过或不匹配明细；JSON 解析失败、路径不存在或语法不支持时会返回明确失败原因，不中断响应展示。

### 代码生成

支持 cURL、JavaScript fetch、Python requests 与 Rust reqwest。选择目标后先打开代码预览，可切换语言并在确认后复制。生成内容使用当前编辑器草稿和已解析的 URL；生成器输出显式请求头与请求体，不自动展开 OAuth 或 AWS 凭据，避免把秘密值写入代码或剪贴板历史。

## 标签恢复

窗口设置新增 `previous_api_projects`，只保存 API 项目 ID，不把项目内容复制到窗口设置。退出时从标签栏收集已打开的 API 项目，下一次启动由外壳读取项目登记表并恢复标签。

项目已被删除、ID 无法解析或项目不存在时跳过该标签，并记录中文警告；其他有效标签继续恢复。API 项目标签与会话标签分开保存，避免把 API 标签误当终端连接。

## 明确未包含

- CyberAPI 的账号、团队、邀请、权限、MCP 服务端与服务器同步。
- GraphQL schema 自动补全与查询执行分析。
- gRPC proto 解析、消息生成与方法补全。
- WebSocket 二进制发送、帧级调试、OAuth 与 AWS 握手认证。
- OAuth 授权码、PKCE、设备码与自动刷新。
- JavaScript 前置脚本和测试脚本。
- OpenAPI 完整校验、全部安全方案与测试用例透传。
- HAR、Insomnia 等未列出的导入格式。
- 批量运行、定时运行和完整响应历史归档。

这些能力如需继续迁移，必须保持 Transom 的实体归属、单向事件、强类型错误、中文日志、有界通道和 UI 线程禁止同步重 IO 契约。

## 验收

自动化验证：

```text
cargo fmt --all --check
cargo check --all-targets
cargo clippy --all-targets -- -D warnings
cargo test -q
```

API 专项测试覆盖：

- HTTP 实际请求、Cookie 发送与 `Set-Cookie` 解析；
- GraphQL 实际 POST 与环境变量替换；
- OAuth client credentials 实际取令牌和 Bearer 头；
- AWS SigV4 确定性签名；
- cURL 多请求头往返与 Postman 嵌套目录；
- OpenAPI YAML；
- WebSocket 实际握手与非法 URL；
- 断言、环境变量和认证配置解析。

真实窗口验收：

- 活动栏 API Client 入口打开当前 API 项目或无项目启动页；
- 项目标签与终端、Git、容器标签共用同一标签栏；
- 选择请求后编辑器正确回填；
- 本地 HTTP 请求显示状态、耗时、字节数、响应头、响应体和断言结果；
- 环境变量、历史、代码生成与 WebSocket 控件可操作；
- 有效 API 项目标签在重启后恢复，失效项目被安全跳过。
