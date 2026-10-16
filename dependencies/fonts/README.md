# 花字字体

花字样式（`TextStyle.font.file`）只写文件名，渲染时按以下顺序查找：

1. 环境变量 `TODO_FONTS_DIR`（多个目录用 `;` 分隔）
2. 本目录 `dependencies/fonts/`
3. `%WINDIR%\Fonts`（微软雅黑等系统字体，作为回退）

本目录中的字体文件被 `.gitignore` 排除，只有本说明入库。`./build.ps1` 会把本目录中的所有文件复制到 `dist/to_do/dependencies/fonts/`。

| 文件 | 来源 | 用途 |
| --- | --- | --- |
| `zihun-jianqi-shoushu.ttf` | 字魂剑气手书（原文件名“字魂剑气手书(商用需授权).ttf”） | 内置样式 `grunge-red-echo` |

**授权：** 字魂剑气手书标注“商用需授权”。对外发布或商用前，须取得字体授权，并把授权文件放在本目录随包分发；或者把样式改用可商用字体。
