# TO DO 扩展规范 v1

更新：2026-10-17。本规范是扩展作者、宿主与 agent 的唯一约定。规范的每一条都有对应的 Rust 类型与校验（`crates/todo_production/src/extension/`），文档与代码不一致时以代码测试为准并修正文档。

产物目录、图片 / 花字 / 配音 / BGM 的落点，以及 `emit → adopt → GeneratedEdit` 的渐进入轨步骤见[成片基础管线](production-pipeline.md)。

## 1. 原则

1. **一种格式，两种写法**。`workflow` 是通用写法：工作程序编排任意流程——代码生成的动画、对选中素材加特效、AI 口播成片都可以，输入、服务、步骤都不受限制。`narration` 是可选的便捷写法：复用宿主的口播流水线（分段 → 配音 → 分镜 → 生图 → 烘焙 → 字幕 → 入轨），只需清单、提示词和可选的烘焙程序。两者产出同一种「时间轴条目」，入轨、编辑、导出完全相同。
2. **扩展不联网、不持有密钥**。宿主服务按需使用，不用就不声明；对话、生图、配音等一切付费调用都经协议交给宿主。宿主按请求指纹去重、缓存、重试、熔断、限流、计费预算；同样的请求永远只付一次钱。
3. **可重跑**。工作程序必须是「给定相同输入与服务结果就产出相同条目」的确定性程序（随机数用宿主下发的 `seed`）。宿主在用户修改后直接重跑流程，已做过的服务调用从缓存瞬间返回，因此「只重做改动部分」对所有扩展自动成立，作者无需自己管理缓存。
4. **渐进入轨**。产物一出来就以条目发出，宿主立即放上时间轴；条目有稳定的 `key` 与版本 `rev`，用于原位替换、保留用户改动和不复活用户删除的片段。
5. **可编辑优先**。能拆成独立图层就不烤成整片：静帧 + 宿主动效、书写回放（reveal）、文字（花字）、独立音轨优先于整段视频。

## 2. 扩展包

`.todoext` 是 zip 包，解压后目录结构：

```text
extension.json        清单（必需）
prompts/              提示词模板（Markdown / 纯文本），清单按相对路径引用
assets/               静态素材：字体、音效、图片、底板、LUT…
worker/               工作程序（workflow 必需；narration 需要烘焙时必需）
previews/             预览图（png/jpg/webp）
tests/                自检样例：输入、录制的服务结果、期望产出
README.md
```

安装时宿主校验路径（拒绝绝对路径、`..`、Windows 保留名）、单文件 ≤ 256 MiB、总计 ≤ 1 GiB、文件数 ≤ 4000，**安装过程不执行任何包内代码**。内置扩展位于 `extensions/builtin/<id>/`，用户安装的位于 `extensions/user/<id>/`；同 id 时用户版本覆盖内置版本。

## 3. 清单 `extension.json`

```jsonc
{
  "spec": "todo-extension/1",
  "id": "builtin.chalk-hand",            // 作者.名称，小写字母数字和 -
  "name": "黑板手绘",
  "version": "4.0.0",                    // 语义化版本；产物指纹包含它
  "description": "长口播一键成片……",
  "kind": "narration",                   // workflow（通用）| narration（口播配方）
  "input": {                             // 可省略：只靠参数驱动（如代码生成的片头）
    "script": { "label": "口播文案", "maxLength": 12000 },
    "media":  { "label": "要处理的片段", "kinds": ["video", "image", "audio"], "min": 1, "max": 4 }
  },
  "params": [ /* §4 */ ],
  "quick": ["voice", "size"],            // 侧栏直接露出的参数（≤ 6）
  "requires": {
    "services": ["chat", "image", "speech"],   // 只能调用声明过的服务
    "runtime": { "type": "python", "version": ">=3.12" }   // 无工作程序时省略
  },
  "worker": {                            // 工作程序；narration 无烘焙时省略
    "entry": "worker/main.py",
    "ops": ["bake_board"],               // narration 用烘焙操作；workflow 必须含 "run"
    "timeoutSec": 900,
    "maxProcesses": 4,
    "memoryMiB": 1500
  },
  "narration": { /* §6，仅 kind = narration */ },
  "budget": { "image": 300, "chat": 120, "speech": 200 },   // 单次成片的调用上限
  "previews": ["previews/01.png"]
}
```

未知字段一律拒绝（防拼写错误静默失效）。

## 4. 参数

```jsonc
{ "id": "size", "type": "select", "label": "画幅", "group": "scene", "default": "1920x1080",
  "options": [ { "value": "1920x1080", "label": "16:9 · 1080p", "hint": "…", "prompt": "…" } ] }
```

| type | 字段 | 说明 |
| --- | --- | --- |
| `select` | `options[]`（1~256，值 `[A-Za-z0-9_.:-]{1,32}`，可带 `prompt` 片段） | 每个选项都有 `prompt` 时可作为提示词占位符 `{{id}}` |
| `number` | `min` `max` `step` `unit?` | 越界拒绝，按 `step` 吸附 |
| `text` | `maxLength`（≤ 2000） `charset?` `placeholder?` | `charset` 列出字母数字之外允许的符号 |
| `bool` | — | |
| `voice` | — | 宿主的音色选择器，值为音色 id（克隆音 `S_…`） |

公共字段：`id`（`[a-z][a-zA-Z0-9_]{0,31}`，唯一）、`label`、`group`（`story` `voice` `scene` `timing` `stroke` `text` `output` `other`）、`default`（必须合法）、`hint?`、`when?`（`{ "<select 参数 id>": ["值", …] }`，仅控制显示）。宿主保存的是**完整解析后的参数**：缺省补默认、未声明键拒绝、越界拒绝。

## 5. 服务（扩展向宿主请求）

| service | request | result |
| --- | --- | --- |
| `chat` | `{ messages[{role,content}], json?: bool, temperature?: 0~1, model?: "default" }` | `{ text }` |
| `image` | `{ prompt, size: "WxH", quality?, transparent?: bool, variant?: n, model?: "default" }` | `{ file, width, height }` |
| `speech` | `{ text, voice: "<音色 id>", sampleRate?: 24000 }` | `{ file, duration, timeline }`（句 / 短语 / 字三级时间，已按真实音频修正） |
| `subtitles` | `{ script, speech: "<speech 结果 file>", maxChars?: 18 }` | `{ cues[{text,start,end}] }` |
| `probe` | `{ file }` | `{ duration, width?, height?, hasAudio }` |
| `encode` | `{ frames: "<目录>", fps, alpha?: bool, quality?: "standard"\|"high", audio?: "<文件>" }` | `{ file, duration, width, height }` |

`encode` 让代码生成动画的程序只需逐帧画 PNG（`<目录>/000001.png` 起连续编号）：宿主用自带 FFmpeg 按统一质量标准编码，透明时输出 packed-alpha，不透明时输出短 GOP H.264，预览拖动与导出质量都由宿主保证。所有服务都按需使用，没有声明的服务不能调用，声明了也不强制使用。

模型名一律写 `default` 或省略（由宿主配置决定）；`variant` 区分同一提示词的多张候选。结果里的 `file` 是工作目录内的相对路径（宿主已放好的副本）。失败时返回 `{ class, message }`，`class` 为 `invalid`（请求本身不合法）、`content_policy`（被审核拒绝，可改写提示词再试）、`budget`（超出清单预算）、`unavailable`（宿主已尽力重试仍失败）。网络抖动、限流、断线续传都由宿主处理，**扩展不要自己重试**。

## 6. `narration` 配置

```jsonc
"narration": {
  "planner": { "prompt": "prompts/planner.md", "maxElements": 4 },
  "assets": {
    "template": "prompts/asset.md",      // 必含 {{draw}}；可用 {{label}} {{role}} 及带 prompt 的 select 参数
    "roles": { "hero": "…", "support": "…", "accent": "…" },
    "avoid": ["photograph", "…"],
    "size": "1024x1024", "transparent": false
  },
  "segment": { "target": 120, "max": 200 },
  "boardSec": 10,
  "pacing": { "transition": 0.5, "minHold": 1.2 },   // 可省略，默认值见 pacing.rs
  "subtitles": { "maxChars": 18 },                   // 省略则不出字幕
  "bake": "bake_board"                               // 省略则元素以静帧 + 宿主入场动效入轨
}
```

宿主负责分段、配音、分镜、生图、排期（含换板过渡与留白）、字幕和入轨；烘焙操作只负责把一块黑板的元素图做成图层条目。

口播配方识别以下约定参数名（声明了就由用户调整，没声明就用清单值）：`voice`（voice 类型）、`split`（`semantic`/`paragraph`/`length`）、`segmentChars`、`boardSec`、`variants`、`subtitles`（bool）、`subtitleChars`、`tail`。只有 `narration` 要求文案输入和 `chat` / `image` / `speech` 服务。

## 7. 工作程序协议 `todo-worker/1`

工作程序以子进程运行（当前目录为扩展包目录），**stdout 每行一条 JSON 消息，stderr 是日志**（宿主截断保留）。环境变量中没有任何密钥。进程会在多次运行之间复用，所以工作目录 `workdir`（每个成片一个，写出的文件都必须在里面）和随机种子 `seed` 随每次 `run` 下发。

宿主 → 程序：

```jsonc
{"type":"hello","spec":"todo-worker/1","host":"0.2.0","package":"…"}
{"type":"run","id":"r1","op":"run","input":{…},"params":{…},"canvas":{"width":1920,"height":1080,"fps":30},"workdir":"…","seed":12345}
{"type":"reply","to":"c7","ok":true,"result":{…}}
{"type":"reply","to":"c7","ok":false,"error":{"class":"content_policy","message":"…"}}
{"type":"cancel","id":"r1"}
{"type":"shutdown"}
```

程序 → 宿主：

```jsonc
{"type":"ready","spec":"todo-worker/1","ops":["run"],"version":"1.0.0"}
{"type":"call","id":"c7","run":"r1","service":"image","request":{…}}
{"type":"progress","run":"r1","phase":"生图","fraction":0.4}
{"type":"emit","run":"r1","items":[ /* §8 */ ]}
{"type":"done","run":"r1","result":{}}
{"type":"failed","run":"r1","error":{"class":"invalid_input","message":"…"}}
```

- 一个进程同一时刻只执行一个 `run`，但可以并发发出多个 `call`（宿主按清单 `budget` 与服务限流排队）。
- `call` 的 `id` 在一次 `run` 内唯一；`reply` 可乱序到达。
- `failed.error.class`：`invalid_input`（输入不合法，重跑无用）、`resource`（内存 / 磁盘 / 超时，宿主可换进程重试一次）、`internal`（程序缺陷）。
- 宿主会在用户修改后重跑同一 `run`；已完成的服务调用立即从缓存回复。
- `cancel` 后程序应尽快以 `failed`（class `cancelled`）结束；超时未结束则宿主终止进程。

## 8. 时间轴条目 `todo-items/1`

```jsonc
{
  "key": "s3/e2/draw",                 // 稳定标识：同一逻辑产物在重跑间不变
  "rev": "9f2c…",                       // 内容版本：输入或结果变化时改变
  "role": "visual",                     // voice | music | sfx | visual | text | subtitle
  "lane": 2,                            // 同 role 内的层序，0 在最下
  "start": 12.35, "duration": 4.2,      // 秒，相对成片起点
  "source": { "type": "file", "file": "out/s3_e2.reveal.png", "in": 0 },
  "transform": { "x": 0.1, "y": -0.05, "scale": 0.6 },
  "intro": { "kind": "fade", "duration": 0.4 },
  "link": "s3/voice",                   // 跟随该条目移动（配音挪了画面跟着走）
  "gain": 1.0,                          // 音频，0..4
  "fade": { "in": 0.15, "out": 0.25 }  // 音频：淡入淡出秒数；画面 / 文字：只用 out（结尾淡出，用于换板交叉过渡），in 必须为 0
}
```

`intro.kind` 取 `fade | rise | pop | slideLeft | slideRight | drop | stamp | grow`；宿主按 core 的同一条曲线计算，片段超过入场时长后保持恒等变换。

`source.type`：`file`（媒体文件：静帧、视频、packed-alpha、reveal 图集、音频）或 `text`（花字 `TextContent`）。宿主校验文件存在且在工作目录内、时间有限且非负、key 唯一。入轨规则：同 key 新 rev 原位替换并保留用户改过的变换；用户删掉的 key 在 rev 不变时不再放回；带 `link` 的条目随被链接条目平移。

## 9. 图层种类

| 图层 | 文件 | 说明 |
| --- | --- | --- |
| 静帧 | png/jpg/webp（可透明） | 配合 `intro` 动效 |
| reveal | `.reveal.png` 图集 + tEXt 元数据 | 书写 / 显现回放，原生着色器逐帧精确，拖动零解码 |
| packed-alpha | mp4（颜色与 alpha 拼接，流标签 `todo_alpha`） | 需要逐帧透明动画时使用（转场、粒子） |
| 视频 | mp4 | 不透明视频 |
| 文字 | `TextContent` | 花字，可编辑 |
| 音频 | wav / mp3 | 配音、音乐、音效，各自独立轨 |

## 10. 自检与交付

`tests/` 放至少一个样例：`input.json`（文案与参数）、`services/`（录制的服务结果，按请求指纹命名）、`expect.json`（期望条目的 key 与 role）。宿主自检命令离线回放服务结果运行流程，校验：清单与参数合法、协议消息合法、条目与文件合法、在预算内完成、两次运行产出一致（确定性）。不通过的扩展不允许安装为可用。
