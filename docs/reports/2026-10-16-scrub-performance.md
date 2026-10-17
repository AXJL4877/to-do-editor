# 拖动预览性能改进

本轮继续处理长 GOP 素材大跨度和反向拖动卡顿，同时补上 Rust 侧的两个阻塞点。

## Rust 热路径

`EditorSession::scrub` 不再调用会复制整个 `Project` 的领域 seek。它只更新播放头，保留原有边界、NaN、同步播放头、传输命令和历史语义。回归测试确认反复拖动不会改变素材集合的存储地址。

文字栅格等待不再阻塞整组视频图层。视频会先出画；同一文字片段、同一布局摘要、同一画布和样式下，如果新文字帧还在生成，会暂时复用该片段上一张已显示的精确栅格，并继续使用当前变换和透明度。换文字、换片段、切源监视器或切工程都会清理回退资格，晚到的旧栅格不会把画面倒退到旧动画帧。

## 原生多层调度

预览 host 为最多四个活动层建立持久解码 worker，每个 worker 独占自己的 MF reader 和 decoder；它们共享受保护的 D3D 设备，但不跨线程读取同一个 decoder 的可变状态。层按自己的源路径 epoch 取消，文字 PNG 路径变化不会取消仍可复用的视频层。

解码层可独立发布，慢层未完成时先呈现已有层；完整层栈才标记为 exact。近似帧和精确帧仍通过原有 `video-approx` / `video-frame` 诊断区分。缓存预算改为所有层共享 256 MiB 上限，避免多层把每层预算相乘。保留的 MF sample 带有运行时 lease，避免退出时 MF 先于显示帧释放导致崩溃。

## 编辑代理

`todo_media` 新增可取消的编辑代理生成：使用随软件打包的 FFmpeg，输出无音频、全 I 帧、最多 1280×720 的 H.264 MP4，保留时间戳和显示比例，原文件不改变。代理按源文件身份和编码 profile 缓存，临时文件验证后原子发布；单文件上限 1 GiB，代理目录总上限 4 GiB，不自动删除用户已有缓存。HDR PQ/HLG 源会回退到原素材，避免无色调映射地丢失高动态范围。

桌面端使用独立的有界后台队列生成代理。生成期间仍可用原素材；当前编辑器显示的素材获得优先级。切换工程、源文件或关闭应用会取消旧任务，过期结果不会安装到新工程。拖动和播放期间使用代理，停止或松手后切回原片并再次请求精确目标；代理只替换解码路径，片段时间、变换、音频和工程引用继续使用原数据。

## 实测结果

测试素材：工程中的 `data/untitled/media/0558b6dc-d131-42ef-9e01-d1b9458df087_2026-09-30_13-16-50.mp4`，约 49.9 秒，关键帧间隔约 8.33 秒。测试使用隐藏 640×360 原生 host，60Hz 输入，2 秒正向、反向和大跨度跳跃；指标是 host 诊断事件，不代表 DWM 或物理显示器 FPS，误差只对当前 top layer 统计。

| 模式 | 层数 | 方向 | 不同帧更新率 | 最大停顿 | 最终精确帧 |
| --- | ---: | --- | ---: | ---: | ---: |
| 原片 + 新调度 | 1 | 反向 | 约 37 fps | 约 630 ms | 约 111 ms |
| 原片 + 新调度 | 2 | 反向 | 约 24 fps | 约 883 ms | 约 199 ms |
| 编辑代理 | 1 | 反向 | 约 60 fps | 约 19 ms | 约 4 ms |
| 编辑代理 | 2 | 反向 | 约 60 fps | 约 19 ms | 约 5 ms |
| 编辑代理 | 1 | 正向 | 约 60 fps | 约 26 ms | 约 1 ms |
| 编辑代理 | 2 | 正向 | 约 60 fps | 约 19 ms | 约 20 ms |

原片大跨度跳跃仍会出现接近 2 秒的等待，这是输入每帧跳到远处、且没有代理时的预期 seek 成本；代理模式下同一测试的更新率约 60fps、最大停顿约 23ms。完整原生回归运行 21 项通过。

验证命令：

- `cargo test --workspace --quiet`：437 passed，0 failed。
- `cargo clippy --workspace --lib --bins --examples -- -D warnings`：通过。
- `cargo clippy -p todo_app -p todo_application -p todo_media -p todo_interaction -p todo_ui --all-targets -- -D warnings`：通过。
- `cmd /c native\\build.cmd build\\verify-scrub`：通过，生成独立验证 host。
- `TODO_PREVIEW_HOST=native/build/verify-scrub/preview_host.exe python -m unittest discover -s native/tests -v`：21 passed。
- `native/tests/benchmark_scrub.py` 已分别跑原片和代理，并保存为 `target/scrub-benchmark-old.json`、`target/scrub-benchmark-verify.json`、`target/scrub-benchmark-proxy.json`。

当前打开的旧 `todo_app.exe` 没有被终止或替换。为了让新版可以立即试用，已生成独立目录 [dist/to_do_scrub](../../dist/to_do_scrub)：

- [todo_app.exe](../../dist/to_do_scrub/todo_app.exe)，SHA-256 `17DACC60930D35209CE5DB92F4194854AAAE3F9EAE2017204209246CC7A6B1C2`；
- [preview_host.exe](../../dist/to_do_scrub/preview_host.exe)，SHA-256 `BA531776A432A3FEFCBFAC939B0D5938DA334347A9900D4B8C388F935C790ED7`；
- 目录内带有 FFmpeg/FFprobe 和字体依赖。

这个目录不会覆盖正在运行的旧包；直接运行其中的 `todo_app.exe` 即可对比反向拖动。正式包若要覆盖 `dist/to_do`，仍需先关闭旧程序后执行 `build.ps1 -Configuration debug`。
