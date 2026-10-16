# 原生运行工具

媒体代码在 `crates/todo_media/`，本目录只存放第三方工具及其随附说明：

```text
dependencies/
  ffmpeg/
    bin/ffmpeg.exe
    bin/ffprobe.exe
    LICENSE
    README.txt
```

花字字体放在 `fonts/`，查找规则和授权说明见 [fonts/README.md](fonts/README.md)。

现有本地工具包已从旧 backend 目录迁入此处。二进制由 .gitignore 排除；新机器需准备 FFmpeg 工具包，或使用 TODO_FFMPEG / TODO_FFPROBE 指定各自可执行文件。运行时也支持 PATH 查找。

Rust 编译不链接 FFmpeg 开发库，不需要 Node/npm，也不会在启动时运行媒体服务器。媒体任务在 Rust worker 上按需启动工具，隐藏窗口，并在结束或超时后回收进程。

`./build.ps1` 默认构建 release，使用独立 target/package 编译目录，打包 Rust 应用、原生预览宿主、两个媒体工具和随附说明到 dist/to_do。`./build.ps1 -Configuration debug` 可用于验证打包。分发时保留工具包的许可证和说明文件。
