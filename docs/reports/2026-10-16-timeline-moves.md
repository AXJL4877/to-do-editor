# 时间轴贴合与关联音视频移动优化

## 结果

移动片段可以准确贴齐非整帧的素材边界。关联音视频移入重叠区间时，优先使用目标轨道，再复用同类型空闲轨道，必要时新增轨道。整组保持相同时间位移、原始源区间与关联标识，一次拖动仍只产生一次撤销。

本次后续修复还将工作区时长与内容范围分离：设置为 300 秒后，长素材完整保留并可在 300 秒外滚动编辑，播放仍在 300 秒停止，时间轴在工作区边界显示标记。

现有项目文件中如果已经被旧逻辑写入了错误的 duration，需要在一次打开工程后通过时长设置恢复为 300 秒；之后导入、移动、裁切和粘贴不会再自动改写这个值。

## 定位与修复

1. 原吸附计算找到精确边界后再次按工程帧率取整，带小数时长的素材会留缝，或形成超过浮点容差的实际重叠。现在吸附直接采用目标边界；普通未吸附移动继续按帧对齐。
2. 原移动策略遇到画面或音频任一轨道重叠就拒绝整个操作。现在按源轨道分组，移除选中片段后统一分配空闲轨道，保持组内间距与关联对象的同步。
3. 鼠标的横向滚动变化原本没有计入拖动位移；现在以拖动开始时的滚动位置计算差值。
4. 已选中的关联视频再次被拖动时，原先可能继续以此前选中的音频为主片段；现在以实际点击拖动的片段作为主片段与目标轨道类型依据。
5. 新增视频轨道会改变音频行索引。现在行索引从当前显示工程转换为稳定轨道 ID，避免从原始工程索引到错误行。
6. 独立审查发现鼠标指向手势中新建轨道后，继续移至无冲突时间会跳回旧轨道。已用端到端测试先复现失败，再在 application 的本次执行副本中保留明确指向的空目标轨道；原始撤销基线不变。撤销、重做与 Escape 取消均包含轨道变化。

锁定源轨道或显式锁定目标仍拒绝整个操作。移动不会覆盖或裁掉未选中片段，也不会强制展开被用户关闭的关联选择。

## 文件分类

- `todo_core/src/edit/track_space.rs`：素材入轨和片段移动共用的空闲检查、轨道复用与创建。
- `todo_core/src/edit/move_clips.rs`：移动校验、按源轨道分组、主片段优先和原子提交。
- `todo_core/src/edit/tracks.rs`：沿用轨道 ID 和类型分区规则，提供内部空轨创建入口。
- `todo_interaction/src/timeline/`：精确吸附、拖动坐标、主片段与显示行转换。
- `todo_application/src/editor/commands.rs`：手势临时目标轨道的执行副本与历史边界。
- `todo_core/tests/timeline_moves.rs`、`todo_interaction/src/input/tests/timeline_moves.rs`：分别保存领域与输入/应用端到端回归。

共用 AI 接口沿用 `MoveClips`，不改变请求结构。已有步骤回执继续返回真实轨道和 createdTrackIds；说明已同步至 shared-control.md。

## 验证

新增 13 项回归测试：6 项 core、7 项 interaction。修复前首先观察到 4 项 core 和 5 项输入测试失败，分别复现重叠拒绝、非整帧贴合、主片段和滚动问题；审查发现的新轨道问题也单独先复现失败。

| 检查 | 结果 |
| --- | --- |
| cargo test --workspace --quiet | 325 passed，0 failed |
| cargo check --workspace | 通过 |
| cargo clippy --workspace --lib --bins --examples -- -D warnings | 通过 |
| cargo clippy -p todo_interaction -p todo_application --all-targets -- -D warnings | 通过 |
| cargo clippy -p todo_core --test timeline_moves --test domain_edits --test editor_basics -- -D warnings | 通过 |
| 本轮 13 个修改 Rust 文件的 rustfmt 检查 | 通过 |
| ./build.ps1 -Configuration debug | 通过，更新 dist/to_do |

固定工作区后重新验证：`cargo test --workspace --quiet` 为 336 passed，0 failed；`cargo check --workspace`、`cargo clippy --workspace --lib --bins --examples -- -D warnings` 和 `cargo clippy -p todo_application -p todo_interaction -p todo_ui --all-targets -- -D warnings` 均通过。全库 all-targets Clippy 仍被未修改的 `todo_core/tests/track_insertion.rs:176` 既有 `double_ended_iterator_last` 告警阻断。

原有“移动重叠应失败”的测试已按新策略改为自动分轨；裁切重叠、类型不兼容和锁定拒绝规则继续保留。覆盖场景包括双边贴合、音视频同时分轨、仅音轨冲突、复用空轨、跳过锁定空轨、多源轨道选中、精确相邻、连续拖动、新轨道定位、撤销/重做和取消。

日志位于 target/timeline-move-*.log 和 target/workspace-time-*.log。时间轴移动审查发现的新轨道落点问题已修复；现有时间轴自动缩放和右键菜单主选择策略没有已确认的本轮回归，继续保留原行为。

## 交付边界

本轮验证使用真实领域、输入和应用会话代码，未进行窗口鼠标交互验收；未改动原生预览渲染代码。调试目录包已更新，未执行 release 打包。工作留在当前未提交迁移目录，没有切换分支、创建提交或修改用户工程数据。

操作规则和模块职责见 [时间轴编辑](../architecture/timeline-editing.md)。
