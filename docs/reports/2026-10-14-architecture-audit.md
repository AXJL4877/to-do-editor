# 项目架构与剪辑能力审查

审查日期：2026-10-14。范围：当前工作区中的 Rust workspace、Node 后端、TypeScript core、Windows 原生预览以及启动与测试配置。本轮只新增审查报告；复现程序集中在被 Git 忽略的 `target/architecture-audit/`。

## 1. 结论

现有项目已经有分层基础，但职责边界还没有形成统一规则。最需要处理的是应用用例分散：`EditorSession` 放在 interaction 中，工程管理和导入任务状态放在 app 中，部分放置策略放在 UI 中。

建议建立独立的 `todo_application` 应用层，收拢工程生命周期、编辑事务、历史、导入任务和播放协调；让 `todo_interaction` 专注输入、手势、选择和界面投影；让 `todo_app` 专注窗口事件循环、依赖组装与外部副作用执行。

以下三点需要区别对待：

1. **没有发现 Rust core 引入 egui 绘制代码。** `todo_core` 的外部依赖只有 serde / serde_json，未发现颜色、控件、字体和像素布局进入其生产代码。把传入片段 ID 集合称为 selected，不等于保存了 UI 选择状态。
2. **core 确实混入了面向用户的中文错误文案。** 这些应改为类型化的领域错误，由 UI 决定显示语言。用户自己填写的轨道名称则属于工程数据，可以保存在 core。
3. **app 中保留窗口句柄、路径解析、事件轮询和调用 HTTP / 原生进程是合理的。** 应迁出的部分是任务归属、状态转移、错误恢复和工程生命周期策略。

文件结构是否合理，最终要由依赖方向、接口和测试保证。仅调整目录名称不能解决这些问题。

## 2. 当前实际依赖

通过 Cargo metadata 核对，当前内部依赖为：

```text
todo_core          → 无其他项目 crate
todo_interaction   → todo_core
todo_ui            → todo_interaction
todo_media         → todo_core
todo_preview       → 无其他项目 crate
todo_app           → core + interaction + ui + media + preview
```

当前没有 crate 循环依赖。问题集中在每个 crate 内承担的职责，以及通过重导出暴露出去的接口。

## 3. 按优先级排列的问题

### A1 · 高：领域约束没有统一收口，调用入口不同会得到不同的合法性保证

`place_media`、`move_clips` 有较完整的兼容性和重叠规则，但公开的 `add_clip`、`move_clip`、`patch_clip` 仍能绕过部分约束。关联裁切的锁定、重叠和成组一致性检查又放在 interaction 中，并依赖 `DragGesture`。

证据：[公开 add_clip](D:/desktop/to_do/crates/todo_core/src/timeline/actions/add_clip.rs:6)、[公开底层修改接口](D:/desktop/to_do/crates/todo_core/src/lib.rs:20)、[关联边缘编辑](D:/desktop/to_do/crates/todo_interaction/src/timeline/group_edit.rs:5)、[粘贴实现](D:/desktop/to_do/crates/todo_interaction/src/clipboard/mod.rs:26)。

本轮最小复现得到：

- 将明确标为 Video、无音频的媒体通过 `add_clip` 放到 `a1`，操作被接受。
- 在 `v1` 已有 0～4 秒片段时，再通过 `add_clip` 添加 0～4 秒片段，重叠被接受。
- 复制 4 秒片段，在 59 秒粘贴后，内容末端达到 63 秒，序列时长仍为 60 秒；正常播放按 60 秒结束。

如果这些函数定位为不检查完整规则的内部原语，应限制可见性；对外统一使用验证后的领域命令。关联裁切、批量粘贴、冲突判定与序列延长规则应落到 core，输入层只负责给出目标时间和片段集合。

### A2 · 高：app 承担应用流程，interaction 同时承担应用会话和输入交互

`App` 管理 Screen、导入批次、任务匹配、保存回执、媒体缓存请求及工程动作。`EditorSession` 同时管理工程内容、保存快照、历史、剪贴板、播放时钟、预览命令、焦点与时间轴控制器。两者之间缺少清晰的应用用例边界。

证据：[App 状态](D:/desktop/to_do/crates/todo_app/src/main.rs:37)、[应用结果执行](D:/desktop/to_do/crates/todo_app/src/main.rs:221)、[导入流程](D:/desktop/to_do/crates/todo_app/src/main.rs:272)、[后台结果处理](D:/desktop/to_do/crates/todo_app/src/main.rs:331)、[EditorSession](D:/desktop/to_do/crates/todo_interaction/src/editor_session/mod.rs:105)。

建议将工程会话、编辑事务、历史、保存状态和任务状态迁入 `todo_application`。工具、焦点、选择、拖拽和缩放保留在 interaction；平台调用保留在 app / 外部适配器。不要仅把当前 main.rs 按长度拆成多个文件后宣称完成分层。

### A3 · 高：后台任务通道会让资源生成阻塞保存，失败重试缺少边界

`MediaBus` 只有一个串行 worker。工程保存、媒体上传、完整批次导入、波形和缩略图都经过同一队列。批量导入会处理完整个批次后才读取下一个 job。Node 的缩略图和探测又使用 `spawnSync`，会阻塞其事件循环。

波形失败时 app 只移除 inflight 标记，不记录失败或退避；下一帧会再次发起请求。缩略图有负缓存，但没有失效和重试策略。请求也不是按可见区域统一调度。

证据：[串行 worker](D:/desktop/to_do/crates/todo_app/src/media_bus.rs:110)、[导入批次循环](D:/desktop/to_do/crates/todo_app/src/media_bus.rs:226)、[波形请求](D:/desktop/to_do/crates/todo_app/src/main.rs:91)、[波形失败处理](D:/desktop/to_do/crates/todo_app/src/main.rs:416)、[同步缩略图生成](D:/desktop/to_do/backend/src/api/media/thumb.ts:66)、[探测实现](D:/desktop/to_do/backend/src/api/media/probe.ts:83)。

建议把任务状态与调度策略交给应用层，I/O worker 执行具体任务；区分工程持久化任务和可取消的资源生成任务，增加请求 ID、去重、失败退避和项目切换取消。当前结论来自调用链检查，未进行大型素材库压力测试。

### A4 · 高：保存和关闭流程尚未形成可靠闭环

关闭按钮直接发送 Close，`on_exit` 只停止预览，没有检查 dirty、保存确认或等待保存完成。Back 会提交保存并立即切回首页，失败时编辑会话已经离开。后端直接覆盖 project.json，未使用临时文件替换。

证据：[关闭按钮](D:/desktop/to_do/crates/todo_ui/src/widgets/chrome.rs:44)、[退出处理](D:/desktop/to_do/crates/todo_app/src/main.rs:612)、[返回动作](D:/desktop/to_do/crates/todo_interaction/src/editor_session/commands.rs:172)、[后端保存](D:/desktop/to_do/backend/src/api/project/save.ts:8)。

建议由应用层处理 RequestClose / RequestBack、保存中、保存成功、保存失败等状态；存储层负责原子保存。用户文案和确认窗口由 UI 展示，不能由 domain 直接弹窗。自动保存和恢复属于下一步能力，目前未发现实现。

### A5 · 中：UI 已经参与剪辑决策

时间轴 UI 计算目标轨道空闲情况，然后根据空闲与否决定传 `Some(track_id)` 还是 `None`，实际改变了素材放置策略；UI 还实现剃刀靠近播放头时的吸附，并在媒体拖拽载荷里判断可接受轨道类型。选择移动目标时也自行判断兼容性。

证据：[放置策略](D:/desktop/to_do/crates/todo_ui/src/widgets/timeline/board.rs:289)、[目标轨道参数](D:/desktop/to_do/crates/todo_ui/src/widgets/timeline/board.rs:326)、[剃刀吸附](D:/desktop/to_do/crates/todo_ui/src/widgets/timeline/board.rs:498)、[拖拽载荷兼容性](D:/desktop/to_do/crates/todo_ui/src/widgets/library/mod.rs:19)、[移动目标判断](D:/desktop/to_do/crates/todo_ui/src/widgets/timeline/clip.rs:147)。

应保留在 UI 的是：命中了哪个矩形、鼠标坐标、绘制占位图、显示可用状态。应迁出的则是：吸附目标选择、重叠如何处理、是否新建轨道、是否允许操作。interaction 生成统一的 DropPreview，core 在提交时再次验证。

### A6 · 中：动作入口重复，部分 Intent 实际无效

工程页使用 UI 自己定义的 `ProjectAction`，由 app 的 `on_home` 处理；interaction 的 `Intent` 又定义了 RefreshProjects、CreateProject、OpenProject、DeleteProject、OpenProjectFile，但检索不到处理分支。它们经过兜底分支最终成为无操作。

证据：[ProjectAction](D:/desktop/to_do/crates/todo_ui/src/widgets/projects.rs:7)、[工程 Intent](D:/desktop/to_do/crates/todo_interaction/src/intents.rs:172)、[分派兜底](D:/desktop/to_do/crates/todo_interaction/src/editor_session/commands.rs:178)、[实际工程入口](D:/desktop/to_do/crates/todo_app/src/main.rs:442)。

建议由应用层定义 AppAction / AppEvent / Effect，按工程与编辑会话拆分；输入事件单独定义。采用穷尽匹配，避免新动作静默落入通用 `_` 分支。`Event::Failed(String)` 也应附带操作类型和 ID，当前任何失败都可能被 app 当作当前导入失败处理。

### A7 · 中：预览布局有两条路径，其中一条已被逻辑短路

UI 把预览孔位作为返回值交给 app，同时还发送 `Intent::Layout` 进入 EditorSession。`on_layout` 在发出命令前先写入 `last_layout`，而 emit 仅在二者不相等时生成布局命令，因此该命令不会生成。

证据：[UI 布局事件](D:/desktop/to_do/crates/todo_ui/src/widgets/layout/mod.rs:90)、[提前写入](D:/desktop/to_do/crates/todo_interaction/src/editor_session/preview.rs:49)、[变化判断](D:/desktop/to_do/crates/todo_interaction/src/editor_session/preview.rs:102)、[另一条实际路径](D:/desktop/to_do/crates/todo_app/src/main.rs:460)。

本轮直接调用 `apply(Intent::Layout { ... })`，得到的预览命令数为 0。当前窗口仍可调整，依靠的是 app 的 sync_popup 路径。建议将窗口几何统一交给 UI → 桌面预览适配器，去掉工程会话中的重复布局路由。

### A8 · 中：Rust 与 TypeScript 两套 core 存在不同语义

根目录 `core/` 并非只有类型：其中有工程构造、默认轨道、时长计算以及完整的旧时间轴操作。后端仍调用 TS 的 createProject 加载工程。TS 的 projectDuration 会考虑媒体库素材时长；Rust 当前采用固定工作区 / 序列时长。不能直接删除整个 TS 目录，也不适合长期维护两套领域实现。

证据：[Rust 时长语义](D:/desktop/to_do/crates/todo_core/src/project.rs:66)、[TS 时长语义](D:/desktop/to_do/core/project.ts:34)、[后端加载时规范化](D:/desktop/to_do/backend/src/api/project/load.ts:10)。

建议以 Rust 领域规则为唯一来源。过渡期保留有明确 schema 的传输模型，清理未被后端使用的 TS 时间轴操作，并增加跨语言文档兼容测试。目标是将工程存储迁入 Rust，Node 只负责仍需要的 FFmpeg 媒体任务。

### A9 · 中：领域错误、显示模型和缓存归属需要细分

- core 的 `Result<Project, String>` 返回本地化显示文案，改为 DomainError；UI 做文案映射。[放置命令](D:/desktop/to_do/crates/todo_core/src/timeline/actions/place_media.rs:48)
- `LibraryItemView` 直接携带完整 MediaItem，UI 通过 interaction 重导出的领域函数自行分类和计算放置时长。应投影成专用显示字段与操作能力。[显示模型](D:/desktop/to_do/crates/todo_interaction/src/view_model.rs:124)
- 搜索在 interaction 先按名称过滤，UI 再按名称或路径过滤，路径匹配无法找回被前一层删除的条目。过滤规则应只有一份。[第一处过滤](D:/desktop/to_do/crates/todo_interaction/src/library/mod.rs:20)、[第二处过滤](D:/desktop/to_do/crates/todo_ui/src/widgets/library/mod.rs:123)
- `ThumbCache` 中的 egui 纹理缓存应留在 UI；原始峰值、素材生成状态、重试策略应属于媒体服务 / 应用层。不要把 GPU 纹理搬入 core。[当前纹理缓存](D:/desktop/to_do/crates/todo_ui/src/thumbs.rs:14)

### A10 · 中：大文件导入尚未端到端流式处理

Rust 上传端已使用 File 流，但 Node readBody 仍收集全部 chunks 并 Buffer.concat，随后 writeFileSync 整体写入。大视频仍会完整进入后端内存，且可能临时保留多份缓冲。

证据：[后端读取请求](D:/desktop/to_do/backend/src/server.ts:29)、[上传调用点](D:/desktop/to_do/backend/src/server.ts:140)、[媒体文件写入](D:/desktop/to_do/backend/src/api/media/upload.ts:6)。

建议媒体适配器直接流式落盘至临时文件，完成后提交；应用层负责进度、取消和失败状态。这里属于 I/O 设计问题，不应由 core 承担。

## 4. 与传统非线性剪辑软件的差距

当前已有基础剪辑、音视频关联、多选 / 框选、撤销重做、媒体缩略图、波形、图片导入 / 原生显示、面板尺寸调整和预览解码缓存。以下比较以基础制作工作流为准，不涉及 AI 功能。素材监视器与节目监视器的分工可参考 [Adobe 官方说明](https://helpx.adobe.com/ie/premiere/desktop/get-started/source-and-program-monitor-adjustments/about-source-monitor-and-program-monitor.html)。

| 能力 | 当前状态与主要不足 | 建议顺序 |
| --- | --- | --- |
| 成片输出 | 未发现时间轴导出、编码参数、导出任务或取消 / 失败恢复入口；还未形成完整制作闭环 | 第一阶段功能 |
| 工程可靠性 | 有手动保存和返回时保存；缺少关闭保护、原子保存、自动保存和崩溃恢复 | 优先于新功能 |
| 素材预剪 | 目前只有节目预览，导入会追加到时间轴；缺少独立 Source Monitor、源素材入出点和预剪后插入 | 第二阶段 |
| 常规编辑模式 | 有移动、剃刀、裁切、变速边缘、波纹删除；缺少完整 insert / overwrite、rolling、slip、slide、轨道目标与源轨映射 | 第二阶段 |
| 多轨视频合成 | 可保存多个视频轨，但 resolve_preview 返回第一个命中的画面，原生 presenter 接收单帧；没有多层合成、透明度、位置缩放与混合 | 与渲染基础一起规划 |
| 序列模型 | 一个 Project 直接带 tracks / fps；没有独立 Sequence、多序列、嵌套、序列尺寸 / 音频格式设置 | 模型演进阶段 |
| 效果与动画 | Clip 只有位置、时长、偏移、速率、关联 ID；没有效果栈、参数曲线、关键帧、转场 | 渲染模型稳定后 |
| 音频制作 | 已有播放、基础多片段混音与波形；领域输出增益固定为 1.0，没有用户可控的轨道 / 片段增益、声像、声道、包络、混音表与响度分析 | 第二至三阶段 |
| 媒体组织 | 平面列表和搜索，没有 bin、元数据列配置、离线标识、重连、代理绑定和归档流程 | 素材量增大前 |
| 播放性能 | 已有最新请求合并和有界解码帧缓存；仍缺代理链路、预览质量等级、后台预渲染、全链路性能观测 | 4K / 长项目阶段 |
| 时间精度 | 时间和 fps 主要使用 f64；没有明确的有理数帧率 / 时间类型、音频采样时间与时码显示策略 | 扩展专业格式前 |
| 工作区与可用性 | 面板可调整，但布局只在本次运行保存；缺少停靠 / 工作区预设、属性面板、可配置快捷键和完善的状态反馈 | 后续迭代 |

代理制作是成熟软件中独立的媒体工作流，可参考 [Adobe 创建代理说明](https://helpx.adobe.com/ie/premiere/desktop/organize-media/ingest-proxy-workflow/create-proxies.html)。不能把当前的解码帧缓存等同于代理系统。

相关实现证据：[Project 模型](D:/desktop/to_do/crates/todo_core/src/project.rs:12)、[Clip 模型](D:/desktop/to_do/crates/todo_core/src/timeline/clip.rs:10)、[单画面解析](D:/desktop/to_do/crates/todo_core/src/timeline/resolve_preview.rs:73)、[固定音频增益](D:/desktop/to_do/crates/todo_core/src/timeline/resolve_preview.rs:66)、[单帧 presenter](D:/desktop/to_do/native/src/presenter.h:4)。

## 5. 建议的职责边界

| 层 | 应拥有 | 不应拥有 |
| --- | --- | --- |
| todo_core | 工程 / 素材 / 轨道 / 片段与时间模型；兼容、重叠、关联、裁切和粘贴约束；结构化错误；预览 / 音频解析 | egui、窗口坐标、输入键值、焦点、选择工具、HTTP、磁盘、线程与本地化文案 |
| todo_application（新增） | 工程生命周期、编辑事务、历史、剪贴板用例、保存版本、播放协调、任务归属；Action / Event / Effect | 像素命中、控件绘制、具体文件对话框、ureq、HWND、进程协议序列化 |
| todo_interaction | 标准化输入、快捷键策略、工具、焦点、选择、拖拽、吸附、缩放、显示模型 | 直接修改工程字段、实现领域不变量、保存文件、发 HTTP、控制原生进程 |
| todo_ui | egui 绘制、命中哪个控件、原始输入翻译、字体配色、纹理与绘制缓存 | 轨道分流 / 冲突解决策略、成组编辑规则、工程保存和任务调度 |
| todo_storage（目标新增） | 工程文档 schema、迁移、验证入口、原子读写、工程路径、恢复文件 | 窗口行为、剪辑规则、用户选择和工具状态 |
| todo_media | 上传 / 探测 / 缩略图 / 波形 / 代理等媒体 I/O 适配 | 页面切换、历史、是否自动插入时间轴、工程编辑规则 |
| todo_preview + native | 原生音视频能力、协议、解码、呈现、音频设备、进程生命周期 | 决定选中哪些片段、删除规则、UI 工具、保存工程 |
| todo_app | main、eframe 事件循环、依赖组装、桌面桥接、执行 Effect 并回传 AppEvent | 决定剪辑是否合法、维护导入业务状态、另建一套工程管理状态机 |

注意：`todo_application` 是应用业务层；`todo_app` 是桌面启动与运行壳。名称相近，但职责明确不同。

建议的数据流：

```text
egui 原始输入 → interaction 的 InputEvent
             → EditorCommand（时间、ID、语义操作）
             → application 编辑事务 → core 校验并生成新工程
             → Effect → app / adapters 执行 I/O
             → AppEvent → application 更新状态
             → interaction 投影 ViewModel → UI 绘制

预览窗口几何：UI → 桌面预览桥接，独立于工程编辑事务。
```

应用层不依赖 interaction；选择、工具与像素信息由输入层持有，需要编辑时转为明确的片段 ID 与时间参数。外部结果必须带 operation_id / session_id，避免旧工程任务影响新会话。

## 6. 建议的目标目录

这是重构目标，尚未创建这些产品目录。第一步只需要建立 application 边界；storage 在迁移工程持久化时独立出来。

```text
crates/
  todo_core/src/
    model/                 # project、sequence、media、track、clip、time
    edit/                  # place、move、trim、split、delete、paste 的完整规则
    query/                 # overlap、linked、preview、audio 等纯查询
    error.rs
    lib.rs

  todo_application/src/
    projects/              # 打开、保存、关闭、恢复、迁移用例
    editor/                # 编辑事务、保存版本与会话
    history/
    clipboard/
    playback/
    media_jobs/            # 状态、请求 ID、取消、重试与优先级
    action.rs
    event.rs
    effect.rs
    lib.rs

  todo_interaction/src/
    input/
    tools/
    selection/
    timeline/              # 拖拽、吸附、坐标转换与缩放
    presentation/          # ViewModel、格式化、可用操作与放置预览
    lib.rs

  todo_ui/src/
    widgets/               # projects、library、monitor、timeline、layout
    style/                 # palette、fonts、controls
    render_cache/          # 缩略图纹理、波形绘制缓存
    lib.rs

  todo_storage/src/
    project_document/      # 文件版本与序列化 schema
    migrations/
    filesystem/            # 原子保存、恢复和路径
    lib.rs

  todo_media/src/
    client/                # 具体媒体服务适配
    imports/
    metadata/
    thumbnails/
    waveforms/
    lib.rs

  todo_preview/src/        # 保持原生进程适配职责
    host/
    protocol/
    lib.rs

  todo_app/src/
    main.rs
    runtime.rs
    adapters/              # desktop、preview_bridge、media_worker
    paths.rs

native/                    # 保留独立原生媒体工程
backend/                   # 过渡期兼容；最终聚焦媒体任务
docs/
  architecture/            # 分层规则、依赖图、协议与模型演进
  plans/
  reports/
```

Sequence 是后续多序列模型演进项，不需要为了第一次分层就强行实现。目录只在存在独立职责时创建；现有命名能表达职责的模块可以保留。

推荐依赖方向：

```text
application → core
interaction → application（只读会话快照与语义命令）/ core 的查询类型
ui          → interaction
storage     → core
media       → core 的媒体数据
preview     → 自身协议与进程能力
app         → 以上各层，负责装配
```

## 7. 迁移顺序与验收

1. **先建立行为基线。** 将本报告中的复现改为正式回归测试，补关闭保存、保存失败、后台任务归属测试；约定类型化错误和领域命令接口。
2. **收拢领域约束。** 成组裁切和粘贴规则进入 core；限制底层修改原语可见性；所有公开编辑命令得到一致的轨道、重叠、来源边界和序列时长保证。
3. **建立应用层。** 迁移历史 / 会话 / 播放协调 / 工程流程，统一 Action / Event / Effect；app 保留副作用执行，interaction 输出语义命令。
4. **净化 UI 边界。** 迁出放置策略和吸附；统一搜索；将完整领域模型投影为 ViewModel；删除重复布局路由。
5. **整理外部服务。** 隔离保存与资源生成，加入退避和取消，改为端到端流式导入；迁移 Rust 工程存储并逐步移除 TS 领域实现。
6. **再扩展剪辑能力。** 优先成片输出、素材预剪和标准编辑模式，随后规划合成、效果、代理与多序列。

验收应包含：Cargo 依赖白名单；core 禁止窗口 / 网络 / 文件系统依赖；应用层禁止依赖 UI；公开编辑命令的约束测试；后台事件归属测试；旧版本工程契约测试；真实 UI 到保存 / 预览适配器的集成测试。当前未发现 CI 中对此类边界的自动检查。

## 8. 本轮验证

- `cargo test --workspace`：当前版本 116 项测试通过。仍有 `ImportMedia` 未使用枚举分支警告。
- `python -m unittest discover -s native/tests -v`：当前已构建原生预览程序的 9 项协议 / 解码回归测试通过。
- Cargo metadata：六个现有 crate 的内部依赖方向与本报告一致，无 crate 循环。
- 临时复现输出：`layout_command_count = 0`；粘贴后 `content_end = 63`、`sequence_duration = 60`；公开 add_clip 接受轨道类型不兼容和重叠。
- 没有进行 4K / 长项目的内存、帧时延和后台负载压力测试，也没有进行真实桌面关闭恢复验证。对应性能风险按静态调用链分析列出；功能缺失按当前源码入口与模型判断。

已有测试通过说明现有覆盖范围内的行为稳定，不能证明模块边界或所有编辑入口已经满足统一约束。
