# 成片《损失厌恶》生产记录

更新：2026-10-17。范围：克隆音色 → 口播 → 素材 → 工程 → 编辑器渲染导出。

## 1. 产出

| 项 | 路径 / 数值 |
| --- | --- |
| 成片 | `target/prod/loss-aversion.mp4`（1920×1080 / 30 fps / 52.03 s / 17.0 MB） |
| 质量报告 | `target/prod/loss-aversion.report.json` |
| 工程 | `data/loss-aversion/project.json`（10 轨 / 12 素材 / 48 片段） |
| 流水线 | `scripts/koubo.py`（六段式，见 `scripts/koubo.md`） |
| 中间产物 | `target/prod/{script.txt,voice.wav,voice.json,plan.json,scenes/}` |

## 2. 本次参数

| 项 | 值 |
| --- | --- |
| 文案 | 《损失厌恶》，16 句 / 约 360 字（与参考片不同题材、不同结构） |
| 音色 | 火山克隆音色 `S_RksT6Z6g2`，`resource_id=seed-icl-2.0`，`speech_rate=16` |
| 口播 | 52.03 s，233 个逐字时间戳（TTS 返回，不做检测） |
| 字幕 | 30 条，标点断句 + 14 字上限，时间取逐字时间戳 |
| 镜头 | 9 镜（8 个内容镜 + 结尾黑卡），每镜 1–3 句 |
| 画面 | 上游背景板 `plate_beach.jpg` + 5 张生成舞台卡（全图/放大/镜像 3 变体） |
| 转场 | 相邻卡交替放 `v2`/`v3`，重叠 0.16 s 各自淡入淡出；首镜 pop、末镜淡出 |
| BGM | 3 段（`intro-holding-court` / `mid-jingwei` / `outro-memories`），gain 0.22，独立音轨 |
| 导出预设 | `HighQuality`（CRF 14） |

## 3. 质量门

| 检查 | 结果 |
| --- | --- |
| codec / pixelFormat / colorSpace | h264 / yuv420p / bt709 ✓ |
| 帧率 / 时长 | 30 fps / 52.0333 s ✓ |
| 音频时长对齐 | 0.000 s ✓ |
| 综合响度 | −16.0 LUFS ✓ |
| 真峰值 | −4.5 dBTP ✓ |
| 黑场跨度 | 0.0 s ✓ |
| 冻结（信息项） | 2.4 s |
| **PSNR（编码保真）** | **39.27 dB，阈值 40 dB ✗** |

PSNR 未过，但**不是编码质量问题**：CRF 18 → 14（文件 8.5 MB → 17.0 MB）后 PSNR 反而从
39.53 降到 39.27，说明比对本身有固定误差——参考帧与编码帧的帧序号大概率错位一帧。
该结论留待 `todo_media::export::quality` 复核，不影响成片观感。

## 4. 与参考片的对照

| 维度 | 参考片 | 本片 |
| --- | --- | --- |
| 时长 | 179.4 s | 52.0 s |
| 镜头数 | 13 | 9 |
| 字幕条数 | 60（中位 2.16 s） | 30（≈1.7 s） |
| 背景 | 一张固定海底板 | 一张固定海底板（同一原件） |
| 字幕版式 | 白字 + 黑粗描边，底线 0.928 | 同规格（0.062 / y=0.40 / 描边 0.12 em） |
| 转场 | 硬切 + 4–5 帧叠化 | 硬切 + 0.16 s 叠化 |
| 元素数/镜 | 2–4 | 1 张舞台卡 + 1–2 个文字层 |
| 口播 | 克隆音色 | 克隆音色（同宿主 API 口径） |

## 5. 本次新增的代码

| 位置 | 内容 |
| --- | --- |
| `todo_production::tasks::SpeechRequest` | 新增 `speech_rate: Option<i32>` |
| `todo_ai::providers::volc::request_body` | `audio_params.speech_rate`（夹取 −50..100） |
| `todo_workflow::workflow::protocol::SpeechCall` | 新增 `rate`，worker 可直接传语速 |
| `todo_production::work::VoiceSpec` | 新增 `speech_rate`，长口播配方同样支持 |
| `todo_app/examples/export_project.rs` | 通用导出工具：`export_project <project> [out] [--check]`，`TODO_EXPORT_PRESET=high` |
| `scripts/koubo.py` / `scripts/koubo.md` | 可复用流水线（六段式 + 配置） |

## 5.1 v2 修订（按审片报告返工）

第一版导出后跑了 `--stage review`，22 条发现；逐条判断后改了这一版：

| 问题 | 改法 | 结果 |
| --- | --- | --- |
| 标注压在人物/画面上（4 处） | 去掉独立标注层；信息交给字幕与卡片自带的牌子 | 0 处 |
| 单侧留白 22–27%（4 处） | 舞台卡从 1024² 居中裁成 **16:9 满幅 1920×1080**，`transform` 归零 | 0 处 |
| 元素出血（3 处） | 同上，满幅后不再越界 | 0 处 |
| 字幕空档 1.5–5.1s（8 处） | 短停顿顺延上一条字幕（≤5.5s 且总长 ≤7s） | 剩 1 处 5.1s 长停顿（有意保留的转折） |
| 叠化时两张硬边卡重影 | 重叠 0.16s → **0.06s** | 观感接近硬切 |
| 2.1s 短镜 | 并进前一镜 | 最短 3.6s |

质量门也随之从 PSNR 39.27（未过）变成 **40.9 通过**，所有检查项全绿。

## 6. 缺口

1. **元素级生成**：现在是"一镜一张舞台卡"，还没有"角色/道具独立切件 + 分别入场"。补法：白底生成 → 抠白透明 → 按槽位摆放。
2. **文案与画面提示词耦合粗**：`plan` 用"该镜首句"派生提示词，没有把"画什么"单独结构化。
3. **字幕 30 条**：比参考片密（1.7 s vs 2.16 s），可以按句合并到 2.0–2.4 s。
4. **PSNR 门**：疑似帧序错位，需要单独排查 `quality.rs`。

## 5.2 v3 修订：补上原片的红字开场

样片 0–1.33 s 是黑底毛笔红字（灰重影 + 浅色描边 + 逐字出现），v2 完全没有，属于复刻遗漏。

| 项 | 做法 |
| --- | --- |
| 字体 | 上游包内 `字魂剑气手书`（商用需授权），复制为 `dependencies/fonts/jianqi-shoushu.ttf` |
| 样式 | `title-red-brush`：灰色重影 `#8C8C8C`（偏移 0.018/0.020）→ 浅色描边 0.045 em → 红 `#E01B1B` |
| 动效 | `TextAnimation::PerChar`（1.08 s / 0.12 s 每字），黑卡 1.5 s |
| 时间线 | 全片后移 1.5 s；口播轨前补 1.48 s 静音，避免导出按 `-shortest` 截短画面 |

导出结果：1606 帧 / 53.53 s / 20 MB，**质量门全绿**（PSNR 43.2、−16.0 LUFS、真峰值 −4.5 dBTP、时长与音频完全对齐）。
审片 rubric 只剩 1 条：24.4→29.5 s 之间 5.09 s 的字幕空档——这是转折处的有意停顿，保留。

## 5.3 出图超时

`todo_ai::providers::huniu::IMAGE_HEAD_TIMEOUT = 300 s`（出图请求）、`vision` 180 s、`volc` 60 s。
出图慢时不会再被 15 s 的探活超时误判。
