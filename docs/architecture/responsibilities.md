# 项目文件夹分工

更新：2026-10-16。本文件描述当前已经落地的结构。

## 各层职责

| 文件夹 | 职能 | 新代码放置原则 |
| --- | --- | --- |
| `crates/todo_core/src/model/` | 工程、素材、轨道和片段的数据模型；片段来源 `ClipSource`（媒体/文字） | 工程需要保存的业务数据放这里；变换模型独立放在 transform/ |
| `crates/todo_core/src/model/text.rs` | 花字标准：文字内容、样式图层、单位、入场动画采样、内置样式和栅格摘要 | 花字数据唯一来源；不含字体读取和像素运算，见 [花字层标准](text-layer.md) |
| `crates/todo_core/src/transform/` | 位置、统一/二维缩放、不透明度的持久化模型、校验和纯几何；元素基准尺寸 `BaseSize` | 变换数据的唯一来源；不包含手柄、鼠标或绘制代码 |
| `crates/todo_core/src/edit/` | 放置、移动、裁切、变速、剃刀、删除、粘贴、锁定等完整规则 | 判断编辑是否合法、如何修改工程，放这里 |
| `crates/todo_core/src/edit/track_space.rs` | 素材入轨与片段移动共用的空闲检测和轨道分配 | 同轨区间冲突、复用空轨、按需新增同类型轨道在此统一 |
| `crates/todo_core/src/query/` | 查片段、关联关系、可放置状态、预览画面和音频场景 | 只读领域查询放这里 |
| `crates/todo_application/src/projects/` | 打开、创建、导入、保存、返回、关闭工程的流程和回执 | 跨步骤的工程用例放这里 |
| `crates/todo_application/src/editor/` | 编辑会话、事务、dirty 状态、保存快照、预览更新计划 | 一次拖动一次撤销等会话规则放这里 |
| `crates/todo_application/src/control/`、`projects/control.rs` | 共用编辑协议、批次、回执、去重和导入状态 | 供内置/外部 AI 共用；不含模型、HTTP/MCP、UI 或 I/O，规则仍由 core 执行 |
| `crates/todo_application/src/history/`、`clipboard/` | 历史快照、剪贴板内容 | 不持有控件和鼠标状态 |
| `crates/todo_application/src/playback/` | 播放状态、播放头推进和音频时钟协调 | 只计算命令，不调用原生进程 |
| `crates/todo_application/src/media_jobs/` | 任务归属、请求去重、并发上限、失败退避、原始波形缓存 | 不包含 HTTP、线程或 GPU 纹理 |
| `crates/todo_application/src/text_rasters/` | 预览需要哪些文字栅格、是否就绪、测得的文字块 | 只记身份和结果；渲染与文件在 worker |
| `crates/todo_interaction/src/input/` | 标准化输入、焦点、选择、预览拖动、输入到语义命令的翻译 | 鼠标或快捷键代表什么操作，放这里 |
| `crates/todo_interaction/src/timeline/` | 工具、拖拽、框选、缩放、像素到时间换算、统一吸附 | 不直接执行工程修改；提交领域命令 |
| `crates/todo_interaction/src/inspector/` | 属性栏的片段投影与输入翻译；`presentation.rs` 提供属性字段，`input.rs` 生成语义编辑命令 | 属性展示和操作在此组织，业务合法性仍由 core 校验 |
| `crates/todo_interaction/src/transform/` | 控制框命中、移动和八手柄缩放的手势计算 | 将鼠标动作转换为 core Transform；不保存或渲染画面 |
| `crates/todo_interaction/src/preview_viewport.rs` | 编辑视口的缩放、平移、固定画框投影和鼠标坐标换算 | 临时观看状态，不进入工程、撤销历史或解码状态 |
| `crates/todo_interaction/src/presentation/` | Editor/Library/Timeline 的 ViewModel、筛选和状态文案 | 展示哪些字段、领域错误如何显示，放这里 |
| `crates/todo_ui/src/widgets/` | 原生 egui 控件绘制；按 `library/`、`timeline/`、`inspector/`、`layout/` 等分组 | 绘制矩形、命中控件、输出 Intent，放这里 |
| `crates/todo_ui/src/widgets/inspector/` | 独立右侧属性板块；`mod.rs` 管理面板标题、空状态和滚动，`clip.rs` 绘制片段字段 | 属性内容不决定节目预览的大小 |
| `crates/todo_ui/src/widgets/inspector/transform/` | 变换参数控件：位置、宽高缩放、统一缩放、不透明度和重置 | 只输出参数修改 Intent，规则不写入控件 |
| `crates/todo_ui/src/widgets/controls/` | 多个板块共用的控件交互；当前包含数值编辑事务辅助 | 工具栏与属性栏共用的逻辑放这里，避免互相依赖 |
| `crates/todo_ui/src/theme.rs`、`thumbs.rs`、`format.rs` | 字体配色、GPU 缩略图纹理、纯显示格式 | UI 专属的表现细节放这里 |
| `crates/todo_storage/src/projects/` | 本地工程仓库接口 | 工程持久化从这里进入 |
| `crates/todo_storage/src/project_document/` | JSON 编解码、版本检查、领域校验入口 | 文档格式兼容放这里 |
| `crates/todo_storage/src/filesystem/` | 路径限制、原子写入、回收目录 | 磁盘操作集中在这里 |
| `crates/todo_media/src/service.rs`、`error.rs` | 原生 MediaService 配置和统一错误 | worker 直接调用；不包含 HTTP 或服务端 |
| `crates/todo_media/src/imports/`、`files/` | 本地素材流式复制、临时文件发布和来源路径解析 | 工程目录通过 storage 获取；失败清理，不修改源文件 |
| `crates/todo_media/src/metadata/` | FFprobe 输出解析和媒体分类 | 时长、音视频流、静态图片和封面排除规则 |
| `crates/todo_media/src/thumbnails/` | JPEG 缩略图生成 | 参数校验与实际媒体任务 |
| `crates/todo_media/src/waveforms/` | 流式 PCM 解析、音频帧与峰值桶 | 内存只保留固定缓冲区和峰值，测试独立放 tests.rs |
| `crates/todo_media/src/cache/` | 素材身份/参数缓存、同键任务去重和原子写入 | 缓存不可用不影响有效结果 |
| `crates/todo_media/src/text/` | 文字栅格 PNG 缓存、字体目录查找 | 由文字专用线程持有；调用 todo_text 渲染 |
| `crates/todo_text/` | 花字栅格化：字体库、排版、图层（填充/描边/偏移/缩放/模糊/做旧）合成 | 纯计算，只读字体文件；不写缓存、不知道工程和预览 |
| `crates/todo_media/src/tools/` | 工具定位和隐藏子进程的有界输出、超时回收 | FFmpeg/FFprobe 统一在此调用，不散落到 UI 或 application |
| `crates/todo_preview/src/` | 原生宿主生命周期和协议 | 传输命令、接收时钟及归一化鼠标事件 |
| `crates/todo_preview/src/viewport.rs` | 视口命令校验与导航事件协议类型 | 只传递视口矩形和输入，不计算交互或修改图层 |
| `crates/todo_app/src/runtime/` | eframe 帧循环、绘制调用、输入/事件转交、执行 Effect | 只做装配和运行，不另写编辑规则 |
| `crates/todo_app/src/adapters/` | `workers` 执行阻塞 I/O；`preview` 桥接预览；`desktop/` 处理窗口和文件对话框 | 桌面平台能力放这里 |
| `dependencies/ffmpeg/` | FFmpeg/FFprobe 原生工具和随附说明 | 不存放应用逻辑，不参与 Rust 链接；由打包脚本复制 |
| `dependencies/fonts/` | 花字字体文件和授权说明 | 字体文件不入库；由打包脚本复制 |
| `tests/support/` | Rust 测试进程模拟器 | 复用测试可执行程序，协议测试无需 Node 或运行时编译器 |
| `native/src/` | Windows 解码、D3D 呈现、音频、原生鼠标捕获 | 不决定删除、选择、历史或工程保存 |
| `native/src/transform/` | 原生变换计算、显示几何、控制框、光标和着色器 | 只处理画面呈现与平台反馈，不修改工程 |
| `native/src/viewport/` | 编辑视口的原生投影、画框及画外区域着色 | 与解码分离，更新视口时重绘保留帧；鼠标事件仍按窗口坐标上报 |
| `docs/architecture/`、`docs/reports/`、`docs/plans/` | 分工约定、检查报告、实施计划 | 文档按用途分类 |
| `data/` | 用户工程与原始媒体；删除工程移入 `.trash/` | 不是测试目录，不存放源码 |
| `target/`、`native/build/` | 测试临时数据、构建产物；原生可执行文件在 `native/build/bin/` | 不提交生成文件 |

## 依赖规则

```text
UI → interaction → application → core
                 ↘ core（只读查询和类型）
storage → core
media → storage → core（工程媒体目录由 storage 校验）
media → core（素材模型）
media → text → core（文字栅格；app 只经 media 使用 text）
preview → 原生协议
app → 组装以上各层
```

- core 和 application 不依赖 egui、HTTP、文件系统适配器或原生窗口。
- UI 的生产代码只从 interaction 获取展示模型和输入类型，不直接拿 Project 修改。
- interaction 保存选择、工具、焦点、手势；application 保存工程会话和用例状态。
- 裁切、重叠、来源边界、轨道兼容、锁定等最终约束由 core 统一执行。
- 窗口孔位走 UI → 桌面预览桥接；面板尺寸不进入工程、不产生撤销记录。
- Effect 描述副作用；app 执行后回传带操作 ID / 会话 ID 的 AppEvent。切换工程后旧回执失效。
- `tests/dependency_boundaries.rs` 检查生产依赖。新增依赖时需要同时说明层次归属。
- application 使用 serde 编解码共用能力层 DTO；这是数据协议依赖，不引入网络或文件系统。接口约定与运行时接法见 [共用编辑能力层](shared-control.md)。
- 媒体处理完全在 Rust worker 中执行，FFmpeg 仅为按需启动的原生工具。目录和打包约定见 [Rust 媒体处理](native-media.md)。
- 时间轴精确贴边、关联移动分轨和手势临时轨道的职责见 [时间轴编辑](timeline-editing.md)。

## 功能应该改哪里

| 需求 | 主要修改位置 |
| --- | --- |
| 新增一种裁切/删除规则 | core/edit → application 命令 → interaction 输入 → UI 入口 |
| 增加按钮、调整颜色、直角样式 | UI/widgets、theme |
| 改吸附范围、鼠标手柄、框选或快捷键行为 | interaction/timeline、input；UI 只调整命中和提示 |
| 新增工程自动保存或恢复流程 | application/projects + storage；app 只执行任务 |
| 增加导入进度、重试或取消策略 | application/media_jobs；适配器落实 I/O |
| 改视频解码、呈现或音频设备 | native + todo_preview；通过协议暴露能力 |
| 新增花字样式或图层效果 | core/model/text.rs（数据与校验）→ todo_text（绘制）；见 [花字层标准](text-layer.md) |

## 本轮基础编辑补全

- 在预览画面按住左键拖动，修改当前可见片段的位置；移动到中心附近会吸附。属性区可输入 X/Y、缩放百分比并重置画面。
- 拖片段左右端修改入出点和时长；按 Alt 拖端点改变播放速率。数值时长也走同一裁切规则。
- 素材放置、片段移动的左右边、裁切端点、剃刀共用吸附计算；目标包括零点、播放头和其他片段边缘。关闭吸附仍保留帧对齐。
- 一次手势形成一次撤销；Escape 恢复手势前内容。轨道锁定、相邻片段、源素材边界和关联音视频统一校验。
- 保存、导入、资源生成分队列；资源任务最多 12 个在途请求，同请求去重，失败按 2/4 秒退避，第三次失败后停止本会话自动重试。
- 本地保存使用临时文件和原子替换；返回等待保存成功；关闭提供保存/放弃/取消。保存期间有新编辑时，继续保存新版本再离开。
- 属性栏固定占据节目预览右侧，可独立调整宽度；选择、切换或取消选择片段不改变预览矩形。播放/暂停位于预览下方。
- Transform 的位置使用画布宽高归一化偏移，大小使用统一缩放和横向/纵向倍率，不透明度为 0..=1。预览控制框是临时显示状态，直接由 interaction 投影并经桌面适配器转发，不进入工程和历史。
- 解码器提供考虑像素比例与可见区域的实际显示比例；preview 传输带素材请求代次的回执，app 排除旧请求，interaction 按素材路径保存临时比例，core 只接收比例计算边界。源视频尺寸与实际显示比例不混用。
- 轨道标题左侧“＋”在被点击轨道下方插入同类型空轨道，支持撤销/重做。链接选择开关只改变选择行为，不创建或解除链接。

多层预览由 core 查询从底到顶的可见图层（最多保留顶层四层），application 生成更新计划，preview 传输整组图层，native 负责解码池与 GPU 透明合成。拖动中的位置、缩放和不透明度更新复用已解码纹理；新图层整组就绪后再替换显示，过期完成通知不清屏，显式空图层立即清屏。相关回归测试放在 `native/tests/`，生成夹具放在 `target/preview_tests/`。

编辑视口默认将固定 16:9 输出画框适应面板，支持 10%–400% 的相对适应倍率、滚轮围绕指针缩放、中键平移、适应复位和仅输出查看。画框外元素在编辑模式加暗显示并可拖入画内；仅输出模式遮蔽画外内容和控制框。interaction 统一把原生窗口坐标换算为画布坐标，app 只转发投影矩形与导航事件；选框与光标使用投影后的窗口坐标。视口状态只在会话中保留，不改工程、元素 Transform 或撤销记录。目前工程尚无输出分辨率字段，后续输出设置应为视口提供画布比例。

目前仍是基础编辑器：尚未实现关键帧、转场、完整导出、代理、自动保存和崩溃恢复。面板大小只在当前运行内记忆。
