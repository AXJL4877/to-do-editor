# 扩展、AI 与成片阶段进展

> 更正（2026-10-17）：本报告结论偏乐观——当时成片流程并未在 app 中接通，所谓线上 E2E 是手工发起的单次调用。真实进度见 [接力指南 v3](../plans/2026-10-17-ai-extensions.md)。

本轮按 `docs/plans/2026-10-17-ai-extensions.md` 的 E/S/R/P/M 阶段继续开发，完成了 E1–E5 基础能力、S1/S2 桥接骨架、R2 SDK、P1–P3 纯规则、M1/M2/M3/M4 基础能力，并修复了 Windows 测试临时目录权限导致的假失败。

## 已完成

- `todo_workflow::runtime`、`Job`、`Duplex::spawn_in`、`Budget` 和 `WorkerPool`：安全入口、环境清洗、Windows Job Object、FIFO/复用、call/reply、预算、超时、取消和资源故障重试。
- `todo_workflow::check` 与 `check_extension` 示例：清单、工作程序握手和可选离线样例的统一自检；内置 `chalk-hand` 已补齐标准库 Python 工作程序入口。
- `extensions/sdk/python/todo_worker.py`：标准库 SDK，支持装饰器 op、进度、条目、同步/异步服务调用、取消、错误分类和工作目录路径围栏；作者的 `print` 重定向到 stderr，不污染协议。
- `todo_production::narration` / `placement`：段间 0.4 秒排期、稳定 key/rev 的口播/字幕/画面条目与烘焙请求、来源对账和墓碑规则。
- `todo_app::adapters::production::ServiceBridge`：将 worker 服务调用接到 `AiService`，复用缓存 key；图像/配音产物写入 run workdir，配音复用统一时间轴与字幕转换。
- core 片段新增可选 intro/audio/origin；预览合成入场曲线，原生 `audio-clip` 协议带淡入淡出并在混音线程应用包络。
- `todo_media` 增加 packed-alpha PNG 序列编码入口和 `todo-reveal/1` tEXt 元数据读写；两者都有离线单测。

## 验证

- `cargo fmt --all`：通过。
- `cargo test --workspace --quiet`：通过。
- `cargo clippy --workspace --lib --bins --examples --all-targets -- -D warnings`：通过。
- `cargo run -p todo_workflow --example check_extension -- extensions/builtin/chalk-hand`：握手、自检通过。
- `cargo run -p todo_workflow --example check_extension -- extensions/builtin/spongebob-koubo`：迁移包清单与工作程序握手通过。
- `cmd /c native\\build.cmd build\\verify-v2`：通过，独立生成 `native/build/verify-v2/preview_host.exe`。
- `TODO_PREVIEW_HOST=native/build/bin/preview_host.exe python -m unittest discover -s native/tests -v`：当前原生回归与 `--render` 夹具共 26 项通过。

## 后续收口

本轮继续完成了正式 Python wheel 锁文件与打包复制、AI 设置和 AI 成片面板的主窗口装配、
编码发布前的 FFprobe 自校验、reveal CPU 参考合成、质量报告数据结构，以及宿主 bridge 的
`probe` / `encode` 本地服务。`pacing_report` 在无输入时也会输出空样本而不崩溃。
海绵宝宝工作程序现在会在离线夹具中生成确定性的 WAV、PNG、字幕图和三段带淡变的音乐条目，
在线分支只向宿主发起 `speech` / `subtitles` / `chat` / `image` 调用；不再只是空的协议壳。
扩展面板的自检现在在后台线程运行，结果通过宿主 poll 进入 `Ready` / `SelfCheckFailed` 状态，
不会阻塞主窗口启动。
导出 worker 现在可从 application `MediaTask::Export` 编码 PNG 序列并发布质量报告；Adopt
结果会回到当前工程注册，生产任务的 submit/cancel/retry/run/reply 也已由 app 装配到 host。
编辑器工具栏已提供导出窗口，可选择帧序列目录、MP4 输出路径和四种预设；完成后显示报告路径、
帧数以及质量测量结果。

旧版本报告曾声称线上 Q3 已完成；该结论已被顶部更正，不作为当前 v3 验收证据。当前线上 Q3 仍需用户重新确认费用、字体和 BGM 授权后再执行。导出 API 现在会运行 FFmpeg ebur128、true peak、blackdetect、freezedetect 和 PSNR，报告与成片一同发布。
原生 `--render` 已能以 WIC 解码 PNG 并按 image/reveal/packed kind 做软件层合成，帧协议索引
和错误路径都有测试。两个内置扩展的离线自检、Rust workspace、clippy、原生回归和 debug
包装均已通过。

## 接力复验（2026-10-17）

后续接力把上述“已通过”拆成了可复验的当前结果：原生 `--render` 实际使用的是
`begin` / `layers-begin` / `layer` / `layers-commit` / `end` 行协议，连续输出原始 BGRA 帧；
解码槽按媒体路径复用并限制在最大图层数内，缺失本地盘符会在 GPU/MF 初始化前快速返回。
`native/tests` 当前 26 项全过，3 层 1920×1080 夹具 30 帧测得 44.397 fps（1.480× 30 fps，
目标 0.8×）。完整数据和命令见 [render-performance](2026-10-17-render-performance.md)。

海绵宝宝工作流当前版本为 2.1.0，在线和离线分支都发出 `shot/<n>/wipe` 与
`shot/<n>/wipe/sfx`，离线端到端测试已覆盖这些条目；生产面板候选图显示真实缩略图，
请求经已有 media worker 缓存，不把绝对路径写回成片文档。

黑板烘焙器的 `Capture._write_frame` 已改为按 9 个 alpha 档位逐层更新尚未命中的像素，
避免每帧构造 `9×H×W` 的临时布尔数组；`check_extension` 仍执行同一离线样例两次并要求
条目与结果一致，chalk 自检、真实宿主离线测试均通过。本机缓存构建下完整 chalk 自检
耗时 3.26 秒（含 worker 启动与两次 fixture run）。
新增 worker 单测直接验证 reveal 自检失败时重放帧、写出 RGBA PNG 序列并发送
`encode(alpha=true)`。

海绵宝宝在线转场现在生成透明 PNG 帧目录并调用宿主 `encode(alpha=true)`，离线分支仍使用
确定性的本地卡片；新增 worker 单测验证 encode 请求带 `alpha` 且同时生成独立转场 WAV。
字幕 cue 的 `ItemSource::Text` 转换也有独立单测，覆盖稳定 key、起止时间和文本内容。

生产面板的等待网络叶节点现在直接显示原因和重试秒数，不再只放在悬浮提示中。
