# DEB 打包说明

构建（版本、`Depends` 之外的控制字段全部取自 `apps/transom/Cargo.toml` 的
`[package.metadata.deb]`）：

```bash
cargo deb -p transom            # 产物在 target/debian/transom_<版本>_<架构>.deb
```

## 依赖清单的复核方法

`Depends` 是按实测手写而不是 `$auto` 自动生成，理由是自动路径会把结果绑到
构建机的包库状态上，同一份代码在不同 Ubuntu 版本得到不同清单。换 gpui 大
版本或引入新的系统库后必须重跑这两条，并同步 `apps/transom/Cargo.toml`：

```bash
ldd target/release/transom | grep -v 'rustc\|linux-vdso\|ld-linux' | awk '{print $1}' | sort
# 逐个反查 soname 属于哪个 Debian 包
for f in $(ldd target/release/transom | awk '/=> \//{print $3}'); do echo "$f -> $(dpkg -S $f | cut -d: -f1)"; done
```

二进制里没有 dlopen 形式的隐藏库依赖（曾用 `strings` 扫 `lib*.so` 核对），
所以 `ldd` 结果就是完整清单。

## 打包边界

- 包内**不写任何系统级默认终端配置**。是否成为默认终端由应用内
  设置 → 通用 → 桌面集成 的开关决定，只影响当前用户目录；系统级
  `/usr/share/xdg-terminal-exec/xdg-terminals.list` 会抢占整机所有用户的
  既有工作流，刻意不做。
- 图标只随一份矢量 SVG 进 `hicolor/scalable/apps/`，主题名与 `.desktop`
  的 `Icon` 键逐字对应（`com.github.mangerle.transom`）。不预生成多档 PNG：
  颗粒感的根源是 launcher 放大小位图，装 SVG 后桌面环境按目标尺寸矢量出图。
- 维护者脚本（`postinst`/`prerm`/`postrm`）只刷新桌面条目与图标缓存，任何
  失败都 `|| true` 吞掉：图标暂时不显示是体验问题，让安装回滚是事故。
  cargo-deb 只收录 `config`/`preinst`/`postinst`/`prerm`/`postrm`/`templates`
  这几个文件名，本 README 不会进包。

## 包内资产与规范对齐

除二进制、桌面条目与图标外，包内还携带 Debian 规范要求的文档资产：

- `usr/share/doc/transom/copyright`：cargo-deb 依据 `copyright` 元数据与
  `license-file` 自动生成，不手工维护；
- `usr/share/doc/transom/changelog.Debian.gz`：由 `changelog` 字段指向的
  纯文本自动压缩；发版时向 `packaging/linux/deb/changelog` 头部追加一条
  记录，版本号必须与 workspace 版本一致；
- `usr/share/man/man1/transom.1.gz`：手册页源文件 `transom.1` 自动压缩；
- `usr/share/metainfo/com.github.mangerle.transom.metainfo.xml`：AppStream
  元数据（软件中心展示用），id 与桌面条目、图标名逐字一致。

## 已知缺口

- **包内没有 `md5sums` 控制成员**：`cargo-deb 3.8.0` 不生成它（源码内无
  `md5sums` 相关实现），不是配置遗漏。影响仅限 `debsums` / `dpkg --verify`
  无法对本包做完整性校验；安装、升级与 apt 依赖解析均不受影响。真要补，
  需要在 `cargo deb` 之后解包生成校验表再重组，属于额外的打包工序。
- 未签名：Release 附件形态，走 GitHub Release 分发，不进任何 apt 源。

## 核验命令

```bash
dpkg-deb -I target/debian/transom_*_amd64.deb   # 控制字段与维护者脚本
dpkg-deb -c target/debian/transom_*_amd64.deb   # 文件布局与权限位
desktop-file-validate packaging/linux/com.github.mangerle.transom.desktop
appstreamcli validate packaging/linux/com.github.mangerle.transom.metainfo.xml
# 手册页可被正常解析（无 roff 告警）
man --warnings -l target/debian/transom_*_amd64.deb   # 或解包后指向 transom.1.gz
```
