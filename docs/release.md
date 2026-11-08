# 发布与自动更新

本文记录 Transom 三端发布流水线、shipup 自更新通道、操作系统代码签名与发布凭据的维护口径。

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

## 2. 签名体系

发布链路包含两层互不替代的信任：

- **Ed25519 更新签名**：保护 `latest.json` 与更新包不被篡改，由 shipup 在安装前校验。
- **操作系统代码签名**：Windows Authenticode 与 macOS Developer ID 证明发布者身份，并让 Windows SmartScreen、macOS Gatekeeper 与公证系统能够验证产物。

Windows 与 macOS 任一代码签名凭据缺失时，`signing-preflight` 会在所有构建开始前失败，流水线不会发布未签名产物。

### 2.1 Ed25519 更新公钥

公钥编译在 `crates/transom-updater/src/lib.rs` 的 `UPDATE_PUBLIC_KEY`：

```text
kp7zz1TbrsgNbx4E7HezQqdkWUrEmM8sH5X1EFAsbHI=
```

该密钥与 snapin 的签名密钥相互独立，两个项目不共用签名身份。

### 2.2 发布所需 GitHub Secrets

以下 Secret 均为必需项：

| Secret | 用途 |
| :--- | :--- |
| `SHIPUP_PRIVATE_KEY` | 签发自更新清单与更新包 |
| `WINDOWS_CERTIFICATE_BASE64` | Authenticode PFX 的 Base64 内容 |
| `WINDOWS_CERTIFICATE_PASSWORD` | Authenticode PFX 密码 |
| `APPLE_CERTIFICATE_BASE64` | Developer ID Application `.p12` 的 Base64 内容 |
| `APPLE_CERTIFICATE_PASSWORD` | Developer ID Application `.p12` 密码 |
| `APPLE_ID` | Apple 公证账号 |
| `APPLE_APP_SPECIFIC_PASSWORD` | Apple ID 的 App 专用密码，不是账号登录密码 |
| `APPLE_TEAM_ID` | Apple Developer Team ID |

Secret 只存放在 GitHub 仓库设置中，不得提交进仓库、写入构建产物、贴进 issue 或输出到日志。GitHub Actions 会对 Secret 值做日志遮蔽，但工作流仍禁止主动打印这些变量。

证书 Secret 的值必须是单行 Base64。Windows PFX 可在 PowerShell 中生成，macOS `.p12` 可在终端中生成：

```powershell
[Convert]::ToBase64String([IO.File]::ReadAllBytes("D:\secrets\transom-authenticode.pfx"))
```

```bash
openssl base64 -A -in ~/keys/transom/DeveloperIDApplication.p12
```

### 2.3 Windows Authenticode

Windows 发布任务从 `WINDOWS_CERTIFICATE_BASE64` 还原临时 PFX，并调用 `packaging/windows/sign-authenticode.ps1`：

1. 在 NSIS 打包前签署 `transom.exe`，确保绿色便携包内也是已签名主程序。
2. NSIS 安装包生成后再次签署 `*-setup.exe`。
3. 使用 SHA-256 文件摘要、SHA-256 时间戳摘要与 DigiCert RFC 3161 时间戳服务。
4. 每次签署后都执行 `signtool verify /pa /all` 并检查 `Get-AuthenticodeSignature` 状态必须为 `Valid`。

PFX 必须包含代码签名私钥，并且证书链能被公开 Windows 信任库验证。临时 PFX 与密码只存在于当前运行器的隔离环境中，PFX 在任务结束后清理。

### 2.4 macOS Developer ID、Hardened Runtime 与公证

macOS 发布任务使用 `packaging/macos/sign-and-notarize.sh` 完成以下闭环：

1. 从 `APPLE_CERTIFICATE_BASE64` 还原 `.p12`，导入一次性临时钥匙串。
2. 查找 `Developer ID Application` 身份，主程序与 `.app` 均使用 Hardened Runtime、可信时间戳和 `packaging/macos/Transom.entitlements` 签名。
3. 从已签名 `.app` 生成包含 `/Applications` 快捷方式的 DMG，并签署 DMG。
4. 调用 `xcrun notarytool submit --wait` 提交 Apple 公证；只有输出包含 `status: Accepted` 才继续。
5. 使用 `xcrun stapler staple` 与 `validate` 把公证票据固定到 DMG。
6. 从同一已签名主程序生成自更新 zip，并重新执行 `codesign --verify --strict`。

entitlements 当前保持空字典：Transom 不需要 JIT、关闭库校验或放宽沙箱的额外权限。不要为了“看起来完整”而添加高权限 entitlement；新增项必须在 macOS 真机验证并有明确安全理由。

### 2.5 私钥位置与重新生成更新密钥

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

Secret 未配置时，发布任务会在 `signing-preflight` 阶段直接失败，不会产出无法验证的 `latest.json`。私钥不得提交进仓库、不得贴进 issue 或日志。

发布任务在签名后会从 `crates/transom-updater/src/lib.rs` 提取内置公钥，逐个复验六个更新通道产物；如果 Secret 配置的是另一把私钥，流水线会在上传前失败。

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

6. 在 GitHub Actions 查看 `Release` 工作流。流水线依次执行：计算版本、校验全部签名 Secret、生成 Changelog、三端双架构打包、Windows Authenticode 签署、macOS 签名与公证、shipup 签名、发布 GitHub Release。
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

在 Windows 主机或 CI 上执行。先设置 `WINDOWS_CERTIFICATE_PATH` 与 `WINDOWS_CERTIFICATE_PASSWORD`，不要在仓库内保存 PFX：

```powershell
cargo build --release -p transom --target x86_64-pc-windows-msvc
.\packaging\windows\sign-authenticode.ps1 `
  .\target\x86_64-pc-windows-msvc\release\transom.exe
cargo packager --release -p transom --target x86_64-pc-windows-msvc --formats nsis
$setup = Get-ChildItem .\target\x86_64-pc-windows-msvc\release -Filter '*-setup.exe' -Recurse | Select-Object -First 1
.\packaging\windows\sign-authenticode.ps1 $setup.FullName
```

NSIS 安装包位于 `target/x86_64-pc-windows-msvc/release/`，文件名形如 `Transom_<版本>_x64-setup.exe`。

### macOS

```bash
cargo build --release -p transom --target aarch64-apple-darwin
cargo packager --release -p transom --target aarch64-apple-darwin --formats app
```

执行脚本前导出全部 `APPLE_*` Secret 环境变量。脚本会自动创建并清理临时钥匙串，同时生成 DMG 与自更新 zip：

```bash
bash packaging/macos/sign-and-notarize.sh \
  target/aarch64-apple-darwin/release/Transom.app \
  dist/transom_<版本>_macos_arm64.dmg \
  dist/transom_<版本>_macos_arm64.zip
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

操作系统签名可单独复核：

```powershell
signtool verify /pa /all /v .\transom_<版本>_windows_x64_setup.exe
```

```bash
codesign --verify --deep --strict --verbose=2 /Applications/Transom.app
xcrun stapler validate dist/transom_<版本>_macos_arm64.dmg
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
- **Windows 仍有 SmartScreen 提示**：当前安装包已做 Authenticode 签名，但新证书可能尚未积累足够下载信誉，SmartScreen 仍可能提示；先确认签名状态为 `Valid`，不要把 Ed25519 更新签名当成 Authenticode 的替代品。
- **macOS 提示无法验证开发者**：当前 DMG 应已完成 Developer ID 签名、公证与 stapling；先运行 `xcrun stapler validate <dmg>` 与 `codesign --verify --deep --strict <app>`。旧版本下载包不在新签名流程的保证范围内。
