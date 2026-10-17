# Rust 媒体处理

媒体链路为 `application Effect → app Workers → todo_media::MediaService → 本地文件 / 原生 FFmpeg 工具`。构建、启动、运行和 Rust 测试均无需 Node、npm 或本地 HTTP 端口。

## 文件分类

```text
crates/todo_media/src/
  service.rs             服务配置、工具位置、缓存位置、超时
  error.rs               I/O、工程目录、工具和媒体格式错误
  imports/mod.rs         流式复制、探测成功后发布素材
  files/                 相对路径边界和临时文件生命周期
  metadata/              FFprobe、格式分类、时长与音轨元数据
  thumbnails/mod.rs      缩略图 JPEG
  waveforms/             PCM 分块与波形峰值
  cache/mod.rs           素材身份、参数、同键去重、原子缓存
  tools/                 可执行文件定位、进程监督和输出限制
```

`todo_storage::ProjectStore::media_directory` 是工程媒体目录的唯一校验入口，复用项目名规范、现有工程检查与重解析点限制。media 因此依赖 storage；两者均不依赖 UI/application。

## 调用

```rust
let media = todo_media::MediaService::new(workspace_root);
let item = media.import_file("工程名", source_path)?;
let metadata = media.probe_media(item.path.as_deref().unwrap())?;
let jpeg = media.get_thumb(item.path.as_deref().unwrap(), 160)?;
let waveform = media.get_peaks(item.path.as_deref().unwrap(), 4096)?;
```

这些方法为阻塞的基础设施能力，应在现有 worker 上执行。共用控制层仍返回任务和 Effect，不直接执行 I/O。

## 导入与路径

- 源文件使用 File + io::copy，不将整段视频读入内存，不修改源文件。
- 工程必须已存在；已删除工程不会被晚到的导入任务重新创建。
- 临时素材采用独立 UUID 文件名，复制/同步/探测完成后才原子发布。错误自动清理临时文件。
- 素材显示名保留原始 Unicode 名称，存储名使用 UUID 与扩展名，避免覆盖同名文件及过长文件名。
- 保存的路径继续为 data/工程名/media/...，兼容既有工程。相对来源规范化后必须位于工作目录内；显式绝对媒体路径可读取旧工程的外部素材引用。

## 探测、缩略图和波形

FFprobe 元数据输出限制为 4 MiB。静态图片默认停留 5 秒；MJPEG 视频按容器与流识别为视频；音频附带封面不计为视频轨。必要时用有效 stream duration 补足缺失的容器时长。

缩略图为宽度 32..512 的偶数像素 JPEG，工具输出限制 8 MiB。波形桶数限制 1..16384，音频以 48 kHz f32le 流式读入，按音频帧对齐声道；只保存固定约 16 KiB 缓冲区、至多 3 字节残留和峰值桶。非有限样本忽略，峰值限制 0..1，缺失样本或不完整帧报错。

缓存位于 cache/media/{thumbnails,waveforms}，键包括格式版本、规范源路径、文件大小、修改/创建时间和输出参数。同一服务克隆的同键调用共用锁，避免重复生成；缓存文件以临时写入再替换发布，失败不影响已生成的有效结果。

## 工具和进程

定位顺序为 TODO_FFMPEG / TODO_FFPROBE 显式路径、工作目录 dependencies/ffmpeg/bin、PATH。FFmpeg 工具只在媒体任务期间启动，无常驻后端。Windows 子进程隐藏窗口，stdout 和 stderr 分开排空；诊断最多保留 4 KiB。默认任务超时为 120 秒，可通过 with_tool_timeout 配置；超时后 kill 并 wait 回收。

## 开发与分发

- `run.ps1` 构建必要的原生预览宿主并运行 Cargo 应用。
- `build.ps1` 默认构建 release，`-Configuration debug` 构建调试包。产物位于 dist/to_do，包含 Rust 主程序、原生预览宿主、两个媒体工具和说明文件。
- 打包使用独立的 target/package 编译目录，避免 Windows 下运行中的开发程序占用输出文件。
- 打包后的程序优先使用自身目录作为数据和资源根目录，不依赖源码仓库位置；TODO_REPO_ROOT 仍可显式覆盖。
- FFmpeg 开发库不参与 Cargo 构建；二进制仅在运行媒体任务或打包时需要。
- 既有 C++/Media Foundation 原生预览保持原生渲染链路，本轮替换的是 Node 媒体服务。

旧后端原文件已可恢复地移到忽略目录 target/retired-node-backend，作为迁移留存，不参与构建、启动、测试或打包。历史报告保留当时的架构描述。

## AI 产物媒体格式

`todo_media::encode_frames` 可把 PNG 序列编码成 packed-alpha 视频：颜色帧在上半部，灰度
alpha 在下半部，视频 metadata 使用 `todo_alpha=stacked`。编码前会对透明像素做有限次数的
RGB 外扩，避免 yuv420p 在边缘产生暗边；取消使用共享进程监督器，输出失败不会留下临时序列。

`todo_media::write_reveal` / `read_reveal` 是 `todo-reveal/1` 的唯一 Rust 编解码入口。文件
宽度为 `2W`，左半是最终 RGBA，右半是逐像素时间与柔化宽度；完整 JSON 放在 `todo-reveal`
的 PNG `tEXt` 块中。探测 `.reveal.png` 时使用元数据里的显示尺寸和时长，不把右半误报成画面。

`todo_media::compose_reveal` 是导出与原生着色器共用语义的 CPU 参考实现：按 R/G 解码
16 位出现时间，按 B 解码柔化边缘，并把参与位与最终 straight-alpha 相乘。编码完成后，
`encode_frames` 会在发布前用 FFprobe 校验视频尺寸、帧数和 `todo_alpha=stacked` 容器标签；
校验失败会删除输出。`todo_media::inspect_quality` / `write_report` 可将帧率、时长、音视频
时长差和编码流信息写成 `<output>.report.json`，响度和 PSNR 在没有参考帧时明确列为未测量。

`preview_host.exe --render` 使用行协议：`begin <width> <height>`，随后每帧发送
`layers-begin`、若干 `layer <kind> <time> <x> <y> <scale> <scale-x> <scale-y> <opacity> <path>`、
`layers-commit`，最后发送 `end`。每个 commit 严格输出一张 `width × height × 4` 的 BGRA
二进制帧；空层仍输出清屏帧。离线路径会用 WIC 解码 PNG，按 `kind=image|reveal|packed` 选择
整图、左半 reveal 或上下 packed-alpha，并按层序做预乘 alpha 合成。输入缺少 begin、层行不完整
或在 begin 前提交会以错误码退出；缺失或不可访问的盘符在初始化 GPU/MF 前快速拒绝。

离线渲染保留最多 `VideoEngine::kMaxLayers` 个解码槽，按媒体路径而不是图层下标复用；图层
重排不会重开相同文件，同一路径在同一帧出现多次时仍使用不同槽位。Rust 入口是
`todo_preview::OfflineRenderer`，负责子进程生命周期、完整帧读回和取消时的 kill + wait。

窗口预览的 `layer` 行同样带有显式 `kind=video|image|packed|reveal|text` 字段，application
从 `MediaItem`（packed 会携带 `alpha: stacked`）生成它；native 不再根据扩展名猜测层类型。
