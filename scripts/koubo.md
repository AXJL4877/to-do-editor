# 口播成片流水线 `scripts/koubo.py`

一条命令把「一句文案」做成「一个可编辑工程 + 一支 MP4」，成片由**编辑器自己的渲染器**导出。

## 六段式

```text
script  → tts  → plan  → scenes  → project  → export
文案      配音    镜头表   画面素材    工程装配    渲染导出
```

| 阶段 | 做什么 | 产物 | 花钱 |
| --- | --- | --- | --- |
| `script` | chat 模型按 `topic` 写 45–60 s 口播稿（已有 `script.txt` 则保留，可手改） | `target/prod/script.txt` | 是 |
| `tts` | 火山克隆音色合成，返回音频 + **逐字时间戳** | `voice.wav`、`voice.json` | 是 |
| `plan` | 句子 → 镜头（每镜 1–3 句、≥5 s），写画面提示词 | `plan.json`（可手改） | 否 |
| `scenes` | 按计划逐镜生成画面，缺图跳过 | `scenes/sN.img` | 是 |
| `project` | 背景板 + 舞台卡（全图/放大/镜像变体）+ 字幕 + 标注 + 三段 BGM，组装 `project.json` | `data/<project>/` | 否 |
| `describe` | 打印每个元素的像素矩形（z 序、时间、位置、尺寸、警告），给人和 agent 核版式 | stdout | 否 |
| `inspect` | 用编辑器离线渲染器出节目帧，降采样成 jpg，供大模型**自己看画面** | `target/prod/review/*.jpg` | 否 |
| `review` | 出**小 mp4**（480p）+ 每个切点前后 5 帧拼图 + 版式/时序 rubric（只报告，不改） | `review/preview-480p.mp4`、`cuts-sheet.jpg`、`report.md/json` | 否 |
| `export` | 调 `todo_app` 的导出适配器：原生渲染器出帧 → FFmpeg 编码 → 混音 → 质量报告 | `target/prod/<project>.mp4` + `.report.json` | 否 |

单独跑某段：`python scripts/koubo.py --stage project`。

## 配置

`scripts/koubo.json`（可选，覆盖默认值）：

```jsonc
{
  "project": "loss-aversion",
  "title": "损失厌恶",
  "topic": "……",
  "style": "海绵宝宝动画风格，扁平卡通，粗黑描边……",
  "voice": { "speaker": "S_RksT6Z6g2", "resource_id": "seed-icl-2.0", "speech_rate": 16 },
  "text": { "sub_size": 0.062, "sub_y": 0.40, "title_size": 0.13 },
  "bgm": [ { "file": "…/intro-holding-court.mp3", "start": 0, "end": 20, "gain": 0.22 } ],
  "labels":  [ { "shot": 0, "text": "限时 30 分钟" } ],
  "numerals": [ { "shot": 2, "text": "30:00", "size": 0.20 } ],
  "transition": 0.16
}
```

密钥走环境变量：`TODO_HUNIU_API_KEY`、`TODO_VOLC_API_KEY`（桌面端存在 Windows 凭据管理器里，
可从 `todo-editor/volc.api_key`、`todo-editor/huniu.api_key` 迁移）。

## 关键设计

1. **字幕不吃检测**：直接由 TTS 的逐字时间戳切句（标点断句 + 14 字上限），所以字字对得上。
2. **转场是两轨叠化**：相邻卡片交替放在 `v2`/`v3`，重叠 `transition` 秒并各自淡入淡出；
   镜头内元素入场用 `intro` 的 pop/rise。硬切用 0 重叠即可。
3. **背景板全片一张**：`plate_beach.jpg`（上游包内原件），舞台卡浮在上面，观感统一。
4. **同轨不重叠**：装配末尾统一 `trim()`，把每条压到下一条之前，避免工程校验报 `overlap`。
5. **成片 = 工程**：导出走 `crates/todo_app/examples/export_project.rs`，与「导出」按钮同一套
   渲染器、混音与质量门（码率/色彩/响度/PSNR/黑场/静帧）。

## 已知缺口

- 元素级素材（角色/道具独立切件）还没做：现在是「一镜一张舞台卡」。补法：白底生成 → 抠白成
  透明 PNG → 按槽位分别入场。
- `scenes` 串行：图片网关并发会断 TLS。
- 文案与画面提示词的耦合还比较粗（提示词 = 该镜首句）。
