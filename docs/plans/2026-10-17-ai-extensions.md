# AI 与扩展接入：接力指南 v3

更新：2026-10-17（取代 v1 / v2；v2 的完整设计细节保留在 [设计稿](2026-10-17-ai-extensions-design.md)）。

本文给接手的 agent，写清**真实现状**、**已知问题**和**剩余工作**，每步给出接口、做法、最小验证和完成标准。动手前先读第 0、2、3 节。

**最终目标**：TO DO 安装后开箱即用。用户只需在「AI 设置」里填写服务地址和 key，就能用两个内置扩展从文案做出可编辑、可导出、质量有保证的成片：

- 「黑板手绘」（`builtin.chalk-hand`）
- 「海绵宝宝口播」（`studio.spongebob-koubo`）

配套文档：

- [扩展规范](../architecture/extension-spec.md)（需要按附录补写，见 P4）
- [AI 服务接入](../architecture/ai-services.md)
- [项目文件夹分工](../architecture/responsibilities.md)
- [Rust 媒体处理](../architecture/native-media.md)

---

## 0. 硬约束（违反即不合格）

1. **分层**：依赖方向由 `crates/todo_application/tests/dependency_boundaries.rs` 强制检查。
   - core 和 application 不做 I/O、不开线程；`todo_production` 是纯领域代码。
   - 只有 `todo_ai` 联网。
   - 子进程只能由 `todo_process`、`todo_preview`（原生宿主）和 `todo_media`（FFmpeg）启动。
   - app 只做装配。
   - 同一条规则只写一份。预览与导出共用 `todo_application::picture_layers`、`todo_core::picture_transform`、`reveal_frame` 和 `reveal_hand`。
2. **不用 Node**。
3. **密钥**：
   - 只存在 Windows 凭据管理器，或环境变量 `TODO_HUNIU_API_KEY` / `TODO_VOLC_API_KEY`。
   - 不进工程文件、配置、日志或测试。
   - 工作程序拿不到密钥。
   - 不读取 `D:\Desktop\密匙`。
4. **付费线上调用**：先征得用户同意；一次只做一项；记录花费。
5. **提交与进程**：
   - 用户没要求就不提交。
   - 不杀、不覆盖用户正在运行的 `todo_app.exe` / `preview_host.exe`。
   - 原生构建输出到独立目录：`native\build.cmd build\<名字>`。
6. **测试先行**。每步收尾都要跑：
   ```powershell
   cargo fmt --all
   cargo test --workspace
   cargo clippy --workspace --lib --bins --examples -- -D warnings
   # 涉及原生：
   native\build.cmd build\<目录>
   $env:TODO_PREVIEW_HOST="<目录>\preview_host.exe"; python -m unittest discover -s native/tests
   ```
7. **换行与编码**：
   - 工作区文件混有 CRLF 和 LF（`core.autocrlf=true`）。脚本做精确替换前，先把 `\r\n` 统一成 `\n`，写回时用 `write_bytes`。
   - 通过 bash heredoc 把中文传给 Python 时，按 UTF-8 读 stdin（`sys.stdin.buffer`）。
8. **Python 运行时**：
   - 用仓库内的 `dependencies/python/python.exe`：3.13.11，带 numpy 2.2.6、opencv-headless 4.10、Pillow 11.1。锁文件 `dependencies/python.lock`，获取脚本 `scripts/fetch_python.ps1`。
   - 开发环境的回退只认 PATH 上真实的解释器。**绝不能用 `WindowsApps\python.exe`**：这个 Store 别名在 stdin 为管道时会挂起。
9. **旧项目** `D:\Desktop\TO-DO-App` 只作参考，运行时不得引用其中任何路径。
10. **报告可信度**：`docs/reports/2026-10-17-extension-runtime.md` 和 `2026-10-17-online-e2e.md` 是上一位 agent 写的，结论夸大。所谓线上 E2E 只是手工发了 1 次 chat、1 次 image、1 次 speech 和 1 次烘焙，不是真实流水线。以本文第 2 节为准。

---

## 1. 终态验收（全部满足才算完成）

1. **开箱**：双击 `dist/to_do/todo_app.exe` 就能用，不需要装任何东西。
2. **AI 设置**：
   - 虎妞和火山各自可以填写、测试服务地址、key、模型、音色和代理。
   - key 显示掩码，并标明来源。
   - 改了地址或代理会自动重启 AI 服务。
3. **AI 成片面板**：
   - 两个扩展卡片显示就绪状态。
   - 参数表单由清单生成：快速参数置顶，其余参数分组折叠，`when` 规则生效。
   - 有文案框和开始按钮。
   - 进度树按段 → 黑板 → 元素展开，显示排队、生成中、等待网络、失败。失败项可重试；可能已扣费的要确认后才重试。
   - 支持元素级编辑：改描述、挑候选、再来一张、上传图片。支持改段落文案。
4. **黑板手绘**（3000 字）：
   - 配音先入轨。
   - 黑板逐块出现：底板；手写元素为带粉笔手的 reveal 图层；渐显 / 盖章元素为静帧加入场动效。
   - 字幕入轨，换板时交叉淡化。
5. **可靠性**：断网自动恢复，不报错、不重复扣费；重启软件后续跑。
6. **尊重用户编辑**：用户删掉的片段不复活；用户的移动和缩放保留；撤销一次撤掉一整批 AI 入轨。
7. **海绵宝宝口播**：保持旧管线的视觉语言，同样渐进入轨：
   - 开场红字碎裂接泡泡转场；
   - 角色、道具、气泡、花字逐镜出场；
   - 转场音效独立成轨；
   - 三段 BGM。
8. **导出**：选输出路径和预设，得到 mp4。要求：
   - 画面与预览一致（同一份合成代码）；
   - 响度 −16 ±1 LUFS，真峰值 ≤ −1 dBTP，时长误差 ±1 帧；
   - 质检报告无红项；
   - 有进度，可取消。

---

## 2. 真实现状（已完成，带测试，可直接依赖）

| 模块 | 内容 | 关键文件 | 测试 |
| --- | --- | --- | --- |
| AI 服务 | 虎妞对话 / 生图、火山 TTS（详见表后） | `crates/todo_ai/` | 40 项；`tests/service.rs` 故障注入 |
| AI 设置 | 面板保存配置与 key；地址或代理变化时，`ProductionHost::reconfigure` 重启服务并重新提交在途任务 | `todo_ui/widgets/ai_settings.rs`、`todo_app/src/runtime/mod.rs::save_ai_settings` | interaction 测试 |
| 进程 | `Duplex` 写入不阻塞（独立线程）；Job Object 限内存，随宿主一起退出 | `crates/todo_process/` | 7 项 |
| 扩展运行时 | 注册表、安装 / 卸载 / 打包、运行时解析、进程池（详见表后） | `crates/todo_workflow/` | `tests/install.rs`、`pool.rs` 内测试、`examples/pool_probe.rs`（调试工具） |
| 自检 | 启动时只握手；完整自检在临时目录跑样例，不写包目录（详见表后） | `todo_workflow/src/check.rs` | `todo_app/tests/e2e_productions.rs` |
| Python SDK | `Worker`、`ctx.call` / `call_async` / `emit` / `progress` / `path` / `check_cancelled`；作者的 `print` 输出到 stderr | `extensions/sdk/python/todo_worker.py` | 同上 |
| 核心片段 | `Clip.intro` / `audio` / `origin` / `outro`（结尾淡出秒数）；`picture_transform` 统一入场与淡出；`MediaItem.reveal: RevealInfo`（softMax、粉笔手图与轨迹）；`AlphaPacking::{Stacked, Reveal}` | `todo_core/src/model/{clip,effects,media}.rs` | core 测试 |
| 批量入轨 | `EditCommand::ApplyGenerated(Vec<GeneratedEdit>)`：Place / Update / Move / Remove 原子执行，一次撤销；slot（如 `visual:2`）加锚点（`Anchor::Track` / `Anchor::Slot`）保证同一成片内的层序 | `todo_core/src/edit/generated.rs` | `todo_core/tests/generated_edits.rs`（5 项） |
| 对账 | `placement::plan / record`（规则见表后） | `todo_production/src/placement.rs` | `todo_production/tests/placement.rs`（6 项） |
| 口播排期 | 稳定 key、交叉淡化、车道组、相对时间烘焙（详见表后） | `todo_production/src/narration.rs` | `tests/flow.rs` 后两项 |
| 成片用例 | 口播编排与 workflow 运行（详见表后） | `todo_application/src/productions/`、`projects/productions.rs` | `productions/tests.rs`（3 项） |
| 宿主装配 | 产物落盘、配音、保存、worker 调用、续跑（详见表后） | `todo_app/src/adapters/production/{host,bridge}.rs` | bridge 单测 |
| 面板 UI | 扩展卡片、清单生成表单、成片进度卡片（详见表后） | `todo_interaction/src/presentation/production.rs`、`todo_ui/src/widgets/production.rs`、`todo_app/src/runtime/productions.rs` | interaction 测试 |
| 黑板烘焙器 | 真实移植旧渲染器，手写元素输出 reveal 图集（详见表后） | `extensions/builtin/chalk-hand/worker/bake.py`、`worker/blackboard/{scripts,assets}` | `e2e_productions.rs`（样例）、`offline_production.rs` |
| reveal 格式 | `todo-reveal/1`：左半直通 RGBA 定格画面；右半 R/G 为 16 位出现时刻，B 为柔化（乘 softMax 秒），A 表示是否参与；iTXt `todo-reveal` 存元数据。探测时识别（`metadata/mod.rs`）；`compose_reveal(color, timing, &meta, t)` 是 CPU 参考实现 | `todo_media/src/reveal.rs` | 2 项 |
| 原生预览 | reveal 着色器（对 4 个时间纹素逐个算可见度后双线性混合）；packed-alpha 着色器；协议 `layer reveal:<soft> <进度0..1> …`；`.reveal.png` 始终保持 BGRA，不转 NV12；上报宽高比时按半幅修正 | `native/src/{presenter.*,video.*,preview_host_main.cpp,decoder.cpp}`、`transform/visual_transform_shader.h` | `presenter_shader_tests.exe`（WARP 真实像素）、`native/tests` 26 项 |
| 离线渲染 | `preview_host.exe --render`：同一套解码器和着色器做离屏绘制（附录 D）；Rust 端为 `todo_preview::OfflineRenderer` | `preview_host_main.cpp::RenderMain`、`presenter::bindOffscreen / renderOffscreen` | `native/tests/test_render_protocol.py`（4 项） |
| 导出（**未接 UI**） | `todo_app::adapters::export::export_timeline`，流程见表后 | `todo_media/src/export/timeline.rs`、`todo_app/src/adapters/export.rs` | `offline_production.rs` 导出段（需要 `TODO_PREVIEW_HOST`，见 3.1） |

表中「详见表后」各项：

- **AI 服务**：错误分类加重试、熔断、日志恢复、内容寻址产物、付费请求的「结果未知」重试预算。
- **扩展运行时**：
  - 运行时解析会剥离密钥类环境变量。
  - 进程池负责握手、进程复用、call / reply、预算、超时、取消、崩溃重试。
  - 启动失败的 worker：排队中的 run 重试一次后报错，并附上 stderr。
- **自检**：
  - 启动时用 `check_extension`，只握手。
  - `check_extension_fixtures(ext, paths, workdir)` 在临时目录把 `tests/input.json` 跑两次，比较两次结果是否一致，不写包目录。
  - `load_and_check` 用 `cache/extension-checks/`。
- **对账规则**：
  - 只处理本成片的片段（按 `ClipOrigin.production` 判断）。
  - 新 key：放置。同 rev：不动。新 rev：原位替换。
  - 用户改过的 transform 和位置保留。记录的是生产者设置的值，不是用户的值。
  - 用户删除的条目写墓碑，rev 变了才重新放。
  - 带 link 的条目跟随父片段移动。
  - `complete` 为真时，移除不再产出的条目。
  - 媒体未登记的先 adopt。
- **口播排期**：
  - key 使用文档 id：`<段>/voice`、`<段>/sub/<n>`、`plate`、`<板>/<元素>`。
  - 板与板之间交叉淡化；跨段时在段间空隙里淡入。
  - 相邻黑板用不同的车道组（每组 4 道）。
  - 烘焙输入用相对黑板起点的时间，所以平移黑板不必重烘。
  - 字幕字号 0.055，y = +0.38。
- **成片用例**：
  - 入口：`Workspace::add_production / edit_production / retry_production / remove_production / production_status`。
  - 口播编排：分段 → 配音 → 分镜 → 生图 → 底板 → 烘焙 → 条目 → 入轨。
  - workflow：run 运行与条目累积。
  - 同一请求可同时服务多个目标；失败的付费请求不会自动重发。
  - 每批入轨后自动保存工程；用户正在拖动时暂停入轨。
  - adopt 失败会被记住，不会死循环。
- **宿主装配**：
  - 产物以 `services/<TaskKey>.<ext>` 硬链接进成片工作目录 `data/<工程>/productions/<id>/work/`。文件按内容寻址，不会串。
  - 配音读出 PCM 交给用例。
  - 成片在独立线程保存，新版本覆盖旧版本。
  - worker 的 call 经 bridge 处理；encode 和 probe 在独立线程运行。
  - 工程打开时 `resume_productions` 续跑。
  - 已修复：workdir 改为规范化路径比较；`service_file` 命名；同一 key 对应多个 call。
- **面板 UI**：
  - 扩展卡片显示状态和问题说明。
  - 参数表单由清单生成。
  - 每个成片一张卡片：进度条、注意事项、失败项重试（可能已扣费的标为「确认后重试」）、树形状态、移出列表。
- **黑板烘焙器**：
  - 真实移植旧 `RegionStreamRenderer`。手写元素记录每个像素的出现时刻，生成 reveal 图集和粉笔手轨迹，自检误差约 0.6%–0.9%。
  - 渐显 / 盖章元素：静帧加入场动效。
  - `plate` 操作生成黑板底。
  - 局部窗口化后，每个元素约 7 s（原来 51 s），输出逐字节一致。
- **导出流程**：
  1. 逐帧用 `export_layers` 加 `picture_layers` 算出图层，文字同步栅格化；
  2. 交给 OfflineRenderer 绘制；
  3. `VideoSink`（FFmpeg rawvideo）编码视频；
  4. `mix_audio` 混音：滤镜图加两遍 loudnorm；
  5. `mux` 合并音视频；
  6. `timeline_report` 出质检报告。

**离线端到端测试**（不花钱）：`cargo test -p todo_app --test offline_production`。它用真实的宿主、进程池、Python 烘焙器和媒体登记，把一个黑板手绘成片跑完，然后断言：

- 时间轴上有 plate、配音和字幕；
- 手写元素是带粉笔手的 reveal；
- 拖到书写中途时，预览送出了 reveal 层（进度在 0 和 1 之间）和手层。

**历史基线**：该数字来自早期接力快照；当前工作区应以最近一次 `cargo test --workspace --quiet`、Clippy 和 `native/tests` 实际输出为准。

---

## 3. 已知问题与坑（接手先处理）

1. **导出质检里 PSNR 的测量方法有误。**
   - `todo_media/src/export/quality.rs::measure_psnr` 测出 16–19 dB；但用 numpy 在 RGB 下直接比较第 16 帧，结果是 38.8 dB（Draft 预设）。所以问题出在测量，不在画质。
   - 怀疑三处：图片序列与视频的帧对齐 / 时间基（`-framerate` 与 select）；`format=gbrp` 对带 alpha 的 PNG 的处理；psnr 按 YUV 平均的口径。
   - 建议改法：在 Rust 里用 `image` 读参考 PNG；用 FFmpeg 把对应帧解码成 rgb24 原始数据（`-vf select=eq(n\,K) -f rawvideo -pix_fmt rgb24`）；自己算 RGB PSNR。抽 5 帧（含第一帧），每帧标准预设 ≥ 40 dB、Draft ≥ 35 dB。
   - 复现：
     ```powershell
     $env:TODO_PREVIEW_HOST = "$pwd\native\build\reveal\preview_host.exe"
     $env:TODO_EXPORT_KEEP_PARTS = 1
     cargo test -p todo_app --test offline_production
     # 中间文件在 target/offline-production/export/e2e.parts/
     ```
2. **`native/build/bin/preview_host.exe` 还是旧版**：没有 reveal / packed，也不支持新的渲染协议。用户关闭 app 后需要执行 `native\build.cmd` 更新它；`build.ps1` 打包也依赖这个文件。在那之前，测试和导出都用 `TODO_PREVIEW_HOST` 指向新构建。
3. **导出没接 UI**：`MediaTask::Export` 和导出窗口仍是「选 PNG 帧目录」的旧做法（`todo_media::export_frames`），需要替换，见 X 节。
4. **配音时间轴有两套算法**：bridge 的 `extension/services.rs::speech_result` 用原文建时间轴，口播用例的 `work::apply_voice` 用 `sanitize_for_speech(text).spoken`。应抽出一个公共函数，两边共用。
5. **烘焙器回退未实现**：reveal 自检不合格时只计数（`fallbacks`），没有退回逐帧 packed-alpha 视频。盖章的粉笔灰也还没做。
6. **海绵宝宝 worker 是玩具**：`extensions/builtin/spongebob-koubo/worker/main.py` 只有 169 行，离线时生成合成的 WAV 和 PNG。旧管线的设计、生图、分层、转场都没有移植。见 K 节。
7. **进程池测试覆盖不足**：只测了握手完成和启动失败。还缺：call、调用未声明的服务、预算、坏条目、崩溃重试、慢调用不超时、取消、进程复用。
8. **面板缺元素级编辑**：用例 API 已经有了（`Workspace::edit_production(id, &[production::Edit])`），UI 还没有入口。
9. **`config/ai.json` 已被 gitignore**：首次运行时没有这个文件就用内置默认值（已支持）；打包时不要带上它。

---

## 4. 剩余工作（按优先级；每步写明完成标准）

### X 导出接入（最高优先）

- **X1 修正 PSNR 测量**：按 3.1 的建议改。
  - 最小验证：`offline_production` 的导出段在 Draft 预设下全绿；新增单测——同一张图编码后再解码，PSNR ≥ 40 dB。
- **X2 应用层任务**：
  - 把 `MediaTask::Export{frames,…}` 换成 `MediaTask::ExportTimeline { project: Box<Project>, output: PathBuf, preset: String }`。
  - 新增 `MediaOutput::ExportProgress { done, total }`；`Exported` 带上未通过的质检项。
  - 新增 `AppAction::ExportTimeline { output, preset }`：由 Workspace 取当前工程快照后发出 Effect。
- **X3 独立导出线程**：
  - 在 `Workers` 里新增导出线程，同一时刻只跑一个导出，调用 `adapters::export::export_timeline`，每 10 帧发一次进度。
  - 用 `AtomicBool` 支持取消，界面上有「取消」按钮。
  - **不要**放进媒体 worker，否则会阻塞缩略图和 adopt。
  - 删除 `todo_media::export_frames` 和 PNG 目录入口，连同 `tests/export_frames.rs`（改为测试 timeline 模块）。
- **X4 导出窗口**：`todo_ui/widgets/export.rs` 去掉帧目录，改为：
  - 输出路径，用文件对话框选择；
  - 预设：标准 / 高质量 / 竖屏 / 草稿。竖屏要求工程画布是竖屏，否则提示先改工程设置；
  - 进度条、取消按钮，完成后显示报告摘要。
  - 移除 `adapters/mod.rs` 里 `export` 模块上的 `#[allow(dead_code)]`。
- **X5 性能**：目标是 1080p30 下 ≥ 0.8 倍实时。现在是逐帧同步「写命令 → 等像素」。如果太慢：
  - 在 `OfflineRenderer` 里做流水线：先发第 n+1 帧的命令，再读第 n 帧；
  - 原生端解码器按路径复用（现在按 slot）。

X 节的最小验证：

- 3 层夹具工程（视频 + reveal + 字幕 + 两条音频）导出后质检全绿；
- 抽帧与预览截图比较，PSNR ≥ 45 dB；
- 取消后 1 s 内子进程全部退出，半成品被删除。

### K 海绵宝宝口播真实迁移（工作量最大）

**来源**：`D:\Desktop\TO-DO-App\spongebob-koubo.todoext`（是个 zip，可解压到 scratchpad 查看）。

- 管线顺序：`clean → tts → design → assets → render → mix_bgm → accept`。
- 包内说明：`README.md`、`EDIT_WORKFLOW.md`、`ACCEPTANCE.md`、`MIGRATION.md`。
- 核心代码：
  - `workflow/02_镜头设计/{design.py,llm_briefs.py,shot_kinds.py,entrance_recipes.py}`
  - `workflow/03_素材生成/gen_from_brief.py`
  - `workflow/kit/kit/{layers_export,render,stage,typo,bubble,imaging,assets}.py`
  - `workflow/bgm_policy.py`

**做法**：保留旧 Python 的设计和绘制代码，只替换两端。

1. **服务端**：删掉 `kit/llm.py` 中调用 `MISHI_ROOT` / `TTS_TIMELINE_ROOT` 的子进程，改用 SDK：
   - 对话：`ctx.call("chat", {messages, json: true})`；
   - 生图：`ctx.call_async("image", {prompt, size, transparent: true, variant})`；
   - 配音：按段调用 `ctx.call("speech", …)`，每次 ≤ 1200 字，自己拼接时间偏移；
   - 字幕：`ctx.call("subtitles", …)`。
   - 删掉包内所有重试和超时逻辑，这些由宿主负责。
2. **输出端**：`layers_export` 不再编 VP9 WebM。
   - 普通元素：静帧 PNG 加宿主入场动效。映射表见[设计稿](2026-10-17-ai-extensions-design.md) K 节；fade、rise、pop、slide、drop、stamp、grow 在 core 里都已实现。
   - 复杂入场、开场红字碎裂、泡泡转场：生成逐帧 PNG，再用 `ctx.call("encode", {frames, fps, alpha: true})` 编成 packed-alpha。
   - 转场音效：打包时预先处理好的 `assets/fx/bubble_wipe_sfx.wav`，作为 sfx 条目。
   - BGM：三条 music 条目，`source.in` 为截取点，`gain` 约 0.3，带 `fade{in,out}`。
3. **渐进 emit**：顺序为配音 → 每镜画面 → 转场与音效 → BGM 与字幕。key 约定：
   - `voice/<段>`
   - `shot/<n>/<元素>`
   - `shot/<n>/wipe`、`shot/<n>/wipe/sfx`
   - `bgm/intro|mid|outro`
   - `sub/<n>`
4. **清单**：
   - `kind: workflow`；
   - `requires.services: [chat, image, speech, subtitles, encode]`；
   - `worker.ops: ["run"]`；
   - 写上 `budget`；
   - 默认音色用咪仔。旧默认 `S_RksT6Z6g2` 是作者账号的克隆音。
5. **验收门**：保留 g1 / g3 / g5 / g8，结果写进 `done.result.gates`，由宿主面板显示。

**宿主侧**：用例的 workflow 分支已经有了（`session.rs::tick_workflow`、条目累积，完成后才移除旧条目），但**没有端到端测试**。

- 仿照 `offline_production.rs` 写一个 `offline_workflow.rs`。
- 难点：worker 的 call 走真实 bridge，会把请求提交给 AiService。两种办法：起一个 FakeServer；或者给宿主加一个测试用的 AI 应答注入点。
- 建议在 `ServiceBridge` 上加一个可选的 `Box<dyn Fn(&TaskSpec) -> Option<FakeAnswer>>`，只在测试里构造。
- 离线样例：`tests/sample/input.json` 加上录制好的服务结果。

**完成标准**：

- `check_extension_fixtures` 两次运行结果一致；
- 离线 workflow 测试中，各类条目都入轨；
- 抽 5 个时刻与旧管线的 `成片.mp4` 截图对比，人工确认视觉语言一致，截图放进 `docs/reports/`；
- 字体和 BGM 的授权按第 6 节确认。

### U 面板补全

1. **元素编辑**：在树里点元素后展开以下操作，都走 `Workspace::edit_production`：

   | 操作 | 对应 Edit |
   | --- | --- |
   | 候选图缩略图网格中挑选 | `Edit::PickVariant` |
   | 再来一张 | `Edit::AddVariant` |
   | 改描述 | `Edit::SetElement{patch}` |
   | 上传图片（先把文件复制进 workdir，再登记相对路径） | `Edit::UploadVariant` |
   | 改段落文案 | `Edit::SetSegmentText` |

   最小验证：interaction 测试确认 UI 动作翻译成了正确的 Edit；用例测试确认改描述只产生 1 次生图和 1 次烘焙（已有类似测试）。
2. **等待网络**：显示原因和倒计时（`TaskState::Waiting(reason)`）。
3. **成本**（可选）：显示已花和预计花费，按任务数估算。

### C 黑板烘焙器补全

1. **自检回退**：`verify` 返回 false 时，重放书写，逐帧写出直通 RGBA PNG，再用 `ctx.call("encode", {alpha: true})` 编码，条目改为指向 mp4。
   - 最小验证：构造一个会被擦除的元素，确认走了回退路径，且画面正确。
2. **盖章粉笔灰**：
   - 用旧 `render.py` 的 `_make_dust` / `_dust_alpha` 生成逐帧 PNG，再 encode。
   - 作为附加条目 `<元素key>/dust`，放在相邻车道组，`link` 指向元素。
3. **性能**：单元素约 7 s。`_write_frame` 里的分档统计约 3 s，可以改为只更新变化区域。清单的 `maxProcesses: 4` 已允许 4 块黑板并行。

### P 其它收尾

1. 统一配音时间轴，见 3.4。
2. 补足进程池测试，见 3.7。
3. bridge 测试：用 todo_ai 的 FakeServer 覆盖 chat 往返、图片落盘、503 后自愈、重跑不重复请求。
4. 文档：
   - `extension-spec.md` 和 `native-media.md`：补上附录内容。
   - `responsibilities.md`：补上 `edit/generated.rs`、`query/reveal.rs`、`productions/`、`adapters/export.rs`、`todo_preview::render`。
   - 删除或更正 0.10 提到的两份报告。
5. 打包：
   - `build.ps1` 复制 `extensions/builtin/**`（不含 tests）、`extensions/sdk/**`、`dependencies/python/**`。
   - 检查产物里没有 `密匙`、`MISHI`、`api_key=`、`sk-`。
   - 原生宿主用最新构建。

### Q 线上验收（须用户同意，花费几元）

- 在真实 app 里各做一次，全程不用命令行：黑板手绘 300 字、海绵宝宝 224 字。
- 中途拔网线 2 分钟，验证能自动恢复。
- 导出后质检全绿。
- 写报告到 `docs/reports/`：请求数、重试次数、耗时、花费、截图。

---

## 5. 关键代码入口

- **开始成片**：UI 发出 `ProductionAction::Start`
  1. `App::start_production`（`runtime/productions.rs`）；
  2. `ProductionHost::create`：解析清单参数、建文档、计算 setup；
  3. `Workspace::add_production`。
- **用例循环**：
  - `Workspace::advance_productions`：各会话 `tick`，然后 `place_productions`。
  - 宿主回报在 `frame.rs` 转成 `AppEvent::Production`，交给 `production_event`。
  - adopt 回执在 `projects/events.rs` 里触发 `advance_productions`。
- **一帧画面**：`todo_application::picture_layers(project, hits, text)`。预览传入 `resolve_layers` 的结果，导出传入 `export_layers` 的结果。
- **宿主执行**：`ProductionHost::execute(ProductionTask)` 处理 Submit / Run / Save / Cancel / Retry / CancelRun；`poll()` 返回 `HostReport`。
- **调试 worker**：
  ```powershell
  cargo run -p todo_workflow --example pool_probe -- <root> <包目录> <op> <workdir> <input.json>
  ```

---

## 6. 需要用户拍板

1. **海绵宝宝字体**：`字魂剑气手书` 需要商用授权；`AmezumiGothic` 有 130 MB。上一位 agent 的报告说「OFL 字体已完成」，需要核实包里实际用了哪些字体。
2. **三段 BGM 的授权**：上一位的报告称用户已经授权，接手前再向用户确认一次。
3. **线上验收（Q 节）的花费**。

---

## 附录

### A. `todo-bake-board/1`（宿主传给 `bake_board` 操作的 input）

```jsonc
{ "spec": "todo-bake-board/1", "board": "b3",
  "canvas": {"width":1920,"height":1080,"fps":30},
  "duration": 9.2,          // 黑板显示时长（秒，相对黑板起点）
  "fadeIn": 0.5,
  "params": {"layout","tone","enter","stroke","fill","hand","handSize","drawSpeed"},  // 取自成片参数
  "elements": [{"id":"e7","key":"b3/e7","role":"hero","label":"…","draw":"…","weight":1,
                "file":"services/<key>.png","lane":1,"at":0.4,"windowEnd":3.9}] }
```

- worker emit 的条目，`start` 相对黑板起点。平移、时长、淡出由宿主处理：`fade.out` 等于下一块黑板的淡入时长。
- `plate` 操作的 input 为 `{key:"plate", canvas, tone}`，emit 一条 lane 0 的条目。
- 清单 `narration` 新增可选字段 `plate`（操作名，黑板手绘为 `"plate"`）。

### B. 条目补充（规范 §8 增量）

- 画面 / 文字条目可以带 `fade: {in: 0, out: s}`，表示结尾淡出（对应 core 的 `Clip.outro`）。
- 音频条目的 `fade` 表示淡入淡出。
- `intro.kind` 取值：`fade | rise | pop | slideLeft | slideRight | drop | stamp | grow`。

### C. `todo-reveal/1`

格式见第 2 节的 reveal 行。元数据：

```json
{"spec":"todo-reveal/1","width":W,"height":H,"duration":秒,"softMax":秒,
 "hand":{"file":"hand_chalk.png","anchor":[ax,ay],"height":相对元素高,"path":[[t,x,y],…]}}
```

- 手图与图集放在同一目录。
- 探测后存进 `MediaItem.reveal`。
- 粉笔手由 core 的 `reveal_hand` 算成一个普通图片层；笔画结束后 0.15 s 内淡出。

### D. 原生协议

**预览层**：

```text
layer <video|image|packed|reveal:<soft>|text> <time> <x> <y> <scale> <sx> <sy> <opacity> <path>
```

reveal 层的 time 是进度（0..1），soft 是柔化宽度占时长的比例。

**离线渲染**：`preview_host.exe --render`

- stdin 依次发送：`begin <w> <h>`；每帧一组 `layers-begin`、若干 `layer` 行、`layers-commit`；最后发 `end`。
- 每次 commit，stdout 输出 w×h×4 字节的 BGRA。
- 解码一律精确。任一层失败时退出码非 0，原因写到 stderr（例如 `cannot open layer N`）。
