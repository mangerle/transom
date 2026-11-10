# 主题来源与映射说明

Transom 的内置主题由三类来源组成：

1. `Transom Dark` 与 JetBrains Islands Dark/Light 是 Transom 自有主题。JetBrains Islands 的界面令牌移植自 `artemevsevev/zed-theme-jetbrains`，原项目使用 Apache License 2.0。
2. `popular_themes.json` 中的 20 套主题移植自 [nyakang/nyaterm](https://github.com/nyakang/nyaterm) 的 `crates/nyaterm-core/src/themes/builtins.json`。该仓库使用 Apache License 2.0。
3. GitHub、Catppuccin、Dracula、Nord、One Dark、Rosé Pine、Tokyo Night、Monokai、Solarized、Gruvbox 与 WindTerm 等名称和基础色值来自各自公开的上游配色规范。Transom 只做 gpui-kit 语义 token、语法高亮与终端角色的映射，不声称拥有这些品牌名称。

映射约定：

- UI 主背景、面板、文字、边框、主色、状态色与交互态来自主题的界面语义色。
- `editor.*` 与 `syntax` 使用同一套界面与 ANSI 色阶，保留原有字重和斜体设定。
- 每套主题的 `terminal` 节包含背景、前景、粗体强调前景、光标、选区、当前行、检索命中背景与边框，以及完整的 16 色 ANSI。
- 终端预设目录与 UI 主题伴生色板由测试逐值校验，避免“跟随界面主题”和显式选择同名终端配色出现色差。

如果后续直接增加 nyaterm 主题文件或代码片段，请同时保留本说明与仓库根目录的 Apache License 2.0。
