# 共用编辑能力层 v1

入口：`todo_application::control`，由 `Workspace::control_context()` 和 `Workspace::control(ControlRequest)` 提供。用于后续内置助手、外部 AI、脚本和自动化适配器。当前实现不包含模型连接、聊天 UI、HTTP/MCP 服务。

## 调用约定

1. 在持有 Workspace 的线程读取 `control_context()`。没有工程时返回 None。
2. 以快照中的 sessionId、revision 构造 ControlRequest；所有修改都必须填写 expectedRevision。
3. 调用 `workspace.control(request)`，拿到 `ControlOutcome { response, update }`。
4. 将 update 交给现有运行时的 AppUpdate/Effect 处理流程。导入任务仍经 Workers 执行，AppEvent 经 Workspace::on_event 回传。宿主还应按现有 App::handle 流程 reconcile 控制器、刷新预览和请求重绘。
5. 向调用方返回可序列化的 response。查询任务以确定异步导入是否完成。

Workspace 由单一宿主持有，`&mut self` 调用顺序串行。将来 HTTP/MCP 适配器应把请求送到该宿主，不另建工程副本作为可写状态。本层不执行文件系统、网络或线程操作，不触发文件对话框。

快照包含工程、轨道/片段、素材、播放头、播放状态、撤销/重做、dirty、忙状态和保留的导入任务。工程快照内的 playhead 与实时播放头同步。当前选择、焦点和面板属于 interaction，后续入口需要时在适配边界补充。

## 编辑示例

先读取上下文，使用实际返回的 sessionId、revision、mediaId 和 trackId：

```json
{
  "requestId": "place-and-cut-1",
  "sessionId": 1,
  "expectedRevision": 0,
  "command": {
    "type": "apply_edits",
    "dryRun": false,
    "edits": [
      {
        "type": "place_media",
        "mediaId": "A",
        "trackId": "v1",
        "timelineStart": 10,
        "sourceRange": { "sourceIn": 3, "sourceOut": 8 }
      },
      {
        "type": "split",
        "clips": [{ "type": "created", "step": 0 }],
        "timelineTime": 12
      }
    ]
  }
}
```

所有时间为秒，源素材区间使用 sourceIn/sourceOut，时间轴位置使用 timelineStart/timelineTime。v1 保留请求精度，不隐式应用鼠标吸附或帧量化；core 要求片段至少一个工程帧长。图片区间要求 sourceIn=0，sourceOut 表示静态画面的停留时长。工程的 duration 是固定工作区/播放结束设置；片段可以超出该边界，超出范围不会自动修改 duration。

省略 sourceRange 表示整段入轨。省略 trackId 表示自动选择轨道；指定轨道被占用时沿用编辑器的溢出策略，可能改放兼容轨道或新增轨道。回执提供实际轨道 ID。

ClipRef 可为 `{ "type": "id", "id": "已有片段ID" }`，或上例的 created 引用。step 从 0 开始，只能指向之前步骤生成且目前仍存在的片段。一次视频入轨可能生成画面和关联音频，created 引用包括二者。TrimClip 和 SetTransform 接收单个 ClipRef；created 分组取轨道顺序中的第一个片段，通常是画面。切开保留左段 ID，右段获得新 ID，切开步骤的 created 仅包含右段，splits 提供原片段/左段/右段映射。

## 命令

| type | 参数 | 结果 |
| --- | --- | --- |
| get_context | 无 | 实时快照，忽略 expectedRevision |
| get_import_job | jobId | 导入进度，忽略 expectedRevision |
| import_media | paths 字符串数组 | 仅导入素材库，返回任务 ID |
| apply_edits | edits, dryRun（默认 false） | 步骤回执及整体 changed |
| undo / redo | 无 | 是否发生内容变化 |

编辑类型为 place_media、move_clips、trim_clip、split、delete_clips、add_track、set_transform、add_text、set_text。add_text 参数为 trackId（可空）、timelineStart、duration、content；set_text 参数为 clip 和 content。content 即工程中的 `TextContent`，内置样式 id 会在首次使用时复制进工程，格式见 [花字层标准](text-layer.md)。移动、删除的 clips 为 ClipRef 数组；linked 布尔值明确是否展开关联对象。trim_clip 的 edge 为 in/out，linked 明确是否同步裁切。split 始终包含关联片段，人工 EditCommand::Split 保留显式选择行为。set_transform 使用完整 core Transform；画面位置、缩放和不透明度沿用现有数据约定。

move_clips 优先使用指定轨道，目标区间被占用时复用同类型空闲轨道，必要时新增轨道。未指定轨道的成员优先保留原轨道；关联音视频使用相同时间位移，各自避开轨道冲突。原来同轨道的选中片段作为一组分配，保留内部间距；clips 中第一个片段所在分组优先获得目标轨道。锁定的源轨道或显式目标轨道仍使整个步骤失败。实际落点与新增轨道通过 updatedClips / createdTrackIds 返回。

set_transform 的 transform 必须完整提供 x、y、scale、scaleX、scaleY、opacity；缺字段或拼错字段均在解析请求时拒绝。此严格输入约定只属于共用接口，工程存储仍保留旧文档缺字段时使用默认值的兼容行为。

每个步骤返回 createdClips、updatedClips、deletedClips（含 trackId 和完整 clip）、createdTrackIds、deletedTrackIds、splits。批次先在副本上顺序执行，成功提交一次历史与一次预览更新。错误包含失败 step，批次不留下部分修改。changed=false 明确表示未发生内容修改，例如片段外的切点。

dryRun=true 返回相同类型的校验结果和变化，但不改变工程、历史、播放头或 revision。模拟生成的 ID 不能用于下一次正式请求，正式执行重新生成 ID。

## 会话、修订号和重试

- sessionId 随工程重新打开变化，旧导入回执和旧请求均不能作用于新工程。
- revision 是独立的运行时内容修订号。人工编辑、导入、撤销/重做、手势取消导致的实际内容变化均递增；播放头、播放和保存回执不递增。工程文档 version 仍表示文档格式。
- 活动手势、导入、关闭确认或工程切换期间，修改返回 editor_busy；保存任务允许与编辑并行，沿用已有快照保存机制。
- 每个修改使用新的 requestId（1..128 字节，不能全为空白）。同 ID 同内容重试返回原回执，update 为空，不重复副作用；改变任何请求字段均返回 request_id_conflict。
- 最近 128 个修改请求的回执可重放，包含失败和 dryRun 请求。原回执中的 revision 是当时的值，调用方需要重新读取实时上下文。过期 ID 返回 request_expired，绝不重新执行。
- 读取查询不缓存，重复 ID 仍返回实时查询结果。每会话最多保留 16384 个修改 ID 标记，之后返回 request_history_full；重新打开工程建立新会话。
- 批次为 1..128 步，导入为 1..256 个非空路径。输入拒绝未知字段。后续网络适配器还应落实连接认证、路径访问范围及请求体大小限制。

成功响应结构：

```json
{ "apiVersion": 1, "requestId": "undo-1", "sessionId": 1, "revision": 2,
  "status": "ok", "result": { "type": "history", "changed": true } }
```

错误响应结构：

```json
{ "apiVersion": 1, "requestId": "edit-2", "sessionId": 1, "revision": 2,
  "status": "error", "error": { "code": "track_locked", "message": "Edit rejected: track_locked", "objectId": "a1", "step": 1 } }
```

常见错误：no_project、session_conflict、revision_required、revision_conflict、editor_busy、invalid_request、invalid_clip_reference、request_id_conflict、request_expired、job_not_found；领域错误保留 media_not_found、clip_not_found、source_bounds、overlap、track_locked 等原始代码。

## 导入生命周期

import_media 仅登记素材库，现有 UI 导入仍自动追加时间轴。两者共用 Rust 本地复制/探测 worker、Ticket、保存和回执处理，最近 128 个导入任务可查询。

任务状态为 running、completed、completed_with_errors 或 failed。每个成功文件有 mediaIds，失败有 errors，completed 表示已处理文件数。部分失败不影响后续文件。重复的成功回执按素材 ID 去重。ImportAborted 用于队列停止等终止错误，释放编辑器 importing 状态。

文件导入独立于时间轴事务；时间轴批次 undo 保留素材库已有素材。导入登记本身也可撤销，但 undo 不删除磁盘源文件。导入沿用自动保存，时间轴编辑沿用编辑器的显式保存流程。

登记前校验探测元数据，非法数据不进入工程。UI 自动追加失败时，合法素材仍保留在素材库并列入任务 mediaIds，errors 报告追加失败，completed 只计一次。本次命令失败由 EditorUpdate.error 明确回传，任务不读取之前遗留的界面状态。

## 验证与示例

```powershell
cargo run -p todo_application --example control
cargo test -p todo_application --test control
cargo test -p todo_core --test control_edits
```

示例使用内存素材，不打开窗口或读取本地文件，执行“源 3..8 秒放到 10 秒，再在 12 秒切开”，打印真实请求和回执，再验证一次撤销。
