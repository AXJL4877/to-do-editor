# X 导出接力收口

日期：2026-10-17（Asia/Shanghai）

本轮按 v3 接力指南完成 X1–X4 的导出接入：

- PSNR 质检不再把 PNG 序列和成片交给 FFmpeg 的 `psnr` 滤镜直接比较。现在抽取首帧和最多四个均匀分布的参考帧，用 FFmpeg 精确解码成 `rgb24`，由 Rust 计算 RGB MSE/PSNR；帧序列、尺寸不一致和空输入都会给出明确诊断。
- `Workspace::dispatch(AppAction::ExportTimeline)` 会复制当前工程快照，生成 `MediaTask::ExportTimeline`。`MediaOutput::ExportProgress` 每 10 帧（以及最后一帧）报告进度，完成事件带上未通过的质检项。
- 导出由 `Workers` 的独立线程运行，不占用导入、缩略图和媒体登记队列。取消通过共享 `AtomicBool` 传给渲染器、FFmpeg 和离线导出流程；失败或取消会删除半成品。
- 导出窗口改为选择 MP4 输出路径，提供标准、高质量、竖屏和草稿预设、进度条及取消按钮；竖屏预设要求工程画布本身为竖屏。

验证：`cargo fmt --all`、`cargo test --workspace --quiet`、`cargo clippy --workspace --lib --bins --examples --all-targets -- -D warnings` 均通过。`todo_media` 的 PNG 序列导出测试实际编码、解码并验证 PSNR 通过。

X5 已完成复验：原生解码槽按媒体路径复用，`native/tests/benchmark_render.py` 在 3 层
1920×1080 夹具上测得 44.397 fps（1.480× 30 fps）。真实导出夹具
`crates/todo_app/tests/three_layer_export.rs` 覆盖视频 + reveal + 字幕图 + 两条音频，
经 `OfflineRenderer → FFmpeg → QualityReport` 跑通，PSNR ≥ 45 dB 且质量门全绿。

后续接力补齐了黑板烘焙器的 C1/C2：reveal 自检失败时第二次重放书写并写出逐帧 RGBA，交给宿主 `encode(alpha=true)` 生成 packed-alpha 条目；盖章元素现在会生成确定性的粉笔灰粒子序列，作为 `<元素key>/dust` 链接条目随主元素移动。Windows 下工作目录的规范化路径也已修复，离线真实宿主测试确认盖章灰条目入轨。

K 项也开始迁移：海绵宝宝 worker 现在会把长文按 1200 字拆分配音，先 emit 多段 voice，再解析 chat 的 shots/scenes 设计并逐镜调用生图，在线和离线分支都包含 `intro_red`、角色、道具和气泡层；字幕 cue 直接作为可编辑 `ItemSource::Text` 入轨，最后 emit 透明泡泡转场、独立 SFX 和三段 BGM。在线转场通过宿主 `encode(alpha=true)` 生成 packed-alpha，离线夹具保持无网络可复现。宿主测试覆盖多镜头、角色/道具/气泡/转场条目、素材登记和工作流文档持久化，并保持两次自检结果一致。完整旧管线的逐帧设计渲染仍待线上验收。
chat 返回的可选 `bgm_plan` 现在会经过有限值/范围校验后决定三段 BGM 时点，缺失或非法时回退到确定性默认值，并写入 workflow 结果。
离线 workflow 验收还会断言角色、字幕、wipe/sfx 的 role/link，以及三段 BGM 的独立 lane 和递增时点。

U 项补上了第一条编辑链路：成片面板现在投影段落、元素和候选图，可显示真实候选缩略图，保存段落文案、保存元素描述、挑选候选、再来一张，以及选择图片复制进生产 workdir 后上传为锁定候选；动作直接转成 `Workspace::edit_production` 的领域 `Edit`，没有在 UI 层复制重算规则。

P7 进程池覆盖补强：新增真实 Python worker 的服务调用往返和同一进程复用测试、运行中取消后在 cancel grace 内终止并返回 `Cancelled` 的测试，以及清单未声明服务直接返回 `Invalid` 且不转发到宿主的测试。

P bridge 收尾：新增 chat 结果、image artifact 落盘、同 key 重跑保留已有文件和 unavailable 错误映射测试；另用本地 FakeServer 驱动真实 bridge + AiService，验证图片请求在 503 后自动恢复（请求 2 次）并落盘。todo_ai 的 FakeServer 仍覆盖完整断路器序列。
