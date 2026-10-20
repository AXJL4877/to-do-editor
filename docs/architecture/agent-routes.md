# Agent 可执行路线（以根目录样片为标准）

更新：2026-10-17。样本：`<repo>/样片/` 下的口播成片（13 镜 / 179s / 60 条字幕）。
本文只写 **agent 能直接执行的东西**：镜头配方、动效 id、字幕规则、验收阈值、调用序列。

## 0. 模型与链路（已切换）

| 项 | 值 |
| --- | --- |
| 语言模型 | `deepseek-flash`（= deepseek v4.1 flash 线） |
| 端点 | `https://api.deepseek.com`（`config/ai.json` 的 `language.baseUrl`） |
| 密钥 | Windows 凭据 `todo-editor/deepseek.api_key`；环境变量 `TODO_DEEPSEEK_API_KEY` 优先 |
| 实测 | 一次 JSON 生成 **1.6 s**；一次审图（1 张节目帧）**3.4 s** |
| 多模态 | 同一端点同一 key：`deepseek-flash` 能读图（`deepseek-v4-pro` 不支持图片） |
| 不变 | 出图/生视频仍走 huniu（`gpt-image-2` / `sora-2`），配音走火山 |

图片和语言是两条独立链路：改语言端点不会影响出图；但**“看图审片”和“写计划”现在共用 DeepSeek 这一条**。

## 1. 样片的结构（可直接照抄的数值）

| 量 | 值 |
| --- | --- |
| 镜头数 / 均值 | 13 / 13.8 s（4.4–36.6 s） |
| 字幕 | 60 条，中位 2.16 s，单行 ≤14 字，白字黑描边 |
| 开场 | 黑底毛笔红字 1.5 s，逐字出现 |
| 结尾 | 黑底标题卡 ≥5 s |
| 转场 | 硬切 + 0.15 s 叠化（`dissolve`） |
| 每镜元素 | 2–4 个：底板 + 1–2 角色/道具 + 0–2 文字层 |

## 2. 四条路线（agent 选一条执行）

### 路线 A：样板复刻（默认，命中率最高）

**输入**：一段文案（≥300 字）+ 样片风格。
**镜头配方**：按句群切镜，每镜 1–3 句、≥3.5 s；依次套用
`底板 → 整幅舞台卡（intro=pop/rise）→ 字幕 → 可选大字标注`。
**动效映射**：

| 事件 | motion id | 参数 |
| --- | --- | --- |
| 换镜 | `dissolve` | 0.15 s，放新镜 `transition` |
| 卡片入场 | `pop`（卡通）/ `rise`（讲解） | 0.3–0.4 s，`intro` |
| 卡片退场 | `fadeOut` | 0.3 s，`outro` |
| 结尾卡 | `fade` | 0.4 s，`intro` |

**执行序列**（`ApplyEdits` 批量，一次一个撤销步）：

```jsonc
[{ "type":"placeMedia", "media":"plate", "track":"v1", "start":0, "duration":D },
 { "type":"placeMedia", "media":"card_1", "track":"v2", "start":0.0, "duration":3.6 },
 { "type":"setIntro", "clip":"card_1", "motion":{"id":"pop","duration":0.35} },
 { "type":"setTransition", "clip":"card_2", "motion":{"id":"dissolve","duration":0.15} },
 { "type":"addText", "track":"t1", "start":1.5, "content":{"text":"…","styleId":"sub-white"} }]
```

**验收**：`review` 无 `Block`；字幕条数 ≈ 句数×1.2；最长镜头 ≤ 30 s。

---

### 路线 B：口播讲解（信息型）

**输入**：文案 + 每句一个“关键词”。
**镜头配方**：一句一镜或两句一镜；每镜 = 底板 + 关键词大字（`stamp` 入场）+ 小卡。
**动效**：关键词 `stamp`（0.35 s），卡片 `rise`，换镜 `dissolve`。
**字幕**：与关键词同步出现，避免同时出现两层大字 → 关键词那条不叠字幕。
**验收**：同屏文字层 ≤ 2；安全区外无文字。

---

### 路线 C：对比 / 数据讲解

**输入**：文案里出现“A 与 B”“贵/便宜”“前/后”。
**镜头配方**：左右分栏卡 + `pushLeft` 推入第二栏；数字用 `grow` 放大。
**动效**：`pushLeft` 0.4 s 做对比揭示，`grow` 0.4 s 做数字强调，`dissolve` 换镜。
**验收**：两栏矩形不重叠（`review` 的 `max_overlap` 检查）；同屏元素 ≤ 5。

---

### 路线 D：快剪卡点（节奏型）

**输入**：文案 + BGM（有节拍）。
**镜头配方**：每 1.5–2.5 s 一镜，切点落在拍点上；每镜 1 张卡 + 1 条字幕。
**动效**：`slideLeft/slideRight` 交替，避免同一方向连续超过 3 次；换镜用硬切（不设 `transition`）。
**验收**：切点间隔方差 < 0.15 s²；无镜头 < 1.2 s。

## 3. agent 的执行循环（Director 阶段对应）

| 阶段 | 输入 | 输出 | 失败重试 |
| --- | --- | --- | --- |
| understand | 文案 + 路线 id | 句群/镜头数 | 1 次，附失败原因 |
| plan | `Describe` 的 `ProjectView` | `ApplyEdits` 批次 | 2 次 |
| review | `Describe` + rubric | `Check[]` | — |
| fix | `Check[]`（只处理 `Block`） | 小幅 `ApplyEdits` | 2 次 |
| finish | rubric 无 `Block` | 结束 | — |

**硬约束（写进 system prompt）**：

1. 只准用本文出现的 motion id；不存在 `zoomInOut` 之类的自造 id。
2. 一次 `ApplyEdits` 只做一件事（一个镜头或一组字幕），便于回滚。
3. 不改 `plate`（底板）以外图层的 z 序。
4. 字幕文本必须来自配音逐字时间戳切分，不许自己估时间。
5. 修复只允许动 `transform` / `motion` / `start` / `duration`，不许删用户素材。

## 4. 速度：现在慢在哪、怎么改

| 环节 | 现状 | 措施 |
| --- | --- | --- |
| 文案/分镜（chat） | 6–10 s/次，串行 | **已切 deepseek-flash，1.6 s/次** |
| 出图 | 40–100 s/张，串行 | 并发 2–3 + 失败重试；同 prompt 命中缓存；尺寸降 768 再放大 |
| 导出 | 1080p 全片约 5 min | 预览用 480p/半帧率；只重导改动区间 |
| reviewer 往返 | 每轮一次全片 | 只查改动区间 ±2 s |

目标：一条 60 s 片子 **≤ 6 分钟**（当前约 18 分钟）。

## 5. 现在就能验证的三件事

```powershell
cargo run -p todo_ai --example ai_check                 # 端点与密钥来源
cargo run -p todo_app --example describe_project -- <project> 12   # 元素矩形
cargo run -p todo_app --example render_frame -- <project> <dir> 3  | # 单帧审图
```
