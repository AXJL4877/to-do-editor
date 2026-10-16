# 花字层标准

更新：2026-10-16。本文件规定花字（样式化文字）在工程中的数据结构、单位、动画和渲染链路。

## 数据结构

文字是片段的一种来源。`Clip` 只保存所有片段共有的时间与变换字段，来源由 `ClipSource` 区分：

```rust
pub struct Clip { id, start, duration, link_id, transform, source: ClipSource }
pub enum ClipSource { Media(MediaSource), Text(TextContent) }
pub struct MediaSource { media_id, offset, rate }
```

工程 JSON 保持扁平：媒体片段仍是旧的 `mediaId/offset/rate`，旧工程无需迁移；文字片段带 `text` 对象，不能出现 `offset/rate`。同时出现 `mediaId` 和 `text` 的片段在读取时被拒绝。

花字分为两部分，二者都放在 `crates/todo_core/src/model/text.rs`：

| 结构 | 保存位置 | 内容 |
| --- | --- | --- |
| `TextContent` | 每个文字片段 | 文字、`styleId`、排版 `layout`、动画 `animation` |
| `TextStyle` | `Project.textStyles` | 字体 `font` 和自下而上的图层 `layers` |

内置样式由 `builtin_text_styles()` 提供。片段第一次使用某个内置样式时，会把该样式复制进工程。之后预设变化不影响已有工程，换机器打开效果也一致。

### 单位约定

- `layout.fontSize`：字高占画布高度的比例，与 `Transform` 的画布比例坐标一致。
- 样式内的 `offset`、`blur`、描边 `width`、`erode.scale` 一律以 em（字号）为单位，同一花字套在任何字号上都保持比例。
- 颜色写成 `#RRGGBB` 或 `#RRGGBBAA`，非预乘。
- 随机效果（`erode`）必须带 `seed`，预览与以后的导出逐像素一致。

### 图层

花字 = 字形轮廓上的若干遍绘制，从下往上合成。效果不单独开字段：

| 效果 | 写法 |
| --- | --- |
| 填充 | `{"kind":"fill","paint":"#E00A0A"}` |
| 描边 / 外框 | `{"kind":"stroke","width":0.12,"paint":"#000000"}` |
| 投影 | 偏移 + 模糊的填充：`{"kind":"fill","offset":[0.035,0.05],"blur":0.025}` |
| 发光 | 模糊的描边 |
| 重影 / 回声字 | 放大的填充：`{"kind":"fill","scale":1.33}`，以文字块中心缩放 |
| 渐变 | `"paint":{"from":"#FFFFFF","to":"#FFE08A","angle":0}`，0° 自上而下 |
| 做旧 / 斑驳 | `"erode":{"amount":0.18,"scale":0.035,"seed":7}` |

参考效果 `s00_intro1_hold.png` 对应内置样式 `grunge-red-echo`：灰色 1.33 倍做旧重影，加偏移软投影，再加红色毛笔字。

```json
{
  "id": "grunge-red-echo", "name": "做旧红字·灰重影",
  "font": { "family": "字魂剑气手书", "file": "zihun-jianqi-shoushu.ttf",
            "fallbacks": ["msyhbd.ttc", "msyh.ttc"] },
  "layers": [
    { "kind": "fill", "paint": "#474747FF", "scale": 1.33, "opacity": 0.92,
      "erode": { "amount": 0.18, "scale": 0.035, "seed": 7 } },
    { "kind": "fill", "paint": "#000000FF", "offset": [0.035, 0.05], "blur": 0.025, "opacity": 0.8 },
    { "kind": "fill", "paint": "#E00A0AFF", "erode": { "amount": 0.08, "scale": 0.02, "seed": 3 } }
  ]
}
```

文字片段示例：

```json
{ "id": "c_1", "start": 0, "duration": 3,
  "text": { "text": "钱越来越 不值钱了", "styleId": "grunge-red-echo",
            "layout": { "fontSize": 0.155, "letterSpacing": 0, "lineHeight": 1.2, "align": "center" },
            "animation": { "intro": { "kind": "perChar", "duration": 1.2, "charDuration": 0.25 } } } }
```

### 动画（第一版）

| `intro.kind` | 效果 | 实现 |
| --- | --- | --- |
| `fade` | 整体在 `duration` 秒内从透明到不透明 | 只改变合成不透明度，不重新栅格化 |
| `perChar` | 逐字出现，每个字用 `charDuration` 秒渐显，最后一个字在 `duration` 秒时完全出现 | 每个输出帧一张栅格，按帧率量化 |

空白字符不参与逐字计数。剃刀切开带入场动画的文字时，只有左段保留入场；右段内容相同但没有入场，避免剪开后重新播放。

## 轨道和编辑规则

- 新增 `TrackKind::Text`，排在所有视频轨上方，即遮盖视频。文字片段只能放在文字轨；文字轨不接受媒体。
- `EditCommand::AddText` 放到指定文字轨，或第一条空闲的文字轨，必要时新建；`EditCommand::SetText` 整体替换内容，保留时间和变换。
- 裁切不受源素材边界限制；文字没有速率，Alt 变速会被拒绝（`UnsupportedClip`）。
- 轨道分组、排序和可互移判断统一由 `TrackKind::section_rank` / `same_family` / `is_visual` 给出，不在各处另写。

## 渲染链路

```text
core      resolve_layers → HitSource::Text { style, frame }    （纯数据，含动画采样）
app层     TextRasters：按摘要判断栅格是否就绪，缺失则发 MediaTask::TextRaster   （无 I/O）
media     TextRasterizer：todo_text 渲染，PNG 缓存在 cache/media/text/       （文字专用线程）
todo_text FontLibrary + 排版 + 图层合成 → 画布大小的透明 RGBA，附带文字块范围
native    透明 PNG 保留预乘 BGRA，与视频统一按 Transform 合成
```

- 栅格为整张画布大小，文字块居中。片段 `Transform` 对它的作用与全屏画面完全一致，整体淡入只乘到图层不透明度上。
- 栅格摘要 `text_raster_digest` 由文字、排版、样式、画布和逐字进度共同决定（FNV-1a）。动画参数本身不进入摘要，同一文字的不同入场方式共用静止栅格。
- 栅格未就绪时预览保持上一组完整画面，就绪后再整组替换。渲染失败只去掉这一层并提示，不阻塞视频。
- 文字线程按队列处理：同一文字后面已有更新的帧时，旧帧标为跳过，拖动播放头时只渲染最新帧。
- 预览的视频图层与文字图层分别计入预算：视频最多 4 层（解码），文字最多 4 层。

## 选择与变换

每个元素有一个“基准尺寸”`BaseSize`，即单位变换下、居中于画布时的大小。媒体为等比适配尺寸；文字为渲染时测得的文字块。命中、选择框、对齐吸附和缩放手柄都基于基准尺寸，所以文字的选择框贴合文字本身，点击文字以外的位置仍能选中下层视频。文字块要等第一张栅格返回后才知道；在此之前，预览上暂时不能选中该文字（时间轴可以）。

## 字体

样式只写字体文件名，不写路径。查找顺序为 `TODO_FONTS_DIR`（可用 `;` 分隔多个目录）→ `dependencies/fonts/` → `%WINDIR%\Fonts`。带路径的文件名会被拒绝。主字体缺字时按 `fallbacks` 逐个回退。字体文件不入库，打包脚本会把 `dependencies/fonts/` 一并复制。见 [字体目录说明](../../dependencies/fonts/README.md)。

## 新增花字样式

1. 在 `builtin_text_styles()` 里追加一个 `TextStyle`，或者直接写进某个工程的 `textStyles`。
2. 效果尽量用现有图层组合表达；确实需要新能力时，扩展 `TextLayerKind` / `TextPaint`，同时补 `text_style_valid` 校验和 `todo_text::raster` 的绘制。
3. 渲染结果变化时提升 `todo_media::text::RENDER_VERSION`，使旧缓存失效。

## 尚未实现

出场与循环动画、逐字以外的逐字特效（弹跳、打字机光标）、局部改色（文字片段内多段样式）、纹理贴图填充、导出合成。
