# 发布与自动更新

本文记录 Transom 三端发布流水线、shipup 自更新通道与签名密钥的维护口径。

## 1. 产物矩阵

发布由 `.github/workflows/release.yml` 在推送 `v*` 标签时触发，也可以在 Actions 页面手动指定已存在的标签。

| 平台 | 架构 | 更新通道产物 | 手动分发产物 |
| :--- | :--- | :--- | :--- |
| Linux | x86_64 / arm64 | `transom_<版本>_linux_<arch>.deb` | `.AppImage` |
| Windows | x64 / arm64 | `transom_<版本>_windows_<arch>_setup.exe`（NSIS） | 绿色便携 `.zip` |
| macOS | x64 / arm64 | `transom_<版本>_macos_<arch>.zip`（内含主程序） | `.dmg` |

`latest.json` 只登记上表“更新通道产物”，AppImage、便携 zip 与 dmg 不进入自更新清单，避免用户被引导到无法自动安装的包型。

### 各平台更新路径

- **Linux deb**：shipup 以安装器模式调用 `pkexec dpkg -i <deb>`，等待退出码后提示重启。依赖系统 polkit；没有桌面 polkit 代理的极简环境会失败，此时用户可手动下载 deb。
- **Windows NSIS**：安装器为每用户模式，shipup 以 `/S` 静默运行并等待退出，不需要 UAC。便携 zip 不参与自动更新。
- **macOS**：shipup 以 archive 模式解压自更新 zip，再替换 `.app/Contents/MacOS/transom`，并清理 Gatekeeper 隔离属性。该路径只替换可执行文件，`Info.plist` 与包内资源保持安装时的版本；Transom 的界面资产编译进二进制，因此当前功能不受影响。将来若新增需要随包更新的独立资源，应改用 `.pkg` 安装器或整包替换方案。
- **AppImage / dmg / 便携 zip**：只用于手动分发，不写进 `latest.json`。

## 2. 签名密钥

### 当前公钥

公钥编译在 `crates/transom-updater/src/lib.rs` 的 `UPDATE_PUBLIC_KEY`：

```text
kp7zz1TbrsgNbx4E7HezQqdkWUrEmM8sH5X1EFAsbHI=
```

该密钥与 snapin 的签名密钥相互独立，两个项目不共用签名身份。

### 私钥位置与 GitHub Secret

维护者本机私钥位于仓库之外：

```text
~/keys/transom/ed25519.key   # 私钥，权限 600
~/keys/transom/ed25519.pub   # 公钥，权限 644
```

发布前必须把私钥内容写入仓库 Secret：

```text
名称：SHIPUP_PRIVATE_KEY
值：  ~/keys/transom/ed25519.key 的完整单行内容
```

Secret 未配置时，发布任务的“校验签名私钥是否已配置”步骤会直接失败，不会产出无法验证的 `latest.json`。私钥不得提交进仓库、不得贴进 issue 或日志。

发布任务在签名后会从 `crates/transom-updater/src/lib.rs` 提取内置公钥，逐个复验六个更新通道产物；如果 Secret 配置的是另一把私钥，流水线会在上传前失败。

### 重新生成密钥

仅在私钥泄漏或需要轮换时执行：

```bash
shipup-cli keygen -o ~/keys/transom-next
chmod 600 ~/keys/transom-next/ed25519.key
```

随后把新公钥写入 `UPDATE_PUBLIC_KEY`，发布一个过渡版本。注意：旧版本客户端只信任旧公钥，无法验证新密钥签出的清单；轮换必须保留“旧客户端仍能更新到过渡版本”的路径，再在过渡版本中切换公钥。当前封装只接受单公钥，因此轮换需要一次明确的人工发布窗口。

## 3. 发版步骤

1. 更新 `Cargo.toml` 的 `workspace.package.version`，并同步 `Cargo.lock`。
2. 在 `packaging/linux/deb/changelog` 头部追加与版本号一致的记录。
3. 更新 `packaging/linux/com.github.mangerle.transom.metainfo.xml` 的 `<releases>`。
4. 执行本地门禁：

   ```bash
   cargo fmt --all
   cargo clippy --locked --workspace --all-targets -- -D warnings
   cargo test --locked --workspace
   cargo check --locked --workspace --all-targets
   ```

5. 提交并推送 `main`，然后打标签：

   ```bash
   git tag v1.6.2
   git push origin v1.6.2
   ```

6. 在 GitHub Actions 查看 `Release` 工作流。流水线依次执行：计算版本、生成 Changelog、三端双架构打包、shipup 签名、发布 GitHub Release。
7. 发布完成后确认 Release 附件包含 `latest.json`、六个更新通道产物，以及 AppImage、便携 zip 与 dmg。

手动触发时，在 Actions 的 `Release` 工作流里填写已存在的标签（例如 `v1.6.2`），流水线会检出该标签构建。

## 4. 本地打包与验证

### Linux

```bash
cargo build --release -p transom
cargo deb -p transom --no-build
APPIMAGE_EXTRACT_AND_RUN=1 cargo packager --release -p transom --formats appimage

dpkg-deb -I target/debian/transom_*_amd64.deb
dpkg-deb -c target/debian/transom_*_amd64.deb
```

AppImage 产物位于 `target/release/`，文件名形如 `transom_<版本>_amd64.AppImage`。

### Windows

在 Windows 主机或 CI 上执行：

```powershell
cargo build --release -p transom --target x86_64-pc-windows-msvc
cargo packager --release -p transom --target x86_64-pc-windows-msvc --formats nsis
```

NSIS 安装包位于 `target/x86_64-pc-windows-msvc/release/`，文件名形如 `Transom_<版本>_x64-setup.exe`。

### macOS

```bash
cargo build --release -p transom --target aarch64-apple-darwin
cargo packager --release -p transom --target aarch64-apple-darwin --formats dmg
```

dmg 位于 `target/aarch64-apple-darwin/release/`；自更新 zip 由同一目录下的 `transom` 单文件压缩而成：

```bash
(cd target/aarch64-apple-darwin/release && zip -j transom_<版本>_macos_arm64.zip transom)
```

### 清单与签名核验

发布任务生成 `release-files/latest.json` 后，可本地复核：

```bash
shipup-cli inspect --manifest release-files/latest.json

shipup-cli verify \
  --manifest release-files/latest.json \
  --package release-files/transom_<版本>_linux_amd64.deb \
  --target x86_64-unknown-linux-gnu \
  --public-key kp7zz1TbrsgNbx4E7HezQqdkWUrEmM8sH5X1EFAsbHI=
```

`transom-updater` 的单测覆盖了事件映射、更新器构建、空端点回退与未签名清单拒绝：

```bash
cargo test -p transom-updater
```

## 5. 客户端行为

- 启动早期调用 `check_and_recover_on_startup()`：新版本连续两次启动失败时用历史备份回滚；成功启动三秒后由后台任务调用 `confirm_update_success()` 清除观察期标记。
- “关于”页的“软件更新”分组提供检查、下载进度、验签、安装与“立即重启”。重启不会在 UI 回调里直接 `exit`：界面只置位意图并退出事件循环，`main` 完成控制器优雅停机后再拉起新进程。
- 下载走 shipup 的流式实现，进度经容量为 1 的通道回传；UI 消费不过来时丢弃旧进度，最终结果单独送达，不会无限积压。
- 清单与更新包必须同时通过 Ed25519 签名与 SHA-256 校验；缺少签名或签名不匹配会在安装前失败。

## 6. 常见问题

- **`latest.json` 404**：Release 未发布成功，或 `DEFAULT_MANIFEST_URL` 指向的仓库/标签不对。确认 Release 附件里有 `latest.json`。
- **签名校验失败**：客户端公钥与发布私钥不匹配，或 Secret 内容带了换行/引号。Secret 应是单行 Base64。
- **Linux 更新无反应**：检查 `pkexec` 与 polkit 代理是否可用；容器/无桌面会话环境需要手动安装 deb。
- **macOS 更新后仍显示旧版本**：确认替换的是 `/Applications/Transom.app` 内的二进制，且没有同时运行旧副本；`Info.plist` 版本不随可执行文件更新变化，以应用内“关于”页显示的编译期版本为准。
- **Windows SmartScreen 提示**：安装包尚未做操作系统级代码签名，这是预期行为；shipup 的 Ed25519 签名只保护更新通道，不替代 Authenticode。
