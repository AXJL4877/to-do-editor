# 花字字体

花字样式（`TextStyle.font.file`）只写文件名，渲染时按以下顺序查找：

1. 环境变量 `TODO_FONTS_DIR`（多个目录用 `;` 分隔）
2. 本目录 `dependencies/fonts/`
3. `%WINDIR%\Fonts`（微软雅黑等系统字体，作为回退）

本目录中的字体文件被 `.gitignore` 排除，只有本说明入库。`./build.ps1` 会把本目录中的所有文件复制到 `dist/to_do/dependencies/fonts/`。

| 文件 | 来源 | 用途 |
| --- | --- | --- |
| `source-han-serif-sc-heavy.ttf` | 思源宋 Heavy（SIL OFL） | 内置样式 `grunge-red-echo` |
| `noto-sans-sc-bold.otf` | Noto Sans SC Bold（SIL OFL） | 字幕与缺字回退 |

**授权：** 内置样式只使用 SIL Open Font License 字体。许可证文本随字体发行包提供；不再把需要另行商用授权的字魂字体放进安装包。
