# 动效模块与同轨转场

更新：2026-10-18。代码：`crates/todo_motion`（格式、求值、本地库）、`todo_core::model::effects` 与 `todo_core::query::resolve_preview`（片段引用与图层求值）。

## 1. 一种格式，三类动效

入场（intro）、出场（outro）、转场（transition）用同一种「动效预设」描述，以 JSON 存在本地，可以复用和分享：

```jsonc
{
  "id": "crossfade",            // [A-Za-z][A-Za-z0-9_.-]{0,63}，全库唯一
  "name": "交叉淡化",
  "kind": "transition",         // intro | outro | transition
  "duration": 0.3,              // 建议时长（秒），片段上保存自己的时长
  "tags": ["基础"],
  "channels": {                 // 被动画的片段；转场里是入画方
    "opacity": [{ "t": 0, "v": 0 }, { "t": 1, "v": 1, "ease": "smooth" }]
  },
  "outgoing": {                 // 仅转场：出画方（空对象 = 保持不变）
    "opacity": [{ "t": 0, "v": 1 }, { "t": 1, "v": 0, "ease": "smooth" }]
  },
  "outgoingOnTop": false        // 仅转场：出画方画在上面
}
```

- 通道包括 `opacity`、`x`、`y`、`scale`、`scaleX`、`scaleY`。不透明度和缩放按乘法叠加到片段自身的变换上，`x`、`y` 按加法叠加（单位是画布宽 / 高的比例，和变换一致）。
- 关键帧 `t` 取 0..1，表示在动效窗口里的位置。`ease` 作用在以该关键帧结尾的那一段上，可选 `linear`、`in`、`out`、`inOut`（三次曲线）、`smooth`、`backOut`、`bounceOut`、`sineInOut`、`hold`。第一个关键帧之前和最后一个之后取值保持不变。
- 校验规则：入场在 `t=1`、出场在 `t=0`、转场入画方在 `t=1`、出画方在 `t=0`，都必须回到恒等变换，保证片段不会跳变。不合规的文件会被跳过，并在启动时提示。

## 2. 本地库

| 来源 | 位置 | 说明 |
| --- | --- | --- |
| 内置 | `crates/todo_motion/src/builtin.json` | 编译期嵌入，一定可用。原来的 8 种入场曲线已原样迁移（有测试逐点对比） |
| 用户 | `<仓库>/motions/*.json` | `MotionLibrary::save_user` 写入，可以删除 |
| 扩展 | `extensions/<id>/motions/*.json` | 随风格包分发，只读 |

优先级：内置 > 用户 > 扩展。id 重复时后加载的被跳过，并给出警告。

工程第一次用到非内置预设时，会把它复制进 `Project.motions`，和花字样式的做法一样。这样工程换一台机器、或者库里的预设改了，都能原样渲染。复制和使用在同一个撤销步里完成，包括控制协议、属性面板（`EditorCommand::EditAll`）和成片入轨（`GeneratedEdit::AddMotions`）。

## 3. 片段上的引用

```jsonc
"intro":      { "kind": "pop",      "duration": 0.3 },
"outro":      { "kind": "fadeOut",  "duration": 0.25 },   // 旧工程的数字写法读作 fadeOut
"transition": { "kind": "dissolve", "duration": 0.15 }    // 从同轨紧挨着的前一个片段切进来
```

## 4. 同轨转场

- 转场存放在后一个片段上。只有前一个片段的结束时间和它的开始时间相差不超过 `1e-4 s` 时才生效。片段被挪开后转场保留、但不再播放，界面上会标成"未生效"。
- 转场窗口以切点为中心：`[cut − d/2, cut + d/2]`，有效时长取 `min(d, 前段时长, 本段时长)`，因此同一个片段两端的转场窗口不会重叠。
- 窗口内这条轨会出两个图层：出画方在结束后继续显示，入画方提前出现。视频源超出范围时停在边界帧，图片和文字不受影响。
- 入场和出场的时钟按"可见区间"计算（把转场的提前和延后部分算进去），所以入画方一出现入场动画就开始。
- 剃刀切分时，入场和转场留在左段，出场移到右段，切点本身是直切。
- 求值只有一处：`todo_core::hit_transform`。预览（`picture_layers`）、导出（`export_layers`）、控制协议的 `describe` 和审片规则都调用它，不会出现不一致。
- 当前不支持音频轨转场，编辑时会被拒绝。

内置转场：`dissolve`（出画方保持不透明，适合满幅画面）、`crossfade`（双方都淡，适合透明抠像叠在同一背景上）、`dip`、`pushLeft`、`pushRight`、`coverLeft`、`coverRight`、`zoomIn`。

## 5. 编辑入口

| 入口 | 写法 |
| --- | --- |
| 核心 | `EditCommand::{SetIntro, SetOutro, SetTransition, SetAudio, AddMotions}` |
| 控制协议 / Agent | `set_intro`、`set_outro`、`set_transition`、`set_audio`（`ControlEdit`）。名称来自动效库时会自动复制进工程 |
| 扩展条目 | `Item.intro`、`Item.outro`、`Item.transition` |
| 界面 | 左侧栏「动效」图鉴（循环预览，一点即用）；属性面板的「动效」区；时间轴切点上的领结标记表示转场窗口 |
