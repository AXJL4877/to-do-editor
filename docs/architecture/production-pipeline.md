# 成片基础管线

更新：2026-10-17。本文件是扩展开发和后续功能接入时的路径与入轨约定。它描述当前已经落地的最小管线：扩展只负责产出条目，宿主负责服务、文件落盘、媒体登记和时间轴编辑。

## 1. 一条产物如何到达时间轴

```text
脚本 / 参数
    │
    ▼
ProductionSession / extension worker
    │  ctx.call("chat" | "image" | "speech" | "subtitles" | "encode")
    │  每完成一个阶段就 emit(items)
    ▼
production workdir
    ├─ services/    宿主服务结果（按任务 key 固定命名）
    └─ out/         worker 的命名产物（板、逐元素图片、BGM 等）
    │
    ▼
RunItems → ProductionSession 缓存条目
    │
    ▼
placement::plan
    ├─ File 尚未登记：生成 adopt 清单，等待媒体登记
    ├─ File 已登记：生成 GeneratedEdit
    └─ Text：直接生成文字 GeneratedEdit
    │
    ▼
Editor::ApplyGenerated（一个批次 = 一个撤销步）
    │
    ▼
Project.media + Project.tracks → 预览 / 导出
```

对应代码位置：

- 条目协议：`crates/todo_production/src/extension/items.rs`
- 入轨规则：`crates/todo_production/src/placement.rs`
- 成片状态机：`crates/todo_application/src/productions/session.rs`
- 媒体登记和批次应用：`crates/todo_application/src/projects/productions.rs`
- AI/本地服务桥：`crates/todo_app/src/adapters/production/bridge.rs`
- worker 事件转接：`crates/todo_app/src/adapters/production/host.rs`

## 2. 每类产物放在哪里

每个成片都有独立目录：

```text
data/<project>/productions/<production-id>/
├─ production.json       成片状态、参数、已产出条目和入轨记录
└─ work/                 worker 和服务共同使用的工作目录
   ├─ services/          宿主服务物化的结果
   └─ out/               worker 命名的交付产物
```

条目里的 `source.file` 始终是相对于 `work/` 的路径。项目媒体登记后，项目文件中的媒体路径是同一份相对路径拼接到 production workdir 的结果。

| 产物 | 生成入口 | 典型位置 | 条目角色 / 例子 |
| --- | --- | --- | --- |
| 生图图片 | worker `ctx.call("image", ...)` | `work/services/<task-key>.<ext>`；需要稳定名字时复制到 `work/out/` | `visual`，如 `shot/0/character`、`b1/e2` |
| 黑板底板、逐元素图、片头图 | worker 自己绘制或复制模板 | `work/out/plate_1920x1080.png`、`work/out/<board>/...`、`work/out/intro-red.png` | `visual` |
| 文字 / 花字 | 条目直接使用 `ItemSource::Text { content }` | 不依赖图片文件；`TextContent` 是源数据 | `text` 或 `subtitle` |
| 花字栅格缓存 | 预览或媒体层按需栅格化 | `todo_media` 的文字缓存，不是扩展交付源 | 由 `TextContent` 派生，不能反向替代源文本 |
| 配音 | worker `ctx.call("speech", ...)` | `work/services/<task-key>.wav`，同时返回真实时长和句/词时间线 | `voice`，如 `voice/0` |
| 字幕 | worker `ctx.call("subtitles", ...)` | 时间线结果进入 `ItemSource::Text` | `subtitle` |
| BGM | worker 从扩展包素材复制，或以后接入音乐服务 | `work/out/bgm-<name>.mp3` | `music`，如 `bgm/intro`、`bgm/mid`、`bgm/outro` |
| 音效 | worker 文件或服务结果 | `work/out/` 或 `work/services/` | `sfx` |
| 透明动画 | worker 输出帧目录，宿主 `ctx.call("encode", { alpha: true })` | 编码结果通常在 `work/services/<task-key>.mp4` | `visual` |

服务文件由宿主按请求 key 固定为 `services/<key>.<ext>`。同一个请求重跑时复用同一路径和缓存，不产生第二份付费结果。worker 的 `out/` 文件名由扩展负责，适合需要在调试、交付和后处理阶段直接定位的命名产物。

## 3. 条目是唯一的入轨接口

扩展向宿主发出的每一条 `Item` 至少包含：

```text
key       稳定的逻辑身份，例如 b1/e2、voice/0、bgm/intro
rev       内容版本；输入或产物变了才递增
role      voice | music | sfx | visual | text | subtitle
lane      同一角色内的层序，0 在下方
start     相对成片起点的秒数
duration  时长
source    File(workdir 内相对路径) 或 Text(可编辑 TextContent)
```

可选的 `transform`、`intro`、`link`、`gain`、`fade` 只描述入轨属性，不改变文件归属。

推荐的 key 规则：

```text
<segment>/<element>       画面元素，例如 s3/e2 或 b1/e2
<segment>/voice            该段配音
<segment>/subtitle/<n>     字幕条目
voice/<n>                  直接按脚本段编号的配音
bgm/<part>                 片头、主体、片尾音乐
```

同一逻辑产物在重跑时保持 key 不变；只在内容改变时改 rev。这样宿主才能区分“同一片段更新”和“新片段”。

## 4. 渐进式入轨的实际步骤

1. **worker 分阶段产出。** 例如先发 BGM 或第一段配音，拿到第一张图后立即再发画面条目，不必等整条片结束。一次 run 内每个 key 只发一次；重跑时用同一 key 配合新的 rev 表示更新。
2. **宿主校验并保留结果。** `RunItems` 进入当前 production 的运行缓冲；workflow 会同步保存条目，narration 会在 run 完成后把缓冲结果并入布局。文件条目必须位于 workdir 内，文字条目保留内联 `TextContent`。
3. **先登记文件，再入轨。** `placement::plan` 遇到尚未登记的 `File` 时，把完整路径放入 `PlacementPlan.adopt`，本轮跳过该条目。application 发出媒体登记任务；登记完成后下一次 `place_productions` 会重新规划同一批条目。
4. **生成一个编辑批次。** 已登记的文件和内联文字会转成 `GeneratedEdit::Place` 或 `GeneratedEdit::Update`。application 用一次 `ApplyGenerated` 提交，因此一次产出批次对应一个撤销步。
5. **后续批次按 key/rev 合并。**
   - 新 key：放到 role/lane 对应轨道。
   - 相同 key + 相同 rev：不重复放置。
   - 相同 key + 新 rev：原位替换，保留用户已经改过的变换；用户没有移动时采用新的制作时序。
   - 用户移动过：后续重跑保留用户位置；`link` 条目跟随父条目的实际平移。
   - 用户删除过：同一 rev 不复活；rev 改变后允许重新生成。
   - `complete = true`：移除本次最终结果中不再出现、且用户没有移动的旧条目；部分 `emit` 不会误删旧条目。
6. **保存和继续。** 成功入轨后保存 production placement 记录。关闭工程再打开时，production.json 中的条目和 placement 状态可以继续同一条管线。

生成完成或用户觉得轨道过多时，时间线工具栏的“整理轨道”会执行一次普通可撤销编辑：它只压缩相邻的未锁定同类轨道，把不重叠的片段装进同一轨道；重叠片段、锁定轨道和静音边界保持原有关系。

这套流程允许画面、配音、字幕和 BGM 以不同速度到达，时间轴始终显示当前已完成部分；后来的产物只通过稳定 key/rev 更新已有内容。

## 5. 四类素材的最小接入模板

### 图片素材

```text
image service → services/<task>.png
worker 选择是否复制为 out/shot-00-character.png
emit Item { role: Visual, source: File { file: ... } }
```

图片是文件型视觉条目。图片只要进入 workdir 并在下一轮媒体登记完成，就可以单独显示；不需要等待配音或字幕。

### 文字花字

```text
worker 直接构造 TextContent
emit Item { role: Text, source: Text { content: ... } }
```

花字的内容、样式和动画参数以 `TextContent` 为源数据，编辑器可以继续编辑。需要位图预览时由 `todo_media/src/text` 派生缓存；扩展不应把缓存 PNG 当成花字的唯一来源。

### 声音

```text
speech service → services/<task>.wav + timeline
emit Item { role: Voice, source: File { file: "services/<task>.wav" } }
```

配音条目使用真实 WAV 时长。字幕服务使用同一份 speech 结果的时间线，不再自行猜测字幕节奏。

### BGM

```text
worker 复制扩展包 assets/ 到 out/bgm-*.mp3
emit Item { role: Music, lane: 1/2/3, key: bgm/intro|mid|outro }
```

BGM 作为独立音乐轨进入工程，便于后续替换、分段、淡入淡出和独立调音。BGM 不应塞进配音文件或整段导出视频。

## 6. 当前基础边界

当前管线已经提供：

- workdir、`services/`、`out/` 的持久目录约定；
- `Item` 的稳定 key/rev、角色、轨道 lane、文件/文字双源；
- 文件先 adopt、登记后入轨的两阶段处理；
- 新增、更新、用户移动、用户删除、关联移动和最终清理规则；
- 每批产出一个可撤销的 `GeneratedEdit`；
- narration 和 workflow 两种扩展都走同一套条目与入轨规则。

后续扩展只需先决定三件事：产物写入 `services/` 还是 `out/`、条目的稳定 key/rev、以及条目出现的阶段。路径索引面板、产物清单 UI、逐条入轨日志属于下一层便利功能，不影响这条基础管线。

## 7. 最小验证

在仓库根目录运行：

```powershell
cargo test -p todo_production --quiet
cargo test -p todo_application --quiet
cargo check -p todo_app --quiet
```

`todo_production` 覆盖条目校验和渐进入轨规则，`todo_application` 覆盖从产出事件到媒体登记、批次入轨和保存的状态机，`todo_app` 检查宿主装配仍能编译。
