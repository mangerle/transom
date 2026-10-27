# 排障手册（Runbook）

面向"应用出问题时如何定位"的操作清单。所有路径以 Linux 为例 macOS 按平台
约定替换（`~/Library/Application Support/transom`）。

## 日志

| 项目 | 位置/取值 |
| :--- | :--- |
| 文件 | `~/.config/transom/logs/transom.log`（轮转：5 MiB × 4 份） |
| 级别 | 设置 → 通用 → 应用日志（即时生效）；启动时 `TRANSOM_LOG=debug` 或 `RUST_LOG=debug` 覆盖 |
| 终端回显 | stderr 只镜像 `warn` 及以上（从终端启动时可见） |
| 凭据 | 绝不记录明文口令/私钥；`debug` 会记录主机名、路径与命令行 |
| 崩溃 | panic 钩子把位置与原因写入日志（`线程发生 panic，位置: ...`），随后进程退出 |

常用过滤：

```bash
tail -f ~/.config/transom/logs/transom.log            # 实时跟
grep -E "连接|认证|握手" ~/.config/transom/logs/transom.log | tail -50
grep "panic" ~/.config/transom/logs/*.log*            # 找崩溃现场
```

## 数据文件

| 文件 | 内容 | 损坏时的影响 |
| :--- | :--- | :--- |
| `sessions.json` | 会话树、文件夹、凭据模板 | 无法解析则界面按空目录启动（日志有记录）；可从备份或云同步恢复 |
| `settings.json` | 偏好、主题、字体、快捷键 | 读取失败按默认配置启动（不阻断） |
| `known_hosts.json` | 主机指纹（TOFU） | 删除后所有主机首次连接重新确认指纹（安全方向） |
| `logs/` `audit/` | 日志与审计录像 | 可直接清理；audit 开启时才产生 |
| 系统钥匙串（service `transom`） | 口令/口令短语 | 条目缺失 = 下次连接重新询问 |

## 常见故障

### 连接失败（超时 / 认证失败 / 主机密钥）

1. 日志按序关注：`连接阶段超时`（网络/防火墙）、`服务器拒绝了全部认证方式`
   （凭据/方法）、`主机密钥与已记录指纹不一致`（换过机器或中间人）。
2. 认证凭据在钥匙串里是否过期：设置页删除对应条目后重连会重新询问。
3. 跳板链失败会带"经跳板机 {host}:{port} 建连失败"上下文，逐跳核对。
4. 老设备只支持 SHA1 用户签名时日志会有 `只支持 SHA1 用户公钥签名` 的
   warn；要拒绝该降级，把 `settings.json` 的
   `allow_sha1_publickey_auth` 改为 `false`。

### SFTP 慢 / 卡住

1. 传输有 30 秒兜底的请求超时与 4 MiB 输出上限（远端 exec）；日志关键词
   `超时`、`输出超过 ... 上限`。
2. 目录打不开且日志出现 `目录条目超过 200000 上限`：目标目录过大，用过滤
   或进入子目录。
3. 速率异常时先确认链路 RTT 与远端磁盘（`df -P -k` 与 `iostat`）。

### 界面卡顿 / CPU 占用高

1. `debug` 级日志出现 `样本被丢弃：事件队列已满` = UI 消费停滞：检查是否
   打开着原生文件对话框、系统是否在挂起恢复中。
2. 大量本地标签时统计采样是共享的（每机器一份）；若仍见多个 `df` 子进程，
   报告即可。
3. 终端刷屏（`cat` 大文件）属正常高负载；读块已是 64 KiB。

### 退出慢 / 退出时连接没有优雅断开

- 正常退出流程：UI 关闭 → 控制平面收尾（断开远端、停转发，上限 3 秒）→
  运行时关闭（上限 2 秒）。日志会依次出现 `控制平面已完成优雅停机` 与
  `控制器主循环已正常退出`。
- 出现 `等待控制平面停机超过 5 秒` = 某个收尾任务卡住：抓取当时日志定位
  是哪个连接。

### 应用无法启动

- `日志系统装配失败`：查看 stderr；不影响启动（降级无日志）。
- 单实例：第二个进程会把请求转交给已运行实例后退出（日志
  `唤起请求已转交给正在运行的实例`）；要并行调试先 `unset XDG_RUNTIME_DIR`
  或换用户目录。
- 设置/会话文件损坏：见上表，均可从 `.bak` 备份或重新配置恢复。

## 依赖升级

```bash
cargo tree -d                      # 重复版本总览（先按 Cargo.toml 的口径分类）
cargo update --dry-run             # 预演 semver 兼容范围内的升级
cargo update                       # 执行；然后跑四道门禁
cargo fmt --all && cargo clippy --all-targets -- -D warnings
cargo check --all-targets && cargo test --workspace
```

规则：跨 major 的"统一去重"（尤其 RustCrypto 家族与 gpui 生态）只随
`russh` / `ssh-key` / `gpui-kit` 的上游升级进行，禁止强行降/升传递依赖；
升级 `russh` 时重点回归：主机密钥校验、SFTP 大文件、ZMODEM 互操作
（`cargo test -p transom-net --test zmodem_interop`）、转发。

## 打包与发布核验

```bash
cargo deb -p transom                        # 产物：target/debian/transom_<版本>_amd64.deb
dpkg-deb -I target/debian/transom_*_amd64.deb   # 控制字段与维护者脚本
dpkg-deb -c target/debian/transom_*_amd64.deb   # 布局与权限位
desktop-file-validate packaging/linux/com.github.mangerle.transom.desktop
appstreamcli validate packaging/linux/com.github.mangerle.transom.metainfo.xml
```

发版时同步 `packaging/linux/deb/changelog`（头部追加一条，版本号与
workspace 版本一致）与 `packaging/linux/com.github.mangerle.transom.metainfo.xml`
的 `<releases>`；详细打包口径见 `packaging/linux/deb/README.md`。

## 数据救援

- 会话目录损坏：优先从"设置 → 备份"导入最近的加密/明文备份，或从云同步
  拉取；`sessions.json` 本身是普通 JSON，可手工修剪后重启加载。
- 指纹库争议（确认被中间人）：直接删除 `known_hosts.json` 里对应条目
  （或整文件），下次连接会重新走 TOFU 确认。
- 审计录像（asciinema v2 格式）可用 `asciinema play <文件>` 回放。