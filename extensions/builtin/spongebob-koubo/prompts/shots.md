你在给“海绵宝宝口播”科普短视频配画面。镜头已经按台词切好：**一句话一个镜头**，你不能合并、拆分或改动台词，只为每个镜头填画面。

## 画面规则（固定配方）

- 开头：黑底红字标题（程序自动做，你只给 `title`）。
- `story` 讲故事：固定的海底草地背景板上放一张主卡片（一组人物和物件画在一起的透明插画），可以再加 0–2 个**附加元素**，在台词说到相应的词时依次弹出。
- `sublime` 升华：黑底上一张虚化的相关画面作背景，屏幕中间压着这句台词的红色毛笔字。只给能脱离故事单独成立的道理、提醒、金句；**台词里出现角色名字、在讲谁做了什么说了什么的句子不能当升华**（程序会把它改回讲故事）。**升华镜头总时长不能超过全片的 {{sublime_share}}%**，大约最多 {{sublime_max}} 秒；可以放在片中，也可以放在结尾。

## 镜头（序号、时长、台词）

{{shots}}

## 可用素材库（附加元素可以直接用这些 id）

角色：
{{cast}}

道具和符号（标了“悬浮”的是符号，飘在半空）：
{{props}}

## 主卡片入场动效（`motion`，只在这句台词有特殊画面时才填，其余留 null，程序会随机挑）

{{card_motions}}

## 附加元素动效（元素的 `motion`，同样只在需要特殊效果时填，否则 null）

{{element_motions}}

## 要求

1. **每个镜头都写 `draw`**（英文，具体写这张主卡片画什么：谁在做什么、有哪些物件、怎么摆），只画一组紧凑的主体，不要背景和地面。**升华镜头的 `draw` 写一个最能代表这句话的单独物件或小场景**（它会被虚化成背景，不要文字），万一它被改回讲故事镜头也能直接用。
2. 主卡片里需要出现的中文写进 `texts`（牌子、气泡、标注；每条 ≤8 字，最多 3 条）。人物是上面角色列表里的，把 id 写进 `cast`（最多 2 个，用来保持长相一致）。
3. **附加元素** `elements`（每镜 0–2 个，**只给大约三分之一的讲故事镜头**，宁少勿多：观众要来得及看。只有台词明确出现第二样东西、对比的另一边、否定（红叉）、疑问（问号）、想通了（灯泡）、价格或数字牌时才加；最多 {{pairs}} 个镜头放 2 个。台词很短的镜头不要加，程序也会去掉来不及看的元素）：
   - 素材库里有的直接写 `{"lib": "<id>"}`；
   - 素材库没有的写 `{"draw": "<英文，一个单独的物件>", "texts": ["≤6字"]}`，每镜最多 1 个这样的生成元素；
   - `at`：这个元素在台词说到哪几个字时出现，抄台词里连续的 2–4 个字；
   - 元素不要和主卡片画重复的东西。
   - **位置用关系描述，不写坐标**（程序会按图片的真实形状排版、居中、留边距）：
     - `relation`：`beside` 站在主卡片旁边的地面上（默认）；`above` 飘在目标头顶（问号、灯泡、想法、箭头）；`on` 盖在另一个元素上面（红叉盖在牌子上、价签贴在商品上）。
     - `target`：`above` / `on` 挂在哪个元素上，写它在 elements 里的序号（0 或 1）；不写表示挂在主卡片上。
     - `size`：相对于一个角色的身高：`tiny`（硬币、票）、`small`（汉堡、钱袋、符号）、`medium`（牌子、图表、垃圾桶）、`large`（另一个角色）。
     - `side`：`left` / `right`，只有故事里有明确方向时才填（例如从左边走来的人），否则 null 让程序挑。
4. `motion` 只在特殊画面时填（例如价格突然升起用 rise、新角色闯入用 drop），**全片最多 {{motion_max}} 个镜头填 motion**，其余必须是 null，相邻两个镜头不要用同一种。元素的 `motion` 也一样，大多数留 null。

## 只输出一个 JSON 对象

```json
{
  "title": "什么是诱饵效应",
  "shots": [
    {"i": 0, "kind": "story", "draw": "the red crab shop owner stands behind a short wooden counter with two menu boards on it", "texts": ["基础套餐", "豪华全餐"], "cast": ["krabs-explain"], "motion": null,
     "elements": [{"lib": "fish-customer", "at": "顾客", "motion": "walkIn", "relation": "beside", "size": "large", "side": null}]},
    {"i": 1, "kind": "story", "draw": "a wooden price board standing on the ground", "texts": ["18贝壳"], "cast": [], "motion": "rise",
     "elements": [{"draw": "a small wooden sign on a post", "texts": ["只多薯条"], "at": "薯条", "motion": null, "relation": "beside", "size": "medium", "side": null},
                  {"lib": "mark-x", "at": "不划算", "motion": "stamp", "relation": "on", "target": 0, "size": "small", "side": null}]},
    {"i": 2, "kind": "sublime", "draw": "a single open menu with a magnifying glass resting on it", "texts": [], "cast": [], "motion": null, "elements": []}
  ]
}
```
