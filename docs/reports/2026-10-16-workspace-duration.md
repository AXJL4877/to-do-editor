# 固定工作区时长与溢出内容

## 规则

工程 `Project.duration` 现在只表示用户设定的工作区长度和播放结束位置。导入、放置、移动、裁切、变速和粘贴都不会再自动调用扩展工作区的逻辑。

如果工作区设置为 300 秒，导入 600 秒素材后：

- 素材库保留完整 600 秒源文件；
- 时间轴片段保留完整 600 秒范围，即使超出 300 秒；
- 播放和播放头结束仍限制在 300 秒；
- 时间轴横向可编辑范围延伸至片段真实结束位置；
- UI 在 300 秒处显示“工作区结束”边界，边界外区域加暗；
- 轨道吸附可以把片段边缘贴到 300 秒边界，但不会剪掉边界外内容。

`content_duration` 计算片段实际最晚结束时间，`timeline_extent` 返回工作区长度与内容长度的较大值。时间轴像素密度只按固定工作区长度计算，所以导入长素材不会把 3:45 之后的片段重新缩放或改变拖动坐标。

## 实现位置

- `crates/todo_core/src/query/project.rs`：工作区长度、内容长度和可滚动范围查询。
- `crates/todo_core/src/edit/`：移除自动扩时；显式 `set_project_duration` 仍可修改设置。
- `crates/todo_application/src/editor/imports.rs`：自动追加按内容末尾放置，播放头仍限制在工作区末尾。
- `crates/todo_interaction/src/presentation/mod.rs`：固定像素密度、内容溢出轴和工作区边界数据。
- `crates/todo_interaction/src/timeline/snapping.rs`：工作区结束加入吸附目标。
- `crates/todo_ui/src/widgets/timeline/board.rs`：绘制边界线、标签与边界外暗区。

## 验证

新增 11 项回归测试，覆盖长视频入轨、直接添加、3:45 后移动、裁切/变速、粘贴、显式调整工作区、长素材连续导入、溢出滚动、固定像素映射和边界吸附。测试先在旧逻辑上失败，再在新逻辑上通过。

最终 `cargo test --workspace --quiet` 为 **336 passed，0 failed**；`cargo check --workspace`、生产 Clippy、application/interaction/UI 的 all-targets Clippy、本轮修改文件的 rustfmt 检查均通过。调试包由 `build.ps1 -Configuration debug` 重新生成，主程序与 `target/package/debug/todo_app.exe` 哈希一致。

全库 all-targets Clippy 仍有一个既有、未修改的 `todo_core/tests/track_insertion.rs:176` 告警；它不影响生产代码和本轮相关 target 的检查。
