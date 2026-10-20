# Agent 导演：全自动成片（海绵宝宝口播为样板）

日期：2026-10-18。来源：`docs/一些新的想法.txt`。对标片：仓库根目录 `什么是诱饵效应.mp4`（解剖见 `docs/reports/2026-10-17-spongebob-reference-anatomy.md`）。

## 目标

用户在内置 Agent 里说一句话，例如“用海绵宝宝风格做 30 秒，讲沉没成本”。之后的流程全部自动完成：写文案、配音、分镜、素材、剪辑、转场、审片、修正。用户只做最后的审批。

第一版先做 30 秒成片，但整套结构要完整，换成 3 分钟时只需改配置。

## 分层：每层只有一个入口

| 层 | 入口 | 职责 |
| --- | --- | --- |
| 能力 | `todo_ai::AiRouter` | 文本、图片、视频、多模态、音频；供应商、重试、计费日志都在这一层以下 |
| 动效 | `todo_core::motion` + `todo_motion` | 入场、出场、转场预设（数据 + 求值），本地库读写 |
| 编辑 | `todo_application::control::ControlEdit` | 时间轴的唯一修改入口，带校验、撤销和预览刷新 |
| 风格 | 扩展（`extensions/builtin/<id>/`） | 配方：版式、素材库、字幕样式、转场规则、节奏范围、审片 rubric |
| 导演 | 内置 Agent（`todo_app::runtime::agent`） | 多轮工具循环：选风格 → 跑配方 → 审片 → 修正 → 交给用户审批 |

`scripts/koubo.py` 里验证过的规则（句群定时、逐字字幕、审片 rubric）迁进海绵宝宝扩展，之后删除这个脚本。

## 1. 动效模块

- 预设：`MotionPreset { id, name, kind: intro|outro|transition, duration, target, outgoing? }`。通道是 opacity、x、y、scale、scaleX、scaleY 的关键帧曲线 `[{t, v, ease}]`。opacity 和 scale 按乘法叠加，x、y 按加法叠加（单位是画布比例）。
- 校验规则：入场在 `u=1`、出场在 `u=0`、转场的入画方在 `u=1`、出画方在 `u=0`，都必须回到恒等变换，保证片段不会跳变。
- 内置预设是 `crates/todo_core/src/model/motions.json`，编译期嵌入，现有的 8 种入场原样迁移进来。用户库放在 `motions/*.json`，扩展库放在 `extensions/<id>/motions/*.json`，由 `todo_motion::MotionLibrary` 读写。
- 工程首次用到非内置预设时，把它复制进 `Project.motions`。解析顺序：先查工程，再查内置。

## 2. 同轨转场

- `Clip.transition: Option<TransitionRef { kind, duration }>`，表示从同轨紧邻的前一个片段切到本片段。只有两段首尾相接时才生效。
- 转场窗口以切点为中心，范围是 `[cut − d/2, cut + d/2]`，并要求 `d ≤ min(前段时长, 本段时长)`。出画方在结束后继续显示，入画方提前出现。视频源超出范围时取边界帧，图片和文字不受影响。
- 求值放在 `todo_core::query`：一条轨在转场窗口内会产出两个 hit。预览、导出和 `describe` 共用同一个函数，所以三者不会不一致。
- 入场和出场的时钟按“可见区间”计算，也就是把转场的提前和延后部分算进去。
- 内置转场：`cut`（不加转场）、`dissolve`（出画方保持不透明，入画方淡入，适合满幅画面）、`crossfade`（双方都淡，适合透明抠像）、`dip`（先淡出再淡入）、`pushLeft`、`pushRight`、`slideLeft`、`slideRight`、`zoomIn`。
- 音频轨暂不支持转场。

## 3. 编辑协议补齐

新增 `ControlEdit`：`set_intro`、`set_outro`、`set_transition`、`set_audio`。`describe` 输出转场状态。界面上在时间轴切点处显示转场标记，属性面板可以选择转场。

## 4. 多模态

在 `huniu` 中实现 `MultimodalRequest`，走 OpenAI 兼容的 `image_url`（data URI），用来审片和挑选候选图。

## 5. 海绵宝宝配方

- 素材库：`extensions/builtin/spongebob-koubo/assets/library/`，包括背景板、舞台、角色和道具的透明 PNG，以及 `library.json`（id、标签、描述、默认槽位）。素材一次性生成，挑选后入库。
- 配方文件 `recipe.json`，包含：
  - 版式模板：single、duo、triptych、endcard；
  - 每个槽位的矩形；
  - 字幕、牌面字、标题的样式；
  - 转场矩阵：同场景用 cut，换场景用 dissolve，时长 0.15 s；
  - 节奏范围：镜头 3–12 s，字幕不超过 14 字，每镜 2–5 个元素；
  - 审片 rubric。
- worker 的流程是：文案 → 配音（逐字时间）→ 由 LLM 按 JSON schema 产出镜头表（不合法就打回重写）→ 按镜头表选模板、选素材、写牌面字（只能从库里选，缺的才生图）→ 确定性编译成条目（包含 transition 字段）→ 渐进入轨。

## 6. Agent 导演循环

工具：`list_styles`、`start_production`、`production_status`、`get_context`、`describe`、`render_preview`、`review_frames`（多模态）、`apply_edits`、`undo`、`finish`（提交审批）。

限制：最多 N 步，修正最多 3 轮；只修改有问题的镜头；整次运行可以一键撤销。

审片分两层：先跑规则检查（越界、遮挡、字幕空档、节奏），再做多模态检查（切点抽帧、拼接图）。最后由用户审批，可以对单个镜头点“重做这一镜”。

## 7. 验收

- 30 秒成片：镜头数等于句群数，字幕首字对齐误差不超过 0.12 s，背景一致，响度 −16 ± 2 LUFS，审片零阻塞问题。
- 和参考片对标：字幕版式（字号 0.062、底线 0.928）、转场类型、每镜元素数都在 rubric 范围内。
- 全程没有人工干预，最后弹出审批卡。
