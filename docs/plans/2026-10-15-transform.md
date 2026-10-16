# Transform 架构实施计划

**Goal:** 建立独立的领域变换模块，统一元素的位置、二维尺寸、透明度，并支持属性编辑和原生节目预览的移动/缩放控制框。

**Architecture:** core/transform 保存持久化模型、校验与纯几何。interaction/transform 负责命中和手势，inspector 负责属性投影/输入；UI 只绘制参数。application 继续使用 SetTransform 事务与历史；app 只映射数据和转发。native 通过 D3D 绘制变换、透明度及控制框，控制框状态不进入工程。

**Tech Stack:** Rust workspace、egui/eframe、Windows D3D11、已有原生预览协议。

**Spec:** 本轮用户要求加入 core Transform 架构，覆盖位置、大小、透明度及预览拖拽；保持清晰目录。

## 模型与接口约定

- `todo_core::transform::Transform`：`x,y,scale,scale_x,scale_y,opacity:f64`。`ClipTransform` 为兼容别名。x/y 仍为相对于画布宽高的中心偏移；scale 为统一缩放；scale_x/y 为横向/纵向尺寸倍率；opacity 范围 0..=1。
- 新字段默认均为 1；旧 transform 缺字段时保持原效果；缺 transform 仍使用居中原大小、不透明。
- `Transform::bounds(source:(u32,u32),canvas:(f64,f64))->Option<TransformBounds>`，返回归一化画布坐标。`TransformBounds {left,top,right,bottom:f64}`，提供 width/height/center 方法。几何无 UI、窗口和 I/O 依赖。
- `PreviewView.selection:Option<TransformBounds>`：仅当前被选中且正在显示的可视片段投影控制框。`editable` 决定是否画手柄/允许编辑。
- `PreviewHost::set_transform(VisualTransform)` 传六参数；保留三参数旧 `transform`。`VisualTransform` 是 preview 的传输类型，不依赖 core。
- `PreviewHost::set_gizmo(Option<TransformGizmo>)`；`TransformGizmo {left,top,right,bottom:f64,editable:bool}`。控制框属于交互显示，app 直接从 ViewModel 转给预览桥接，不进入 application 的持久化命令。
- 预览拖动内部移动；边缘手柄调整单轴尺寸；角手柄保持比例缩放；固定对侧边/角；缩至最小尺寸停止，不翻转。一次手势一次撤销，Escape 取消，锁定不可编辑。
- 属性栏提供位置 X/Y、统一缩放、横向/纵向缩放、透明度百分比和重置，沿用数值编辑事务。

## 文件职责与任务

- [ ] core：新增 `src/transform/{mod,model,geometry}.rs`，从 model/clip.rs 抽离旧结构，集中 SetTransform 校验，保持历史/复制/切分/序列化兼容。测试默认/旧 JSON、二维比例与透明度、非法值、编辑后保留。
- [ ] interaction：新增 `src/transform/` 放命中与手势；input/preview.rs 只协调事务和选择。投影 selection，移动/八手柄缩放/取消/锁定测试，保留原预览输入回归。
- [ ] native/preview：扩展六参数线协议，变换和透明度只重绘保留帧；归类新几何/控制框模块；GPU 控制框、八手柄、光标，协议与原生回归。旧进程不强制关闭。
- [ ] UI/app/application：属性控件扩展，适配六参数和控制框转发，更新受影响构造/兼容测试。窗口大小变化只更新控制框投影，不写工程。
- [ ] 验证：Rust workspace 测试、Clippy/格式、native 构建与 Python 回归、用户工程只读兼容及哈希；更新职责与修改报告。

## 复核重点

- 高 DPI、预览面板大小变化、宽/竖素材下的手柄位置与命中一致。
- 不透明度 0 时仍能通过控制框找回/修改元素；黑边与控制框不受元素透明度影响。
- 属性未提交时开始预览手势、切换片段和 Escape，不丢失其他编辑。
- 旧 JSON 的 x/y/scale 与新参数默认行为一致；新参数经过粘贴、剃刀、裁切、移动和保存后不丢失。
- 已显示旧可执行文件由用户保存关闭；构建产物若被占用，在 target 中以独立名称验证。
