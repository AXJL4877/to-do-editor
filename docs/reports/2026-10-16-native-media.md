# Rust 原生媒体迁移交付记录

## 范围与结果

素材导入、元数据探测、缩略图、音频波形均改为 `todo_media::MediaService` 本地调用。应用 worker 直接调用 Rust 服务，保留现有任务队列、Ticket、共用 AI 控制接口和事件回执。构建、测试、启动和媒体任务不再依赖 Node/npm、HTTP 媒体服务或后端端口。

用户已确认 FFmpeg/FFprobe 可保留为原生工具，由 Rust 按需直接启动并随软件打包。既有 C++/Media Foundation 预览宿主继续使用原生渲染链路，本轮未重写。

## 实现与整理

- `imports/` 负责流式复制与探测后发布；失败清理临时文件，保留源文件，不重新创建已删除工程。
- `metadata/` 负责媒体类型、音轨、时长和尺寸；区分静态图、MJPEG 视频与音频封面。
- `thumbnails/` 负责 JPEG 生成及缓存内容校验；`waveforms/` 固定缓冲区流式处理 PCM 峰值。
- `cache/` 按源文件身份及输出参数失效，同服务克隆的同键请求去重；缓存不可写时仍可返回有效生成结果。
- `files/` 负责路径边界和临时文件生命周期；工程媒体目录复用 `ProjectStore::media_directory` 的项目与重解析点检查。
- `tools/` 负责工具定位、输出上限、并发排空 stdout/stderr、Windows 隐藏窗口及超时后终止回收子进程。
- FFmpeg 工具从旧 backend 迁入 `dependencies/ffmpeg/`，移除 ureq 和旧 HTTP 客户端。
- Rust 测试程序自身模拟协议子进程，预览和进程管理测试无需 Node 或运行时编译器。
- `run.ps1` 只构建必要的原生预览宿主并运行 Rust；`build.ps1` 使用独立 `target/package` 编译目录，避免开发程序占用 Windows 输出文件。

旧 Node 后端原文件可恢复地留存在忽略目录 `target/retired-node-backend/`，不参与当前构建、启动、测试和打包。当前源码目录没有 JS/TS/npm 包文件。历史报告保留原架构描述。

## 验证证据

| 检查 | 结果 |
| --- | --- |
| 移除 PATH 中所有 Node 目录后，`cargo test --workspace --quiet` | 312 passed，0 failed |
| 同一无 Node 环境的 `cargo check --workspace` | 通过 |
| 同一无 Node 环境的 `./build.ps1 -Configuration debug` | 通过，生成 dist/to_do |
| `python -m unittest discover -s native/tests -v` | 20 passed，0 failed |
| `cargo clippy --workspace --lib --bins --examples -- -D warnings` | 通过 |
| `cargo clippy -p todo_media -p todo_storage -p todo_app -p todo_preview -p todo_application --all-targets -- -D warnings` | 通过 |
| 本轮修改 Rust 文件的 rustfmt 检查及 PowerShell 脚本语法解析 | 通过 |
| `cargo run -p todo_application --example control` | 入轨、关联切开、一次撤销通过，素材仍保留 |
| 打包资源检查 | 主程序、preview_host.exe、FFmpeg、FFprobe、LICENSE、README.txt 齐全；打包工具可执行 |

真实媒体测试覆盖 Unicode/空格路径、图片/音频/视频、损坏/缺失/空文件、失败清理、已删除工程、路径越界、参数缓存、源文件替换、损坏 JPEG 缓存恢复和并发请求。工具测试覆盖 stderr 大量输出、失败退出、stdout 超限和超时回收；worker 测试覆盖部分失败与完成事件。

独立只读审查发现两项问题，均已修复并验证：运行时 cache 忽略规则需锚定仓库根目录，避免隐藏源码 `src/cache/mod.rs`；JPEG 缓存需实际解码校验，损坏内容会重新生成。后一问题已先用损坏缓存测试复现失败，再修复通过。

详细日志保存在忽略目录 target/ 下：native-media-no-node-tests.log、native-media-check.log、native-media-package.log、native-media-preview-tests.log、native-media-clippy.log、native-media-target-clippy.log 和 native-media-control-example.log。

## 产物和使用

```powershell
# 开发运行
.\run.ps1

# 默认 release 打包
.\build.ps1

# 本轮已验证的调试包
.\build.ps1 -Configuration debug
```

产物位于 `dist/to_do/`，主程序为 `dist/to_do/todo_app.exe`。移动整个目录时保留旁边的 preview_host.exe 和 dependencies/ffmpeg/。程序优先从自身目录定位打包资源及 data/cache；TODO_REPO_ROOT 可显式覆盖工作目录。

## 检查边界

- 本轮完成 debug 目录打包，未运行 release 打包或制作安装器。
- 没有额外进行窗口交互验收；导入、进程管理和预览通过真实媒体、worker 与原生协议回归验证。
- 全库格式检查仍存在其他文件的既有格式差异；全库 all-targets Clippy 的既有 core/tests/track_insertion.rs:176 告警不属于本轮修改。生产代码、本轮涉及 crate 的全部 target 及修改文件的格式检查均通过。
- 继续使用当前未提交迁移工作区，没有创建提交、切换分支或修改用户工程数据。

目录及接口说明见 [Rust 媒体处理](../architecture/native-media.md)，原生工具准备方式见 [dependencies](../../dependencies/README.md)。
