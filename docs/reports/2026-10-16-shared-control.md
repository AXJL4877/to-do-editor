# 共用编辑能力层交付记录

## 本轮范围

实现 `todo_application::control` 的 Rust 调用入口和可序列化 JSON 协议，供后续内置助手与外部 AI 适配器共用。本轮不启动聊天 UI、模型连接或 HTTP/MCP 服务。

入口是 `Workspace::control_context()` 与 `Workspace::control(ControlRequest)`。协议和宿主 Effect 接法见 [接口说明](../architecture/shared-control.md)。

## 已实现

- 工程/素材/时间轴快照，独立会话和内容修订号。
- 素材库导入、任务进度、成功素材 ID、部分失败和终止回执。
- 指定源区间入轨、移动、裁切、关联切开、删除、添加轨道和画面变换。
- 顺序执行的原子批次，失败不提交，成功只产生一次撤销；可引用前一步创建的片段。
- dryRun、结构化领域错误、切开 ID 映射、实际片段及轨道回执。
- 修改请求去重、回执重放、过期请求拒绝、人工修改后的修订冲突检查。

## 相关优化

- 源区间入轨按实际选中时长判断轨道占用，音视频使用一致的源偏移。
- 新增 SplitLinked，保留原 Split 的显式选择语义。
- 统一手工编辑、导入和共用批次的工程提交逻辑。
- 分离素材登记与可选自动入轨；手工导入继续自动追加。
- EditorUpdate.error 只描述本次命令，导入任务不再依赖遗留 UI 状态。
- 非法导入元数据拒绝进入工程；合法素材自动入轨失败仍保留并正确报告素材 ID。
- 成功导入回执去重；工作线程停止时结束导入状态。
- 控制接口严格校验完整变换参数，保留原有工程文档的兼容反序列化。

## 验证证据

| 检查 | 结果 |
| --- | --- |
| cargo test --workspace --quiet | 306 passed，0 failed |
| npm test --prefix backend | 7 passed，0 failed |
| cargo check --workspace | 通过 |
| cargo clippy --workspace --lib --bins --examples -- -D warnings | 通过 |
| cargo clippy -p todo_application -p todo_app --all-targets -- -D warnings | 通过 |
| cargo clippy -p todo_core --test control_edits -- -D warnings | 通过 |
| 本轮全部 Rust 文件的 rustfmt --check | 通过 |
| cargo run -p todo_application --example control | 通过，真实内存工程完成入轨、切开及一次撤销 |

本轮新增 27 项回归测试：core 5 项、共用接口 21 项、停止导入队列 1 项。独立只读代码审查发现的两项问题均通过新增测试复现失败，再修复通过。

完整验收覆盖：导入素材库后使用返回的素材 ID 入轨；源 3..8 秒放在时间轴 10 秒并在 12 秒切开；关联音视频保持对应；一次撤销保留导入素材。

## 检查边界

- 全库 `cargo fmt --all -- --check` 仍报告本轮未改动文件中的既有格式差异，例如 core/edit/tracks.rs、validate.rs、interaction/input/preview.rs。
- 全库 `cargo clippy --workspace --all-targets -- -D warnings` 被既有 core/tests/track_insertion.rs:176 的 double_ended_iterator_last 告警阻断；生产代码及本轮相关测试的 Clippy 均通过。
- 没有新增实时 UI 或外部通信入口；宿主和预览 Effect 的连接通过应用层与 worker 回归验证，没有进行新增功能的窗口交互验收。
- 沿用当前工作区的未提交迁移，没有创建提交、切换分支或修改用户工程数据。
