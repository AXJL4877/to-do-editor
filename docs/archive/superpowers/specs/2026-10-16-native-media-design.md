# Rust 原生媒体处理迁移

用户要求：移除所有 Node 运行/构建/测试依赖，媒体处理直接由 Rust 实现，目录按职责分类。用户确认可保留 FFmpeg/FFprobe 原生工具，由 Rust 直接调用并随软件打包。

- `todo_media::MediaService` 替代 HTTP MediaClient，提供 import_file、probe_media、get_thumb、get_peaks。
- `imports/` 流式复制本地文件到已有工程的 media 目录，临时文件探测成功后原子发布；失败清理临时文件，不改变源文件。
- `metadata/` 解析 FFprobe；区分视频、音频和静态图片（默认停留 5 秒），排除附加封面误判音频为视频。
- `thumbnails/` 生成 JPEG；`waveforms/` 流式读取 PCM，只保留桶峰值和不足一帧的字节。
- `cache/` 按源规范路径、大小、修改/创建时间及参数区分缓存；同服务同键去重，缓存损坏或不可写时仍可生成结果。
- `tools/` 定位 FFmpeg/FFprobe、隐藏进程窗口、并发排空输出、限制输出内存并在超时后终止和回收子进程。
- `todo_storage::ProjectStore::media_directory` 统一工程命名和目录边界校验，阻止导入重新创建已删除工程。
- `todo_app` worker 使用直接方法调用，继续复用既有队列、Ticket、共用控制接口及 AppEvent。
- 工具包迁入现有 `dependencies/ffmpeg/`；backend 的 Node 源码、包文件、生成依赖可恢复地退役至忽略目录 `target/retired-node-backend/`，删除无用途 JS 辅助文件。
- 预览协议测试改用 Rust 测试子进程，测试保留真实 stdin/stdout、饱和队列、退出和杀进程行为。
- run.ps1 只运行原生软件；build.ps1 打包 Rust 程序、既有原生预览宿主及 FFmpeg 工具。媒体工具在运行时查找，不影响 Rust 编译。
- 直接 cargo 构建不需要 Node/npm。已有工程和用户素材保持兼容，原生 C++ 预览渲染不在本轮重写范围。

验收：真实图片/音频/视频导入、探测、缩略图、波形；失败清理、Unicode/空格路径、缓存失效、流式分块、工具超时、worker 部分失败；无 Node PATH 的工作区测试、编译检查和打包。
