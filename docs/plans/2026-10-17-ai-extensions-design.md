# AI 与扩展接入设计稿（v2，参考用）

> 进度与接力以 [2026-10-17-ai-extensions.md](2026-10-17-ai-extensions.md)（v3）为准；本文保留完整的设计细节（入场映射、质量门禁、协议设计），其中“当前实现进度”一节已过时。

更新：2026-10-17。本文**取代 v1 计划**，是后续 agent 的施工图。目标：TO DO 安装后**开箱即用**，用户只需在「AI 设置」里填服务地址和 key，就能用两个内置扩展「黑板手绘」与「海绵宝宝口播」从文案一键做出可编辑、可导出、质量有保证的成片。

配套规范（施工中以它们为准，改了规范必须同步代码测试）：

- [扩展规范 v1](../architecture/extension-spec.md)：清单、`todo-worker/1` 协议、`todo-items/1` 条目。代码 `crates/todo_production/src/extension/`。
- [AI 服务接入](../architecture/ai-services.md)：`todo_ai` 的执行模型、错误分类、服务商契约。
- [项目文件夹分工](../architecture/responsibilities.md)、[共用编辑能力层](../architecture/shared-control.md)、[时间轴编辑](../architecture/timeline-editing.md)、[花字层标准](../architecture/text-layer.md)、[Rust 媒体处理](../architecture/native-media.md)。

---

## 0. 先读：硬约束

违反下面任何一条的改动都不合格。

1. **分层**：依赖方向见 `responsibilities.md`，由 `crates/todo_application/tests/dependency_boundaries.rs` 强制检查。规则：
   - core 和 application 不做 I/O、不联网、不开线程。
   - `todo_production` 是纯领域代码。
   - 只有 `todo_ai` 联网。
   - 只有 `todo_process` 启动子进程（`todo_media` 和 `todo_extensions` 都经过它）。
   - app 只做装配。

   新增依赖时必须同时修改边界测试，并写明层次归属。同一条规则只能有一份实现：比如入场曲线，预览和导出共用 core 的同一个函数。
2. **产品里不用 Node**。旧扩展的 `checks/*.mjs`、`scripts/*.mjs` 一律不迁移。只为生成一次性对照数据的 Node / Python 脚本放 scratchpad，不进仓库。
3. **密钥**：
   - 只存 Windows 凭据管理器（`todo_ai::secrets`）或环境变量 `TODO_HUNIU_API_KEY` / `TODO_VOLC_API_KEY`。
   - 不进工程、配置、日志、安装包、测试夹具和截图。
   - 显示只用 `Secret::masked()`。
   - 扩展工作程序**拿不到任何密钥**：启动时剥掉名字里含 `KEY` / `TOKEN` / `SECRET` / `PASSWORD` 的环境变量。
   - 运行时**不读** `D:\Desktop\密匙` 或任何外部工具目录。
4. **花钱的线上调用**只能用 `cargo run -p todo_ai --example ai_probe -- <tts|chat|chat-json|image>`。每次只跑一项，先告诉用户，并把花费写进汇报。端到端「真跑一次」只在 Q3 做，且须用户同意。
5. **不提交**，除非用户明确要求。不杀、不覆盖用户正在运行的 `todo_app.exe` / `preview_host.exe`；原生构建输出到独立目录（`native\build.cmd build\<名字>`）。
6. **测试先行**：每步先写会失败的测试，再实现。每步完成后跑：
   ```powershell
   cargo fmt --all
   cargo test --workspace
   cargo clippy --workspace --lib --bins --examples -- -D warnings
   # 涉及原生：
   native\build.cmd build\<独立目录>
   python -m unittest discover -s native/tests
   ```
   测试里的子进程用 `tests/support`，即测试可执行程序自身，不依赖脚本宿主。Python 工作程序的测试在找不到 Python 时 `skip`，并打印原因。
7. **换行**：仓库开了 `core.autocrlf=true`，工作区文件可能是 CRLF。脚本里做精确字符串替换前先统一换行，优先用编辑工具；Python 写文件用 `write_bytes` / `newline=""`，避免把 LF 文件改成 CRLF。
8. 旧项目 `D:\Desktop\TO-DO-App` **只作参考**，可以读、复制算法，但运行时不能引用其中任何路径。

---

## 1. 终态与验收

### 1.1 用户路径（必须全程无需命令行）

1. 解压或安装 `dist/to_do`，双击 `todo_app.exe`。不要求用户装 Python、FFmpeg、Node。
2. 打开「AI 设置」：
   - 虎妞：服务地址（默认 `https://huniuai.com`）、API Key；
   - 火山：服务地址（默认 `https://openspeech.bytedance.com`）、API Key、默认音色；
   - 点「保存」。key 显示为掩码，并标明来源（凭据管理器 / 环境变量）。
3. 新建或打开工程 → 打开「AI 成片」面板，看到两个内置扩展卡片，各带就绪状态：
   - 缺 key → 「去设置」；
   - 运行时损坏 → 「修复」。
4. 选「黑板手绘」，粘贴 3000 字口播，参数保持默认，点「生成」：
   - 配音先出现在时间轴；
   - 每块黑板随后陆续出现（底板、逐元素书写图层、字幕）；
   - 面板实时显示每段 / 每块的状态。
5. 断网 10 分钟再恢复：
   - 面板显示「等待网络」，**不报错、不需要点任何按钮**；
   - 网络恢复后自动继续，已完成的部分不重做、不重复扣费。
6. 关闭软件再打开：已完成的结果直接显示，未完成的自动续跑。生图只续下载，不重新提交。
7. 局部编辑，都只重做受影响部分，其他片段（含用户的手动移动 / 缩放）保持不动：
   - 改一个元素描述 → 只重画这一张；
   - 「再来一张」；
   - 挑候选；
   - 上传自己的图；
   - 改一段文案 → 只重配这一段；
   - 删掉的片段不会被「复活」。
8. 「导出」→ 选预设（1080p30 标准 / 高质量 / 竖屏）→ 得到 mp4：
   - 画面与预览逐帧一致；
   - 响度达标；
   - 自动质检报告无红项。
9. 选「海绵宝宝口播」走同样流程，结果保持旧管线的视觉语言：
   - 开场红字碎裂 → 泡泡转场揭开；
   - 角色、道具、气泡、花字按短语出场；
   - 转场音效在独立音效轨；
   - 三段 BGM 分轨。

### 1.2 两个内置扩展的质量标准

| 项 | 黑板手绘 | 海绵宝宝口播 |
| --- | --- | --- |
| 呼吸感 | 写完后停留 p10 ≥ 1.0 s；换板 → 第一笔的中位数 ≥ 0.5 s；换板只发生在口播停顿内（`pacing.rs` 已实现，P2 入轨后用 `examples/pacing_report.rs` 复测） | 每镜入场错开 ≥ 0.25 s；开场红字句不叠字幕 |
| 画面 | 粉笔元素真透明、无黑边 / 绿边；书写回放逐帧确定，拖动零解码；粉笔手跟随笔尖 | 透明层一律 PNG 或 packed-alpha，**禁止**近黑抠像；转场边缘平滑 |
| 字幕 | 每条 ≤ `subtitleChars`，与逐字时间戳对齐，误差 ≤ 1 帧 | 同左，`intro_red` 句无字幕 |
| 声音 | 口播 −16 LUFS ±1，真峰值 ≤ −1.5 dBTP | 口播同左；BGM 按计划为 0.3 倍左右，淡入淡出；转场音效对齐泡泡 |
| 可编辑 | 每个元素一个独立片段（可挪、可缩、可删、可重画） | 每个角色 / 道具 / 气泡 / 花字一个独立片段 |
| 稳定 | 断网、网关 5xx、限流、重启都自动恢复；同一请求只付一次钱 | 同左 |
| 速度 | 3000 字、默认参数：首条配音入轨 ≤ 60 s；不含生图等待的本地计算 ≤ 成片时长的 0.5 倍 | 224 字样稿全流程 ≤ 10 分钟（旧管线实测 9.4 分钟） |

### 1.3 工程指标

- 预览：多层（≤ 16 层，解码视频 ≤ 6，文字 ≤ 4）拖动保持现有水准（代理 60 fps 更新、最大卡顿 ≤ 26 ms）。reveal 层不得引入解码。
- 导出：1080p30 在 8 核机器上 ≥ 0.8 倍实时（30 s 片 ≤ 40 s）；导出帧与预览合成同一份代码。
- 所有后台任务可取消，取消后 1 s 内子进程退出。应用崩溃时子进程随之退出（Job Object）。

---

## 2. 现状（2026-10-17，可直接依赖）

| 模块 | 内容 | 测试 |
| --- | --- | --- |
| 预览层数 | core `MAX_PREVIEW_LAYERS = 16`（解码 ≤ 6、文字 ≤ 4），协议与原生 `kMaxLayers` 同步；修复了 5 层起报错 | `todo_core/tests/preview_layer_budget.rs`，`todo_app` 的 `preview_layer_budget_matches_the_protocol`，`native/tests/test_layer_budget.py` |
| 拖动预览 | 解码器分级 seek（Exact / Partial / Abandoned）、按流 worker、全 I 帧编辑代理 | `native/tests/*`，`docs/reports/2026-10-16-scrub-performance.md` |
| `todo_process` | `supervise` / `capture`（超时、取消、双管道排空、有界诊断）；`Duplex`：长驻子进程，stdin 由独立线程写入，**写入永不阻塞调用方**；Windows Job Object 绑定与内存上限 | `crates/todo_process/src/tests.rs`（7 项）；Job/媒体/扩展编译与 workspace 测试 |
| `todo_ai` | 见下 | 40 项，含 `tests/service.rs` 本地故障注入；`-- --ignored` 跑真实凭据管理器 |
| `todo_production` | 见下 | `golden`、`timing_golden`、`pacing`、`flow`、`task_keys`、`extension_spec` |
| 扩展包 | `extensions/builtin/chalk-hand/`：清单 `builtin.chalk-hand` 4.0.0、`prompts/planner.md`、`prompts/asset.md` | `extension_spec::the_chalk_hand_package_is_valid` |
| `todo_extensions` | 清单/安装、运行时环境隔离、按请求指纹预算、`todo-worker/1` 进程池 | `tests/install.rs`（3 项）及 pool/runtime/budget 单测 |

`todo_ai` 内容：

- 配置 `config/ai.json`，内置默认值，模板 `config/ai.example.json`；
- 凭据管理器密钥链，环境变量可覆盖；
- 错误分类、带抖动的退避、截止时间；
- 按服务商熔断；
- 每任务一个原子 JSON 日志，重启恢复，有「结果未知」付费重试预算；
- 内容寻址产物仓库，可续传，校验后才发布；
- 火山 TTS、虎妞对话（SSE、JSON 模式）和生图适配器；
- `AiService`：独立线程上跑单线程 tokio，接口 `submit` / `cancel` / `retry` / `set_key` / `poll` / `artifact_path`；
- 示例 `ai_check`（免费自检）、`ai_probe`（付费探测）、`import_env`（开发者一次性导入）。

`todo_production` 内容：

- 任务 `TaskSpec` / `TaskKey`；
- 分段 `segment`；
- 口播清洗与三级时间轴 `voice`（含按真实音频修正、句边界吸附）；
- 字幕 `subtitles`（切条、对时、静音检测）；
- 分镜 `storyboard`（含 `BREATHING_RULE`）；
- 呼吸感排期 `pacing`；
- 成片文档 `production`（`todo-production/2`，整批编辑）；
- 任务规划与回填 `work`（`plan_work`、`apply_outline` / `apply_voice` / `apply_storyboard` / `apply_image`，以任务指纹判断过期）；
- 扩展规范类型与校验 `extension`（`manifest`、`protocol`、`items`、`narration_spec`）。

`todo_extensions` 内容：

- `registry`：扫描内置 / 用户扩展，同 id 时用户版覆盖内置版；坏包列入 `rejected`，不可运行；`Extension::build_key()` 为 `id@version`；
- `install`：`.todoext` 先在内存里校验路径 / 大小 / 数量 / 重复项 / 清单，再解压到临时目录原子换入，失败回滚；
- `uninstall`；
- `pack`：跳过隐藏文件和 `__pycache__`。
- `runtime`：解析包内 Python/native 入口，移除 key/token/secret/password 环境变量，注入 SDK 路径；
- `budget`：同一服务请求指纹只计一次，按清单限制独立请求数；
- `pool`：工作程序握手、按 build key 复用、FIFO、call/reply、条目校验、取消、超时和资源故障换进程重试。

**已知缺口**，就是本指南要做的：

- 服务桥；
- 内嵌 Python；
- 成片存储与用例；
- 入轨对账；
- reveal / packed-alpha / 入场 / 音频增益淡变；
- 两个扩展的工作程序；
- AI 设置和成片界面；
- 导出；
- 打包。

`chalk-hand` 清单引用的 `worker/bake.py` 还不存在，所以目前 `Registry::scan` 会把它列为 rejected，这是预期行为。

---

## 3. 总体架构

### 3.1 线程与进程

```text
UI 线程（eframe）
  Workspace（application）── 持有工程会话 + ProductionSession（成片用例状态机，纯逻辑）
     │ Effect::Production(ProductionTask)          ▲ AppEvent::Production(ProductionOutput)
     ▼                                              │
app/adapters/production/ ProductionHost（独立线程，只做 I/O 与转发）
  ├─ AiService（todo_ai 线程，tokio）       ← chat / image / speech
  ├─ WorkerPool（todo_extensions 管理线程） ← 扩展工作程序子进程（Job Object）
  ├─ ServiceBridge                         ← 把工作程序的 call 转成 AiService 任务或本地服务
  ├─ ProductionStore（todo_storage）        ← 成片文档、工作目录
  └─ MediaService（todo_media）             ← probe、encode（packed-alpha / H.264）、素材登记
preview_host.exe（原生）                    ← 预览合成；导出时以 --render 模式离线合成同一套着色器
ffmpeg.exe                                  ← 编码、混音、响度
```

原则：

- **决策在 application，执行在 adapter**。ProductionHost 不判断「下一步做什么」，只执行 Effect、回传结果。
- 「下一步」由 `todo_production::work::plan_work` 和 application 的 `ProductionSession` 决定。

### 3.2 两种扩展的数据流

**口播配方**（`kind: narration`，黑板手绘）由宿主编排：

```text
文案 ─ plan_work ─┬─ Outline(chat) ─ apply_outline ─ 分段
                  ├─ Voice(speech) ─ apply_voice ──── 时间轴 + 字幕条 ─┐
                  ├─ Storyboard(chat) ─ apply_storyboard ─ 黑板/元素 ──┤
                  └─ Image(image) ─ apply_image ─────── 候选图 ─────────┤
                                                                       ▼
                  pacing::schedule ─ 烘焙 run(bake_board) ─ 工作程序 emit 条目
                                                                       ▼
                  narration_items（底板 / 配音 / 字幕）+ 烘焙条目 ─ reconcile ─ 时间轴
```

**通用工作流**（`kind: workflow`，海绵宝宝）由工作程序编排：

```text
宿主 run(op="run", input={script, media}, params) ─▶ 工作程序
工作程序 ─ call(chat/image/speech/subtitles/probe/encode) ─▶ ServiceBridge ─▶ reply
工作程序 ─ emit(items) ─▶ 宿主立即 reconcile 入轨（渐进）
工作程序 ─ done(result) ─▶ 宿主保存 run 状态
```

重跑时，已完成的服务调用由 AiService 日志按 `TaskKey` 立即返回，所以「只重做改动部分」对 workflow 扩展自动成立。

### 3.3 运行时目录（相对 app 根 = `repo_root()`）

```text
config/ai.json                      AI 非密钥设置（设置面板写入；缺省用内置默认值）
cache/ai/tasks/<key>.json           任务日志（todo_ai）
cache/ai/blobs/<2>/<sha256>.<ext>   产物仓库（todo_ai）
extensions/builtin/<id>/            内置扩展（随包，只读）
extensions/user/<id>/               用户安装的扩展
extensions/sdk/python/todo_worker.py  Python 工作程序 SDK（PYTHONPATH 注入）
dependencies/python/                内嵌 CPython + numpy / opencv-headless / pillow / fonttools
dependencies/ffmpeg/bin/            FFmpeg（已有）
data/<工程>/project.json            工程（已有）
data/<工程>/media/                  导入素材（已有）
data/<工程>/productions/<pid>/production.json   成片文档
data/<工程>/productions/<pid>/work/             工作目录（服务结果副本、工作程序产物）
```

服务结果放进工作目录的规则：

- 位置：`work/services/<task-key>.<ext>`；
- 同盘优先 `std::fs::hard_link`，失败再复制；
- 工作程序只看到工作目录内的相对路径。

### 3.4 新代码放哪

| 内容 | 位置 |
| --- | --- |
| 入场曲线、片段音频增益 / 淡变、片段来源 `ClipOrigin`、reveal / packed 的媒体种类 | `todo_core/src/model/`（数据 + 校验）、`todo_core/src/query/`（按时间求值） |
| 条目 → 时间轴的对账规划 | `todo_production/src/placement.rs`（纯函数，输入工程快照，输出编辑计划） |
| 口播配方的条目生成（配音 / 底板 / 字幕 / 烘焙输入） | `todo_production/src/narration.rs` |
| 成片用例状态机、Effect / Event、撤销语义 | `todo_application/src/productions/` |
| 成片面板与 AI 设置的展示模型、输入翻译 | `todo_interaction/src/presentation/production.rs`、`ai_settings.rs`；`todo_interaction/src/production/` |
| egui 控件 | `todo_ui/src/widgets/production/`、`todo_ui/src/widgets/ai_settings.rs` |
| 成片存储 | `todo_storage/src/productions/` |
| 进程池、服务预算、运行时解析 | `todo_extensions/src/{pool,budget,runtime}.rs` |
| 编码（packed-alpha / 短 GOP H.264）、导出混音与质检 | `todo_media/src/{encode,export}/` |
| 装配、线程、事件转发、服务桥 | `todo_app/src/adapters/production/` |
| 离线合成 | `native/src/render/` + `preview_host_main.cpp` 的 `--render` 模式 |

---

## 4. 实施步骤

### 当前实现进度（2026-10-17）

已落地：E1–E5 基础运行时与自检、S1 转换与 S2 服务桥（含 probe/encode 本地媒体服务）、
R1/R2 Python 运行时与 SDK、P1–P5 成片存储/条目生成/对账/Adopt、M1 片段效果与混音淡变、
M2 packed-alpha 编码与 FFprobe 发布校验、M3 reveal 元数据/CPU 参考合成、M4 原生淡变、
U1 AI 设置与 U2 扩展状态面板、黑板 reveal 烘焙器、海绵宝宝离线渐进入轨工作流、原生
`--render` PNG/reveal/packed 软件合成、显式 layer kind 协议、MediaItem packed-alpha 元数据、
PNG 序列导出入口、质量报告数据结构、application 的成片任务/Adopt/导出 worker 装配与 Q1/Q2
打包。
导出入口现在包含 ebur128/true peak、blackdetect、freezedetect、PSNR 测量、取消时的进程监督、
导出窗口和质量报告显示；X5 本地实测已完成，Q3 线上链路和字体/BGM 授权仍待用户重新确认。
剩余工作集中在把最终时间轴逐帧合成器、完整生产参数表单和运行事件树继续接入 application
session。离线验证、宿主基础能力和线上自愈证据已经具备，下面步骤按依赖顺序保留作产品收口。

顺序和依赖：

```text
E（进程池）→ S（服务桥）→ R（Python 运行时 + SDK）
M（媒体能力：core + 原生）与 P1（成片存储）可与 E/S/R 并行
P2–P5（成片用例与入轨）依赖 E、S、M1
U（界面）依赖 P4
C（黑板手绘烘焙器）依赖 R、M3
K（海绵宝宝迁移）依赖 S、R、M1、M2
X（导出）依赖 M
Q（打包与端到端）最后
```

每步格式：**做什么 → 接口 → 最小验证 → 完成标准**。

### E 扩展运行时（`todo_extensions`）

**E1 运行时解析 `runtime.rs`**

```rust
pub struct RuntimePaths { pub app_root: PathBuf, pub python_override: Option<PathBuf> }
pub fn worker_command(ext: &Extension, paths: &RuntimePaths) -> Result<Command, RuntimeError>;
```

python 解释器按以下顺序找：

1. 环境变量 `TODO_PYTHON`；
2. `<app_root>/dependencies/python/python.exe`；
3. 开发机上 PATH 里的 `python`。

Release 构建只认第 2 项，找不到报 `RuntimeError::PythonMissing`，界面显示「修复」。

命令：

- python 型：`python -X utf8 -B -s -u <entry>`。`-u` 必须有，否则 stdout 缓冲会卡住行协议。
- native 型：包内 exe。必须位于包目录内，并且是 `.exe`。
- 当前目录：包目录。

环境变量：

- 从父进程继承，然后：
  - 删掉名字（大写后）含 `KEY` / `TOKEN` / `SECRET` / `PASSWORD` 的变量；
  - 删掉 `TODO_HUNIU_API_KEY`、`TODO_VOLC_API_KEY`；
  - 删掉 `PYTHONHOME`、`PYTHONPATH`、`PYTHONSTARTUP`；
- 再设置：
  - `PYTHONUTF8=1`、`PYTHONIOENCODING=utf-8`、`PYTHONDONTWRITEBYTECODE=1`、`PYTHONNOUSERSITE=1`；
  - `PYTHONPATH=<app_root>/extensions/sdk/python`；
  - `TODO_EXTENSION_ID`、`TODO_HOST_VERSION`。

最小验证：

- 单元测试检查环境：注入 `FOO_API_KEY` 后不出现在 Command 的 envs 里，且出现 `PYTHONPATH`；
- 找不到解释器时报错；
- native 入口越界被拒。

**E2 Job Object（`todo_process/src/job.rs`）**

```rust
pub struct Job { … }               // CreateJobObjectW
impl Job {
    pub fn new(memory_limit: Option<u64>) -> Result<Self, ProcessError>; // KILL_ON_JOB_CLOSE | PROCESS_MEMORY
    pub fn assign(&self, child: &std::process::Child) -> Result<(), ProcessError>;
}
```

- `Duplex::spawn` 增加 `Option<&Job>` 参数，或提供 `spawn_in(job, …)`。
- 工作程序按清单 `worker.memoryMiB` 设内存上限。
- windows-sys 需要加特性 `Win32_System_JobObjects`、`Win32_System_Threading`；`todo_process` 的依赖与边界测试同步更新。

最小验证：

- fixture 模式 `alloc`：分配 2 GiB，在 256 MiB 上限下被系统终止，`Duplex` 收到 `Output::Closed`；
- drop `Job` 后子进程消失（`try_wait` 得到退出）。

**E3 服务预算 `budget.rs`**

```rust
pub struct Budget { … }            // 每个成片一个，跨多次 run 共享（Arc<Mutex<…>>）
impl Budget {
    pub fn new(limits: &BTreeMap<Service, u32>) -> Self;
    /// 同一请求（服务 + 规范 JSON 指纹）只计一次：重跑、崩溃重试命中缓存不扣预算。
    pub fn charge(&self, service: Service, request: &serde_json::Value) -> Result<(), ServiceError>;
}
```

- 指纹用 `todo_production::tasks::fingerprint`。
- 超限时返回 `ServiceErrorClass::Budget`。
- 清单没写的服务不限次数，但只有 `requires.services` 里声明了才能调用；这一检查由进程池做。

最小验证：

- 同请求 N 次只计 1 次；
- 第 limit+1 个不同请求被拒。

**E4 进程池 `pool.rs`**

```rust
pub type RunId = u64;
pub struct RunRequest {
    pub extension: Arc<Extension>, pub op: String,
    pub input: Value, pub params: Value, pub canvas: Canvas,
    pub workdir: PathBuf, pub seed: u64, pub budget: Arc<Budget>,
}
pub enum WorkerEvent {
    Progress { run: RunId, phase: String, fraction: f64 },
    Items { run: RunId, items: Vec<Item> },
    Call { run: RunId, call: String, service: ServiceCall },
    Done { run: RunId, result: Value },
    Failed { run: RunId, error: RunError, stderr: String },
}
pub struct PoolOptions { pub runtime: RuntimePaths, pub hello_timeout: Duration /*30 s*/,
    pub idle_timeout: Duration /*120 s*/, pub cancel_grace: Duration /*5 s*/,
    pub run_timeout: Option<Duration> /*测试覆盖清单 timeoutSec*/ }
impl WorkerPool {
    pub fn start(options: PoolOptions, events: Sender<WorkerEvent>) -> Self;
    pub fn run(&self, request: RunRequest) -> RunId;
    pub fn reply(&self, run: RunId, call: String, result: Result<Value, ServiceError>);
    pub fn cancel(&self, run: RunId);
    pub fn shutdown(self);          // 发 shutdown，2 s 后 kill
}
```

实现要点：

- **单管理线程**，一个收件箱。所有子进程的 stdout 用 `Duplex::spawn(…, sender, |o| Inbox::Output(pid, o))` 汇入同一个 channel。计时器用 `recv_timeout` 驱动：握手超时、运行超时、空闲回收、取消宽限。
- **握手**：
  - 启动后发 `hello{spec, host, package}`；
  - 30 s 内必须收到 `ready`，否则 kill 并报 `Resource`；
  - 校验 `spec == "todo-worker/1"`，且 `ops` 覆盖清单 `worker.ops`。
- **复用**：
  - 进程按 `build_key()` 分组，同组最多 `maxProcesses` 个；
  - 一个进程同一时刻只执行一个 run，其余 run 排队（FIFO）；
  - 空闲 120 s 关闭。
- **call 处理**：
  - 服务未在 `requires.services` 中声明 → 立即 `reply ok:false class:invalid`；
  - `ServiceCall::parse` 失败 → `invalid`；
  - `budget.charge` 失败 → `budget`；
  - 否则发 `WorkerEvent::Call`，由宿主 `reply`；
  - 同一 run 内 call id 重复 → run 失败（`Internal`）。
- **emit 处理**：
  - 用 `validate_items(items, &已发出的 key, |f| workdir.join(f).is_file())` 校验，失败时 run 以 `Internal` 失败并 kill 进程；
  - 通过的条目路径保持相对，由宿主再解析。
- **超时**：只计「没有未答复 call」的活跃时间。等宿主服务（如生图排队）不算工作程序超时。
- **取消**：发 `cancel{id}`，5 s 内没有收到 `failed(cancelled)` 就 kill。
- **故障**：进程崩溃、EOF 或输出非法行时，当前 run 记 `Resource`，**在新进程里自动重试一次**；服务调用由 AiService 日志命中，不重复付费。第二次再失败才发 `Failed`。`stderr` 带最后 8 KiB。
- 收到 `failed{class:resource}` 也重试一次，`invalid_input` / `internal` 不重试。

最小验证：在 `crates/todo_extensions/tests/fixtures/echo/` 放 `extension.json` + `worker/main.py`，用 SDK（R2）或手写行协议。按 `params.mode` 覆盖下列情况，每种一个测试，找不到 Python 时跳过：

| mode | 预期 |
| --- | --- |
| `ok` | progress → emit 2 条 → done；事件顺序正确 |
| `call` | 发 chat call，测试线程 reply 后工作程序把结果写进 done |
| `undeclared` | 调用未声明的 `image`，立即收到 `invalid`，不产生 `WorkerEvent::Call` |
| `budget` | 预算 1 次时第二个不同请求收到 `budget` |
| `bad_item` | emit 指向不存在的文件，run 失败（`Internal`） |
| `crash_once` | 第一次 `os._exit(3)`，自动在新进程重试后成功；进程 id 不同 |
| `slow` | 有未答复 call 时不超时；无 call 时超过 `run_timeout` 失败 |
| `cancel` | 不响应 cancel 的程序在宽限后被 kill，事件为 `Failed{Cancelled}` |
| `reuse` | 连续 3 个 run 用同一进程（`ready` 只出现一次） |

完成标准：

- 上述测试全绿；
- clippy 干净；
- `lib.rs` 导出 `pool`、`budget`、`runtime`；
- `dependency_boundaries.rs` 中 `todo_extensions` 的依赖清单同步更新。

**E5 扩展自检 `check.rs` + `examples/check_extension.rs`**

```text
cargo run -p todo_extensions --example check_extension -- extensions/builtin/chalk-hand
```

依次检查：

1. 清单合法；
2. 工作程序握手；
3. 对 `tests/<样例>/input.json` 离线运行：服务调用从 `tests/<样例>/services/<task-key>.json|<ext>` 回放，缺少录制就失败并打印缺少的 key；
4. 条目合法；
5. 两次运行条目完全一致（确定性）。

app 安装用户扩展时调用同一检查，不通过就不启用。最小验证：echo 夹具通过；改坏一行输出后报出具体问题。

### S 宿主服务桥（`todo_app/src/adapters/production/bridge.rs`）

ServiceBridge 是 adapter 代码，只做转换和 I/O；转换规则放 `todo_production`，避免同一规则写两份。

**S1 纯转换（`todo_production/src/extension/services.rs`）**

```rust
pub fn task_for(call: &ServiceCall, host: &HostDefaults) -> Option<TaskSpec>; // chat/image/speech → TaskSpec；其余 None
pub fn speech_result(text: &str, raw: &Value, pcm: Pcm, file: &str) -> Value;   // { file, duration, timeline }
pub fn subtitles_result(call: &SubtitlesCall, timeline: &Timeline, pcm: Pcm) -> Value; // { cues }
```

- `model: "default"` 或缺省时用 `HostDefaults`。
- 音色以 `S_` 开头时，资源换成 `seed-icl-2.0`（与 `narration_spec` 共用同一个函数，先抽出来）。
- `speech_result` 复用 `voice::build_timeline` + `retime` + `snap_sentence_edges`。
- `subtitles_result` 复用 `subtitles::cues` + `detect_silences`，与 `apply_voice` 走同一条路径（抽公共函数，不复制）。

最小验证：

- 黄金测试：同一份录制 TTS 结果，经 `apply_voice` 与 `speech_result` / `subtitles_result` 得到的时间轴和字幕条完全相同；
- `S_` 音色走克隆资源。

**S2 桥接执行**

| service | 执行 | 结果文件 |
| --- | --- | --- |
| chat | `AiService::submit(task_for(..))`，Done 后取 `output.text` | — |
| image | 同上，Done 后把 `artifact_path` 硬链接 / 复制到 `work/services/<key>.<ext>` | `{file,width,height}` |
| speech | 同上，读 WAV → `speech_result` | `{file,duration,timeline}` |
| subtitles | 本地：读 `speech` 文件和对应的时间轴（桥接缓存 `key → Timeline`）→ `subtitles_result` | `{cues}` |
| probe | `todo_media` metadata | `{duration,width?,height?,hasAudio}` |
| encode | `todo_media::encode`（M2） | `{file,duration,width,height}` |

- AiService 的 `Failed` 映射为工作程序错误：
  - `content_policy` → `content_policy`；
  - `invalid` → `invalid`；
  - 其余（含 `possibly_billed`）→ `unavailable`，消息带「可能已扣费」提示。
- `Waiting` / `NeedsAttention` **不回复**工作程序，让它等着；界面显示等待原因。
- 生图和配音并发受 `AiService` 限流控制，桥不另设上限。
- 取消 run 时，取消它名下尚未完成的 AiService 任务。只有没有其他 run 或成片在等这个任务时才取消（引用计数）。

最小验证：在 `todo_app` 的 adapter 测试里，用 `todo_ai` 测试中的 `FakeServer` 起 AiService，配合 echo 工作程序的 `call` 模式：

- chat 往返；
- image 文件出现在 workdir；
- 断网（FakeServer 返回 503 数次）后自动恢复，工作程序最终成功；
- 重跑同一 run，FakeServer 请求计数不增加。

### R Python 运行时与 SDK

**R1 `dependencies/python/`**

- 来源：python.org 的 Windows embeddable 包（3.13.x amd64），加上固定版本的 wheel：
  - `numpy`、`opencv-python-headless`、`pillow`、`fonttools`；
  - 版本写在 `dependencies/python.lock`，带 sha256。
- 脚本 `scripts/fetch_python.ps1`：
  1. 下载并校验；
  2. 解压；
  3. 在 `python313._pth` 打开 `import site`；
  4. 执行 `python -m pip install --no-deps --require-hashes -r dependencies/python.lock --target dependencies/python/Lib/site-packages`。
- 二进制不入库（加进 `.gitignore`）；`build.ps1` 复制到 `dist/to_do/dependencies/python`。
- 开发机可以先用 `TODO_PYTHON` 指向旧项目的 `runtime/python/python.exe`（3.14.2，已有 numpy 2.4.1、cv2 4.13、PIL 12.1）过渡，但**不得写进任何配置**。

最小验证：执行 `dependencies\python\python.exe -X utf8 -c "import numpy, cv2, PIL, fontTools; print('ok')"`，结果记入汇报。

**R2 SDK `extensions/sdk/python/todo_worker.py`**（扩展作者唯一需要的库，只用标准库）

```python
from todo_worker import Worker, ServiceError, RunFailed

worker = Worker(version="1.0.0")

@worker.op("run")
def run(ctx):                      # ctx.input / ctx.params / ctx.canvas / ctx.workdir(Path) / ctx.seed / ctx.cancelled()
    ctx.progress("配音", 0.1)
    voice = ctx.call("speech", {"text": "...", "voice": ctx.params["voice"]})   # 阻塞，可在线程池里并发
    futures = [ctx.call_async("image", {...}) for ...]                           # 返回 Future
    ctx.emit([{...item...}])
    return {"ok": True}            # → done.result

worker.serve()                     # 读 hello → 回 ready（ops 来自注册的装饰器）→ 循环
```

行为要求：

- **stdout 只写协议**：SDK 启动时把 `sys.stdout` 换成写到 stderr 的包装，协议写入保存下来的原始 fd 1。这样作者的 `print` 不会破坏协议。
- `call` 的 id 自增。reply 可乱序到达，用 `threading.Event` 分发。
- `ServiceError` 携带 `cls`（`invalid` / `content_policy` / `budget` / `unavailable`）。
- 收到 `cancel` 后 `ctx.cancelled()` 为真，`ctx.call` 抛 `RunFailed("cancelled")`。
- 未捕获异常 → `failed{class:"internal", message: 最后一行 + traceback 摘要}`。
- `MemoryError` / `OSError(ENOSPC)` → `resource`。
- 写文件辅助：`ctx.path("out/a.png")` 保证在 workdir 内，并自动创建父目录。

最小验证：

- E4 的 echo 夹具改用 SDK，测试全绿；
- 额外测「作者 print 不破坏协议」。

### M 媒体能力（core + 原生）

**M1 core 片段扩展**（`todo_core/src/model/clip.rs` + 新 `model/effects.rs`）

```rust
pub struct Clip { …, pub intro: Option<Intro>, pub audio: Option<ClipAudio>, pub origin: Option<ClipOrigin> }
pub struct Intro { pub kind: IntroKind, pub duration: f64 }
pub enum IntroKind { Fade, Rise, Pop, SlideLeft, SlideRight, Drop, Stamp, Grow }
pub struct ClipAudio { pub gain: f64 /*0..=4*/, pub fade_in: f64, pub fade_out: f64 }
pub struct ClipOrigin { pub production: String, pub key: String, pub rev: String }
pub fn intro_at(intro: &Intro, t: f64) -> IntroSample { opacity: f64, dx: f64, dy: f64, scale: f64 }
```

- 序列化：都可省略，旧工程照常打开。`todo_storage` 文档版本 +1，读旧版本补默认值，测试覆盖新旧两种。
- `resolve_preview`：每层的 transform 与 `intro_at(片段内时间)` 合成后下发。**导出调用同一函数**。
- 曲线（u = t/duration，裁到 [0,1]；e = 1−(1−u)³，即 ease-out cubic；坐标沿用 core Transform 约定：画布宽高的分数，y 轴方向以现有 `VisualTransform` 为准）：
  - `Fade`：opacity × e
  - `Rise`：dy = +0.06 × (1−e)（从下方升起），opacity × min(1, 2u)
  - `Pop`：scale × (0.6 + 0.4 × backOut(u))，opacity × min(1, 3u)；backOut 取 s = 1.70158
  - `SlideLeft` / `SlideRight`：dx = ±0.25 × (1−e)，opacity × min(1, 2u)
  - `Drop`：dy = −0.15 × (1 − bounceOut(u))，opacity × min(1, 4u)
  - `Stamp`：前 70% 时 scale × (1.3 − 0.3 × easeInCubic(u/0.7))；后 30% 时 scale × (1 − 0.03 × sin(π × (u−0.7)/0.3))；opacity × min(1, 4u)
  - `Grow`：scale × max(0.01, e)
- 音频场景：`ResolvedAudioClip` 带 gain 和 fade，下发给原生混音器（M4）。
- `todo_production::extension::items::IntroKind` 改为直接复用 core 的 `IntroKind`，不再各定义一份。规范文档 §8 列出全部取值。
- 条目新增音频字段 `fade: { in, out }`（秒，仅音频角色），在 `validate_items` 里校验。

规范 v1 尚未发布，以上属于「增量补充」，`extension-spec.md` 同步写清楚。

最小验证：

- core 单元测试：每种曲线在 u=0 / 0.5 / 1 处的值，u≥1 时恒等；
- 旧工程 JSON 往返；
- `resolve_preview` 在片段开头 0.1 s 时 opacity < 1。

**M2 packed-alpha 视频**

- 格式（写进 `native-media.md`）：
  - H.264 High，yuv420p，BT.709 limited；
  - 宽 = W，高 = 2H；上半部是颜色（直通 alpha，透明区已做颜色外扩），下半部是 alpha，存在亮度里；
  - 视频流 metadata `todo_alpha=stacked`；
  - `-g` 取 fps 值（1 s GOP）以便拖动，`-crf 16`，`-preset slow`。
- `todo_media/src/encode/`：
  ```rust
  pub fn encode_frames(frames_dir, fps, alpha: bool, quality, audio: Option<&Path>, out: &Path, cancel) -> Result<EncodeInfo, MediaError>;
  ```
  - Rust 读 PNG 序列（`image` crate），做颜色外扩：透明像素的 RGB 填入最近的不透明像素颜色，迭代 8 次的 3×3 膨胀即可；拼成上下两半，写 rawvideo 到 FFmpeg stdin（`todo_process::supervise`）。
  - 不透明：`-c:v libx264 -pix_fmt yuv420p -crf 18 -g <fps>`；
  - 带音频：AAC 192k。
  - 编码后用 ffprobe 自校验尺寸、帧数、metadata，不符就删掉文件并报错。
- 探测：metadata 中有 `todo_alpha=stacked` → `MediaItem.alpha = Some(AlphaPacking::Stacked)`，显示尺寸为 W×H。
- 原生：解码后的纹理在着色器里采样上下两半，输出预乘 alpha。layer 协议新增 `kind` 字段（`video | image | packed | reveal | text`），由 core 根据 `MediaItem` 决定，**原生不靠扩展名猜**。

最小验证：

- Rust：用 3 帧半透明 PNG 编码后 ffprobe 校验；
- 原生：`native/tests/test_packed_alpha.py` 合成一层 packed 叠在纯色上，抽检像素，alpha 误差 ≤ 3/255，边缘无暗边：边缘像素亮度 ≥ 95% 理论值。

**M3 reveal 图层 `todo-reveal/1`**

- 文件：`<name>.reveal.png`，8 位 RGBA PNG，宽 2W、高 H。
  - 左半部：最终画面，straight alpha。
  - 右半部：逐像素出现时间。R、G 组成 16 位 t（0..65535 ↦ 0..duration）；B 为柔化宽度 s（0..255 ↦ 0..`softMax` 秒）；A 为 255 表示该像素参与 reveal。
  - `tEXt` 关键字 `todo-reveal`，内容为 JSON：
    ```jsonc
    { "spec": "todo-reveal/1", "width": W, "height": H, "duration": 3.2, "softMax": 0.6,
      "hand": { "file": "hand_chalk.png", "anchor": [0.18, 0.05], "height": 0.42,
                "path": [[0.00, 0.31, 0.22], [0.033, 0.33, 0.24], …] } }   // [t秒, x, y]，x/y 为元素框内 0..1；hand 可省略
    ```
- 求值：片段内时间 τ 时，像素可见度 = `smoothstep(t − s, t, τ)`，输出 alpha = 左半 alpha × 可见度。τ ≥ duration 时为最终画面，片段可以比 duration 长，之后保持最终画面。
- 粉笔手：
  - path 线性插值得到笔尖位置；
  - 手图按 anchor 对齐笔尖，高度 = 元素框高 × `height`；
  - 在 [path 首点, path 末点 + 0.15 s] 内显示，最后 0.15 s 淡出；
  - 手图文件在 reveal 同目录。
- 原生：
  - 首次使用时用 WIC 读 PNG，解析 tEXt，上传两张纹理（画面与时间图）和手图；按路径缓存；
  - **之后每帧只更新常量缓冲区，零解码**；
  - 着色器放 `native/src/transform/` 旁的 `reveal_shader.h`。
- 探测：`todo_media` 读 PNG 的 tEXt（需要 `png` crate，加入 workspace 依赖），得到 `MediaKind::Reveal`、时长、尺寸。
- 预览预算：reveal 不计入「解码视频 ≤ 6」，计入总 16 层。

最小验证：

- Rust：编码 / 解析往返（`todo_production` 或 `todo_media` 提供 `RevealMeta` 读写，**只有一份实现**，烘焙器的 Python 写法以它的测试夹具为准）；
- 原生：`native/tests/test_reveal.py` 用 64×64 夹具，t 随 x 线性增长，在 τ=0.5·duration 处左半可见、右半不可见，误差 ≤ 2/255；拖动 100 次 seek 时解码计数为 0。

**M4 原生混音增益与淡变**

- 协议 `AudioClip` 增加 `fade_in`、`fade_out`；`audio.cpp` 中对每个片段乘以 gain × 线性淡变包络。
- 最小验证：`native/tests` 用 1 kHz 正弦波夹具，淡入中点的 RMS 约为 50%（±5%）。

**M5 文字层下发**：字幕条与花字已由现有文字栅格支持，无需改动；确认 `clean-white-outline` 样式用于字幕，位置为底部安全区（y 偏移 +0.38）。

### P 成片存储与用例

**P1 `todo_storage/src/productions/`**

```rust
pub struct ProductionStore { … }
impl ProductionStore {
    pub fn list(&self, project: &str) -> Result<Vec<ProductionSummary>, StorageError>;
    pub fn load(&self, project: &str, id: &str) -> Result<Production, StorageError>;  // 校验 spec 版本
    pub fn save(&self, project: &str, doc: &Production) -> Result<(), StorageError>;    // 原子写，rev 单调
    pub fn workdir(&self, project: &str, id: &str) -> Result<PathBuf, StorageError>;    // 创建并校验在工程内
    pub fn delete(&self, project: &str, id: &str) -> Result<(), StorageError>;          // 移入 .trash
}
```

- 路径限制与原子写复用 `todo_storage/src/filesystem/`。
- 最小验证：往返；损坏 JSON 报可读错误；workdir 越界（id 含 `..`）被拒；并发保存时后写赢且不出现半个文件。

**P2 口播条目生成 `todo_production/src/narration.rs`**

```rust
pub struct NarrationLayout { pub segment_gap: f64 /* 默认 0.4 s */, pub tail: f64 }
pub struct BakeRequest { pub board: String, pub input: Value }   // 发给工作程序 bake_board 的 input
pub fn segment_starts(doc: &Production, layout: &NarrationLayout) -> Vec<f64>;  // 各段配音的起点（相对 t0）
pub fn bake_requests(doc: &Production, spec: &ProductionSpec, pacing: &Pacing, canvas: &Canvas) -> Vec<BakeRequest>;
pub fn host_items(doc: &Production, spec: &ProductionSpec, layout: &NarrationLayout) -> Vec<Item>; // 配音、字幕；无 bake 时元素静帧 + intro
```

- 每块黑板的 `input` 包含：
  - 元素：id、role、enter、图片在 workdir 内的路径、label；
  - 版式 `layout`、`tone`、`stroke`、`fill`、`hand`、`handSize`、`drawSpeed`；
  - 排期：`pacing::schedule` 的 `TimedBoard`，换算成相对成片起点的秒；
  - 稳定 key 前缀 `s<段>/b<板>`；
  - 输入指纹。黑板的 `Rendered.fingerprint` 等于它时不重烘。
- key 约定：
  - `s<i>/voice`
  - `s<i>/sub/<n>`
  - `s<i>/b<k>/plate`（黑板底，若 `plate=board`）
  - `s<i>/b<k>/e<m>/draw`
- rev：内容指纹的前 16 位。
- 配音条目 `link` 为空；同段的画面和字幕 `link: "s<i>/voice"`，配音被用户挪动时它们跟着走。
- 最小验证：
  - 用 `tests/fixtures` 里已有的录制配音构造两段文档，断言段间留白 0.4 s、key 稳定；
  - 改第二段文案后第一段的所有 rev 不变；
  - 字幕条时间落在本段配音区间内。

**P3 入轨对账 `todo_production/src/placement.rs`**

```rust
pub struct PlacementPlan { pub adopt: Vec<PathBuf>, pub edits: Vec<PlacementEdit>, pub tombstones: Vec<(String,String)> }
pub enum PlacementEdit {
    Place { key: String, rev: String, role: ItemRole, lane: u32, start: f64, duration: f64, source: PlacedSource,
            transform: Option<ItemTransform>, intro: Option<Intro>, audio: Option<ClipAudio> },
    Replace { clip_id: String, rev: String, source: PlacedSource, duration: f64, keep_transform: bool },
    Shift { clip_ids: Vec<String>, delta: f64 },
    Remove { clip_id: String },              // 仅用于「上游已不再产出」的条目，且该片段未被用户改动过
}
pub fn reconcile(project: &Project, production: &str, placement: &Placement, items: &[Item], t0: f64) -> PlacementPlan;
```

规则，与规范 §8 一致：

1. 工程中按 `ClipOrigin{production, key}` 找片段。
2. key 不存在、也不在墓碑里 → `Place`。
3. key 存在且 rev 相同 → 不动。
4. key 存在、rev 不同 → `Replace`：**保留用户改过的 transform / 时间位置**，时长按新条目更新，左端不动。
5. `placement.placed` 里有、工程里找不到 → 视为用户删除，写墓碑 `(key, rev)`，以后同 rev 不再放回；rev 变了（用户点了重画）才重新放。
6. 带 `link` 的条目：被链接片段的实际起点与条目给出的起点之差，平移到链接方上。
7. 轨道：
   - 按 role 映射：voice / music → Audio，sfx → Sfx，visual → Video，text / subtitle → Text；
   - 同一成片的 `(role, lane)` 映射到固定轨道，第一次时用 `track_space` 分配，并记进 `Placement`；
   - 冲突时沿用 core 的溢出策略。

整批编辑在 application 里作为**一个事务**提交：一次对账 = 一条撤销记录，名称「AI 入轨」。用户撤销它会删除这些片段，下一次对账按规则 5 写墓碑，因此撤销不会被自动重放。

最小验证（纯函数测试，构造 `Project`）：

- 首次放置；
- 同 rev 幂等，第二次调用的计划为空；
- 新 rev 保留用户 transform；
- 用户删除后不复活；
- 重画后复活；
- 链接平移；
- lane 分轨稳定。

**P4 application 用例 `todo_application/src/productions/`**

```rust
pub struct ProductionSession { doc: Production, spec: …, in_flight: BTreeMap<TaskKey, Target>, runs: …, status: … }
pub enum ProductionTask {                     // Effect::Production(ProductionTask)
    Submit(TaskSpec), Cancel(TaskKey), Retry(TaskKey),
    Run { run: LocalRunId, extension: String, op: String, input: Value, params: Value },
    Reply { run: LocalRunId, call: String, result: Result<Value, ServiceError> }, // 仅测试用；正式由 bridge 处理
    CancelRun(LocalRunId), Save(Production), Adopt(Vec<PathBuf>),
}
pub enum ProductionOutput {                   // AppEvent::Production(ProductionOutput)
    TaskProgress { key, state }, TaskDone { key, output, file }, TaskFailed { key, class, message, possibly_billed },
    Speech { key, file, raw, pcm_samples, rate },   // adapter 读好 WAV 后回传
    RunProgress { run, phase, fraction }, RunItems { run, items }, RunDone { run, result }, RunFailed { run, error },
    Attention { provider, attention }, Saved { rev }, Adopted { media: Vec<MediaItem> },
}
```

- `tick()`：
  - `plan_work` 产出的 Work 中，不在途的提交（每个成片最多 64 个在途任务）；
  - 回执经 `apply_*` 写回文档，保存（节流：500 ms 合并一次）；
  - 板子的图齐了，且指纹与 `Rendered` 不同 → 发 bake run；
  - 收到条目 → `reconcile` → 先 `Adopt` 新文件（媒体登记，见 P5），完成后用核心编辑命令提交事务。
- `ControlEdit` 不需要新增：对账走 core 编辑命令，并给片段写 `origin`。另外在 control 里新增只读的 `get_production` 与 `apply_production_edits`，复用 `production::Edit`，供外部 AI 和脚本使用。
- 用户编辑（`production::Edit` 整批）→ `apply_edits` → `refresh` → `tick`。
- workflow 扩展：开始 = 一个 `Run{op:"run"}`；用户改参数或文案 → 取消当前 run 并重跑；条目处理同上。成片文档为 workflow 增加 `workflow: Option<WorkflowState { last_input_fingerprint, items: Vec<Item>, result }>`（`todo-production/2` 增量字段）。
- 切换工程时取消本工程所有 run，AiService 任务不取消：它们下次打开工程时直接命中缓存。

最小验证（application 单元测试，不起线程）：

- 喂入录制的 outline / voice / storyboard / image 回执，断言 Effect 序列：配音条目先于黑板条目入轨；
- 改一个元素描述只产生 1 个 image Submit 和 1 个 bake Run；
- `TaskFailed{possibly_billed}` 不自动重试，状态为「需要处理」；
- 切换工程后的旧回执被忽略（沿用 ticket 机制）。

**P5 app 装配 `todo_app/src/adapters/production/`**

- `ProductionHost::start(root)` 依次：
  1. `AiConfig::load(root)`；
  2. `Secrets::resolve`；
  3. `AiService::start(ServiceSettings{…})`，返回值中的 `Recovered` 交给 application；
  4. `Registry::scan(root/extensions/builtin, root/extensions/user)`；
  5. `WorkerPool::start`。
- 媒体登记 `Adopt`：新增 `MediaTask::Adopt { files }`。文件已在工程目录内时**不复制**，只 probe 并登记为 MediaItem；路径为相对 repo 根的 `data/...`；同路径同大小同修改时间则复用已有 MediaItem。
- 事件循环：每帧 `poll` AiEvent / WorkerEvent → 转 `AppEvent::Production`；`WorkerEvent::Call` 不进 application，直接交 bridge。
- 关闭窗口：
  1. 保存所有成片；
  2. `WorkerPool::shutdown`；
  3. drop `AiService`。在途日志保持 `Submitting` / `Downloading`，下次启动时恢复。
- 最小验证：
  - adapter 测试用 FakeServer + echo 工作程序，走一遍「开始 → 条目入轨 → 关闭 → 重开 → 不再请求」；
  - 断言 FakeServer 第二次启动请求数为 0。

### U 界面

**U1 AI 设置**（`todo_ui/src/widgets/ai_settings.rs`；展示模型在 `todo_interaction/src/presentation/ai_settings.rs`）

字段：

- 虎妞：服务地址、API Key、对话模型、生图模型（下拉框可手填）；
- 火山：服务地址、API Key、默认音色；
- 网络：代理（直连 / 系统 / 自定义）。

行为：

- key 输入框为密码模式，显示掩码与来源；环境变量覆盖时只读，并提示「由环境变量提供」。
- 「保存」时：
  1. `AiConfig::save`（原子、校验，URL 由 `normalize_base_url` 规范化）；
  2. `Secrets::save`；
  3. 只改了 key → `AiService::set_key`；改了地址或代理 → 重启 AiService（任务日志会恢复在途任务）。
- 「测试连接」（免费）：
  - 虎妞发 `GET {base}/v1/models`：200 → 「可用」，401/403 → 「key 无效」，网络失败 → 原因。**先用 `ai_probe` 风格的一次性示例确认该端点免费且存在，再接入界面**；不存在就改为只校验格式，并说明「首次生成时验证」。
  - 火山没有免费端点，只做格式校验；首次请求遇到 401 会触发 `NeedsAttention`，界面弹回设置页。
- 清除 key：`Secrets::save(provider, None)`。

最小验证：

- interaction 单元测试：地址规范化、空 key 校验、环境变量来源时只读；
- UI 交互测试沿用 `todo_ui` 现有模式；
- 用 `MemoryStore` 跑「保存 → 读取」，掩码不泄露原文（断言 Debug / Display 输出不含 key）。

**U2 成片面板**（`todo_ui/src/widgets/production/`）

1. **扩展列表**：卡片显示名称、描述、预览图、就绪状态（缺 key / 运行时缺失 / 自检失败 / 就绪）。用户扩展可以「安装 .todoext」或卸载；安装走 E5 自检。
2. **新建**：
   - 参数表单由清单自动生成：按 `group` 分节，`quick` 参数置顶，其余收进「更多」，`when` 控制显示；
   - `voice` 类型参数用音色选择器（内置几个常用音色 + 手填 `S_…`）；
   - 文案框显示字数与上限；workflow 扩展若声明了 `input.media`，显示素材选择（从料箱挑）；
   - 入轨位置：播放头 / 接在末尾，默认播放头；
   - 「生成」按钮。
3. **进度**：
   - 树形结构，成片 → 段 → 黑板 → 元素（workflow 扩展只显示 run 的 phase 和进度）；
   - 每项状态：排队 / 生成中 / 下载中 / 等待网络（附原因与下次重试倒计时）/ 需要处理 / 完成 / 失败；
   - 顶部汇总：已完成 x / y、预计费用（按任务数估算，可选）。
4. **编辑**：
   - 段：改文案、拆、合、删、重新分镜；
   - 元素：改描述（label / draw）、候选图网格（挑选、再来一张、删除、上传自己的图）；
   - 每个操作都映射成 `production::Edit`。
5. **需要处理**：
   - 401/402 横幅 → 打开 AI 设置；
   - `possibly_billed` 项显示「可能已扣费，确认后重试」按钮；
   - 内容审核拒绝显示原因，并提供「改写描述」入口。
6. **时间轴联动**：选中 AI 片段时，属性栏显示来源（扩展 · key）以及「在成片面板中定位」「重画」；入场与音频参数可编辑（M1 字段）。

最小验证：

- interaction 层 ViewModel 测试：录制文档 → 树结构与状态文案；
- `when` 显示逻辑；
- 参数越界提示（复用 `Manifest::resolve_params` 的错误）。

### C 黑板手绘烘焙器（`extensions/builtin/chalk-hand/worker/bake.py`）

**来源**：

- 旧 `D:\Desktop\TO-DO-App\todo-editor\backend\skills\chalk-hand\render.py`：
  - `render_scene_clips`、`render_plate`、`place_scene`、`layout_rects`；
  - `element_crop`、`tip_path`、`iter_entrance_frames`、`_make_dust`；
- 旧 `dependencies/vendor/blackboard/scripts/`：
  - `stream_render.py`、`soft_cutout.py`、`element_locator.py`、`blackbg_element_hand.py`；
- 素材：`assets/chalkboard-bg-black.png`、`hand-chalk.png`。

复制到包内 `worker/blackboard/` 与 `assets/`，在文件头注明来源；**删除**对 `av`、`subtitle_pipeline`、`blackbg_compose`、`BLACKBOARD_ROOT` 环境变量和外部路径的依赖。

**操作**：

- `plate`：输出 `out/plate.png`（按画布尺寸裁切或缩放黑板底），emit 一条 `visual` lane 0 的条目，覆盖成片全长。`plate=none` 时不调用。
- `bake_board`：input 见 P2。对每个元素：
  1. `soft_cutout` 抠透明，按 `tone` 转粉笔色（`to_straight_rgba`）；
  2. `place_scene` 排版，得到元素框 → 条目的 transform（x / y / scale，相对画布）；
  3. 按 enter 分三种：
     - `draw`：用旧 `RegionStreamRenderer` 按蒙版逐区骨架书写，**不写视频帧**，而是记录每个像素首次达到最终 alpha 50% 的帧号，得到 t 图。`fill=wipe` 的上色像素 t 取写完时刻，s 取 0.4 s；线稿 s 取 2 帧。笔尖轨迹 → `hand.path`（`hand=none` 时省略）。写出 `out/<key>.reveal.png`。
     - `fade`：输出静帧 PNG，条目 `intro: fade 0.4 s`。
     - `stamp`：输出静帧 PNG，条目 `intro: stamp 0.35 s`。粉笔灰用 `_make_dust` 逐帧生成 PNG 序列，经 `encode{alpha:true}` 得到 packed-alpha mp4，作为同 key 加 `/dust` 的附加条目，`link` 到元素。
  4. **reveal 自检**：
     - 在 8 个均匀时刻，用 t 图合成画面，与 `RegionStreamRenderer` 实际帧的 alpha 比较，平均绝对误差 ≤ 2/255、最大 ≤ 24/255；
     - 不通过（例如渲染器有擦除或回退等非单调效果）→ 回退为逐帧 PNG + `encode{alpha:true}` 的 packed-alpha 视频；
     - 回退次数写进 `done.result.fallbacks`。
- 时间：条目 start 等于 `TimedElement.at_ms`，duration 到黑板结束（`TimedBoard.end_ms`）。reveal 的 `duration` = `window_end_ms − at_ms` 按 `drawSpeed` 缩放，且不超过窗口。
- 确定性：所有随机数由 `ctx.seed` 加元素 key 的哈希派生；不得读取时间或系统随机。

**测试**（`extensions/builtin/chalk-hand/tests/`）：

- `sample/input.json`：一块黑板，3 个元素，分别用 draw / fade / stamp；
- `sample/services/`：录制的 3 张元素图（从旧项目已生成的元素图里挑，不花钱）；
- `expect.json`：期望的 key 与 role。

`check_extension` 通过，两次运行逐字节一致。

最小验证（Rust 侧）：

- `todo_extensions` 集成测试跑 `bake_board` 样例；
- 用 M3 的 `RevealMeta` 解析输出，`duration` 与窗口一致；
- 原生测试载入输出的 reveal 文件，在 τ=0 时全透明，τ=duration 时等于最终画面。

**质量对照**：

1. 取旧项目任一成片的 3 块黑板，用旧 `render.py` 出 WebM，新烘焙器出 reveal；
2. 在相同时刻抽帧比较，同一时刻覆盖率误差 ≤ 5%；
3. 截图放进 `docs/reports/`。

### K 海绵宝宝口播迁移（`extensions/builtin/spongebob-koubo/`，`kind: workflow`）

**来源**：

- `D:\Desktop\TO-DO-App\spongebob-koubo.todoext`，旧格式 `todo-skill-pack/1`，可解压查看；
- 管线：`clean → tts → design → assets → render → mix_bgm → accept`；
- 详见包内 `README.md`、`EDIT_WORKFLOW.md`、`ACCEPTANCE.md`、`MIGRATION.md`。

**新清单要点**：

- `id: studio.spongebob-koubo`，`version: 2.0.0`，`kind: workflow`；
- `input.script`（≤ 12000）；
- `requires.services: [chat, image, speech, subtitles, encode]`，`runtime: python`；
- `worker: { entry: "worker/main.py", ops: ["run"], timeoutSec: 1800, maxProcesses: 1, memoryMiB: 3000 }`；
- `budget: { image: 80, chat: 60, speech: 60 }`。

参数保留以下几项，删掉宿主不再需要的 `quality` 和 `fps`（由工程决定）：

- `voice`：`voice` 类型，默认 `zh_female_mizai_uranus_bigtts`。旧默认是作者账号的克隆音 `S_RksT6Z6g2`，**不能**作为分发默认值；
- `split`、`layout`、`tone`、`enter`、`drawSpeed`。

**目录**：

```text
worker/main.py          SDK 入口：run(ctx) 编排下列阶段并渐进 emit
worker/koubo/           旧 workflow/ 下的 Python 模块（cleanup、bgm_policy、richness、02_镜头设计/*、03_素材生成/gen_from_brief、kit/*）
                        —— 改为包内相对导入，去掉 paths.py 里的绝对路径和环境变量
assets/fonts/           随包字体（见 §6 授权）
assets/fx/bubble_wipe_frames/  泡泡转场 RGBA 帧（打包期从 bubble_wipe.mp4 预先抽取，运行时不再调用 FFmpeg）
assets/fx/bubble_wipe_sfx.wav  预先按 2 s / volume 5.0 处理好的转场音效（打包期一次性生成）
assets/bgm/{intro,mid,outro}.mp3
prompts/                从 llm_briefs.py 抽出的提示词（便于审阅与改写）
tests/sample/           224 字样稿 + 录制服务结果 + expect.json
```

**服务替换**（全部删掉 `kit/llm.py` 中 `MISHI_ROOT` / `TTS_TIMELINE_ROOT` 的子进程调用）：

| 旧 | 新 |
| --- | --- |
| `kit/llm.py` 的 chat 子进程（600 s 超时 × 3 层重试） | `ctx.call("chat", {messages, json:true})`。**删掉包内所有重试和超时**，由宿主负责（规范 §5） |
| `03_素材生成/generate_images.py` 自建批调度和 wait 轮询 | 并发 `ctx.call_async("image", {prompt, size, transparent:true, variant})`；排队、限速、长尾都由宿主处理 |
| `01_口播试听/run_tts.py` 整稿一次 TTS | 按段落调用 `ctx.call("speech", …)`（每次 ≤ 1200 字），拼接时间轴偏移；字幕用 `ctx.call("subtitles", …)` |
| `layers_export` 用 FFmpeg 编 VP9 alpha WebM | 静帧 PNG + 宿主 `intro`（见下表）；复杂入场、开场碎裂、泡泡转场 → 逐帧 PNG + `ctx.call("encode", {alpha:true})` |
| `mix_bgm.py` 用 FFmpeg 混 BGM | 三条 `music` 条目：`source.in` = trim_start，`gain` ≈ 0.3，`fade{in,out}` 按 `bgm_plan`。混音由宿主导出负责 |
| `accept_gates/g1–g8` | 保留 g1（镜头多样性）、g3（BGM 规定曲目）、g5（对话顺序，无对话线索记 N/A）、g8（版式与文字）作为工作程序内检查，结果写 `done.result.gates`，宿主在面板显示。g2/g4/g6/g7 的计时与关机检查删除，由宿主统一度量 |

**入场映射**（`stage.py` 的 `fx_*` → 宿主 IntroKind；没有对应项的逐帧编码）：

| 旧 fx | 新 |
| --- | --- |
| `fade` / `dim_up` | `Fade` |
| `rise` / `float_in` | `Rise` |
| `pop` / `punch` | `Pop` |
| `slide_left` / `slide_right` / `roll_*` | `SlideLeft` / `SlideRight` |
| `drop` / `hop` | `Drop` |
| `stamp` | `Stamp` |
| `grow` / `unfold` | `Grow` |
| `cut` | 无 intro |
| `spin` / `swing` / `toss` / `wipe` / 开场红字碎裂 / 泡泡转场 | 逐帧 PNG → packed-alpha |

旧的待机动画 `idle`（漂浮、呼吸缩放）v2 不做，元素静止。之后如有需要，再加宿主的「循环动效」，单独立项。

**渐进入轨顺序**（与旧 `EDIT_WORKFLOW.md` 一致）：

1. 配音完成后立刻 emit 全部 `voice` 条目；
2. 设计完成后按镜头顺序：每镜素材齐了就 emit 该镜所有 visual（底板 lane 0、角色、道具、气泡、花字）；
3. 泡泡转场画面与其 `sfx` 条目同时 emit；
4. 画面全部完成后 emit 三条 BGM 和字幕（`intro_red` 句不出字幕）。

key 约定：

- `voice/<段>`
- `shot/<n>/<元素>`
- `shot/<n>/wipe`、`shot/<n>/wipe/sfx`
- `bgm/intro|mid|outro`
- `sub/<n>`

**最小验证**：

- 离线样例 `tests/sample`：录制服务结果，可从旧管线的历史产物整理，按新 TaskKey 命名，用 `check_extension` 跑出 key 列表；检查 g1/g3/g5/g8 全绿，两次运行一致；
- 抽 5 个时刻，与旧管线 `成片.mp4` 截图对比，人工确认视觉语言一致，截图放进 `docs/reports/`。

### X 导出

**X1 离线合成**：在 `preview_host.exe` 增加 `--render` 模式，复用同一套解码器、着色器和图层合成。

- stdin 收 JSON 行：
  - `begin{width,height,fps,frames}`；
  - 每帧 `frame{index, layers:[…与预览相同的 layer 结构…]}`；
  - `end`。
- 输出：stdout 写每帧 BGRA rawvideo。
- 由 Rust 在两个进程之间做零拷贝管道：`Command::stdout(Stdio::from(ffmpeg_stdin))`。
- 状态与错误写 stderr 行。
- 解码一律 Exact（不允许 Partial / approx），顺序前进。
- 文字层用文字栅格 PNG，按输出分辨率渲染。

**X2 图层序列**：`todo_core::query` 新增 `export_layers(project, t) -> Vec<PreviewLayer>`，与 `resolve_preview` 共用实现，只是去掉「拖动预算」限制，仍保留 16 层上限，超出时导出前报错列出位置。`todo_media/src/export/` 逐帧调用并写给原生。

**X3 音频**：`todo_media/src/export/audio.rs` 由 core 音频场景生成 FFmpeg 滤镜图。

- 每片段：`atrim` → `asetpts` → `atempo`（变速）→ `volume`（gain）→ `afade`（in/out）→ `adelay`；
- 合并：`amix=normalize=0`；
- 响度两遍处理：第一遍 `loudnorm=I=-16:TP=-1.5:LRA=11:print_format=json` 测量，第二遍用 `measured_*` 线性归一；
- 输出 48 kHz 立体声，AAC 192k。

**X4 预设**（`todo_media/src/export/presets.rs`）：

| 预设 | 视频 | 音频 |
| --- | --- | --- |
| 标准 1080p30 | libx264 `-preset slow -crf 18 -pix_fmt yuv420p -colorspace bt709 -color_primaries bt709 -color_trc bt709 -movflags +faststart`，GOP 2 s | AAC 192k |
| 高质量 | `-crf 14 -preset slower` | AAC 256k |
| 竖屏 1080×1920 | 同标准；要求工程画布为竖屏，不自动裁切 | 同标准 |
| 快速草稿 | `-preset veryfast -crf 23` | AAC 128k |

**X5 质检**（导出完成后自动执行，报告写进 `<输出>.report.json`，界面显示）：

- ffprobe：编码、像素格式、色彩标记、帧率；时长与时间轴误差 ≤ 1 帧；音视频时长差 ≤ 1 帧。
- 响度：`ebur128` 积分响度 −16 ±1 LUFS，真峰值 ≤ −1.0 dBTP。时间轴无音频时跳过。
- 画质：抽 10 帧，把合成器原始帧与解码后的导出帧比较，PSNR ≥ 40 dB（标准预设），用来防止编码参数退化。
- 黑场 / 冻帧：`blackdetect` 与时间轴空白区对照，非空白区出现黑场 > 0.5 s 则报警。

**X6 界面**：导出对话框包括预设、路径、进度（帧 / 速度 / 剩余）、取消；完成后显示质检报告。

最小验证：

- 3 层夹具工程（视频 + reveal + 字幕 + 两条音频）导出，X5 全绿；
- 导出第 0、15、29 帧与预览截图（`native/tests` 已有截图能力）PSNR ≥ 45 dB；
- 取消后 1 s 内两个子进程都退出，并删除半成品文件。

### Q 打包、首次运行与端到端

**Q1 打包**：`build.ps1` 增加复制：

- `extensions/builtin/**`，排除 `tests/`；
- `extensions/sdk/**`；
- `dependencies/python/**`；
- `config/ai.example.json`。

**不复制** `config/ai.json` 和任何 `.env`。另加检查：打包产物中不得出现 `密匙`、`MISHI`、`api_key=`、`sk-` 等字样，用脚本 grep，命中就失败。

**Q2 首次运行**：

- 没有 `config/ai.json` 时用内置默认值，不报错；
- 没有 key 时成片面板引导去设置；
- 扩展就绪检查（运行时与自检）在后台执行，不阻塞启动。

**Q3 端到端**：

- 离线：录制样例跑通两个扩展、导出、质检，作为 `cargo test -p todo_app --test e2e_productions`。找不到 Python 时跳过。
- 线上：**须用户同意**，花费约几元。黑板手绘 300 字、海绵宝宝 224 字各真跑一次，记录：
  - 各阶段耗时；
  - 请求数；
  - 重试次数；
  - 质检报告。

  然后在中途拔网线 2 分钟，验证自动恢复，结果写进 `docs/reports/`。

**Q4 文档**：

- 更新 `responsibilities.md`（新增 crate 和目录）、`extension-spec.md`（M1–M3 的增量）、`ai-services.md`（设置面板）、`native-media.md`（packed / reveal / --render）；
- `README.md` 增加「AI 成片」使用说明。

---

## 5. 质量门禁汇总

只有每个阶段都满足下表，才算完成；不允许放宽门槛来过关，有争议时报告给用户。

| 门禁 | 阈值 | 由谁检查 |
| --- | --- | --- |
| 工作区测试 | `cargo test --workspace` 全绿；clippy `-D warnings` 干净 | 每步 |
| 原生测试 | `python -m unittest discover -s native/tests` 全绿 | M、X |
| 扩展自检 | 两个内置扩展 `check_extension` 通过，确定性一致 | C、K、Q |
| 呼吸感 | 写完后停留 p10 ≥ 1.0 s；换板 → 第一笔中位数 ≥ 0.5 s；p90 提前书写 ≤ 0.9 s | P2 后用 `pacing_report` |
| 字幕对齐 | 字幕起止与逐字时间差 ≤ 1 帧 | P2、K |
| reveal 精度 | 自检 MAE ≤ 2/255；原生拖动零解码 | C、M3 |
| packed-alpha | alpha 误差 ≤ 3/255，无暗边 | M2 |
| 导出 | 时长 ±1 帧、PSNR ≥ 40 dB、−16 ±1 LUFS、TP ≤ −1.0 dBTP | X5 |
| 自愈 | FakeServer 503 / 断连 / 401 换 key / 重启续跑场景全过；重跑请求数为 0 | S、P5 |
| 安全 | 打包产物与日志中搜不到 key；工作程序环境中没有 key | E1、Q1 |

---

## 6. 需要用户拍板的事项

执行到相应步骤前先问，不要自行决定。

1. **字体授权**：已改为随包分发的 SIL OFL 字体（思源宋 Heavy、Noto Sans SC），不再把字魂或 AmezumiGothic 放进安装包。
2. **BGM 授权**：三段规定曲目（Holding Court、精卫（降调）、TRA$H - Memories）的随包授权仍待用户确认；获得确认后扩展将把它们作为独立音乐条目分轨，并在导出时保留淡入淡出。
3. **默认克隆音**：旧包默认 `S_RksT6Z6g2` 属于作者的火山账号，别人用会失败。新版默认用咪仔；作者本机可以在参数里改回。
4. **「测试连接」**：虎妞的 `GET /v1/models` 是否免费、是否存在，需要一次免费探测确认（U1）。
5. **线上端到端**（Q3）的花费和时间。

---

## 7. 附录

### 7.1 已有可复用的入口

- 编排：`todo_production::work::{plan_work, refresh, apply_outline, apply_voice, apply_storyboard, apply_image}`；`pacing::schedule`；`extension::narration_spec`。
- AI：`todo_ai::{AiConfig, Secrets, AiService, default_paths}`；示例 `ai_check`（免费）。
- 扩展：`todo_extensions::{Registry, load, install, uninstall, pack}`；`todo_production::extension::{manifest::Manifest, protocol::{HostMessage, WorkerMessage, ServiceCall}, items::{Item, validate_items}}`。
- 子进程：`todo_process::{supervise, capture, Duplex, Output, hide_window}`。

### 7.2 常用命令

```powershell
cargo run -p todo_ai --example ai_check                      # 免费：配置与密钥来源自检
cargo run -p todo_ai --example ai_probe -- chat              # 付费：一次只跑一项，先告知用户
cargo run -p todo_production --example pacing_report         # 呼吸感指标
cargo run -p todo_extensions --example check_extension -- extensions/builtin/chalk-hand   # E5 之后
```

### 7.3 旧项目参考位置（只读）

| 内容 | 位置 |
| --- | --- |
| 黑板手绘渲染端 | `D:\Desktop\TO-DO-App\todo-editor\backend\skills\chalk-hand\render.py`、`PROTOCOL.md` |
| 黑板渲染内核 | `D:\Desktop\TO-DO-App\todo-editor\dependencies\vendor\blackboard\` |
| 旧 Python 运行时（开发期过渡） | `D:\Desktop\TO-DO-App\todo-editor\runtime\python\python.exe` |
| 海绵宝宝包 | `D:\Desktop\TO-DO-App\spongebob-koubo.todoext`（zip） |
