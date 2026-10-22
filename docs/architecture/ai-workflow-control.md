# AI 工作流控制面 v1

更新：2026-10-22（刀5：`import_extension`）。与 UI 同源：外部 Agent / 内置 Agent / 成片面板共用 `Workspace::control` 与桌面 `ProductionHost`，禁止旁路改 `production.json`。

权威相关文档：`agent-bridge.md`、`shared-control.md`、`production-pipeline.md`、`extension-spec.md`。

## 1. 传输

- Agent Bridge：本机 JSONL（见 `agent-bridge.md`），发现文件 `cache/agent-bridge.json`。
- 需要宿主的命令（`needs_production_host`）由桌面 runtime 走 `adapters/production/workflow_control.rs`；只读工程态走 `Workspace::control`。
- 变更类命令必须带读取到的 `expectedRevision`；先 `dryRun` 再提交（`import_extension` / `start_workflow` 的 dry-run 仅需合法 session）。

## 2. 商业运行态（三方一致）

`WorkflowRunState`（serde `snake_case`）：

| 状态 | 含义 |
| --- | --- |
| `idle` | 无活动 / 未开始 |
| `validating` | 预留：启动前校验（当前 dry-run 瞬时完成） |
| `running` | 有本地 run 或阶段/节点在跑 |
| `waiting_service` | 等网络/AI/本地服务 |
| `paused_error` | 失败或 workflow error；可重试 |
| `completed` | done==total 且阶段均为 Done，且入轨未失败（placed>0 或用户主动清空）；`placed=0`+`unadoptable`/`placementBlocked` → `paused_error`，可 `retry_workflow` |
| `cancelled` | 用户取消 / 停止 |

映射实现：`todo_application::control::workflow::commercial_run_state`。  
UI 节点态（queued/running/waiting/…）仍通过 `ui_aggregate` / `nodes[].uiState` 暴露；Bridge 读 `runState`。

## 3. 命令一览

| type | 宿主 | 作用 |
| --- | --- | --- |
| `get_context` | 否 | 工程 revision / session / 媒体与轨道摘要 |
| `describe` | 否 | 元素级几何快照 |
| `get_production_context` | 否 | 成片/工作流快照：runState、节点、错误、入轨摘要 |
| `get_workflow_run` | 否 | 单个 production / runId 轮询 |
| `list_extensions` | **是** | 已安装扩展摘要与自检状态 |
| `import_extension` | **是** | 安装 `.todoext`（`dryRun` 只校验）；与 `todo_workflow::install` / Host 同源 |
| `start_workflow` | **是** | `dryRun` 校验；正式启动返回 `productionId`+`runId` |
| `cancel_workflow` | **是** | 停止 live graph（与 UI 停相同路径） |
| `retry_workflow` | **是** | `Workspace::retry_production`（可带 `taskKey`）；并清除 placement rejected |
| `apply_edits` / `undo` / `redo` | 否 | 已入轨条目的安全编辑（既有协议） |
| `import_media` | 否 | 素材导入（非扩展包安装） |

### `import_extension`

```json
{
  "type": "import_extension",
  "path": "D:/packs/demo.todoext",
  "dryRun": true
}
```

- 成功：`result.type = extension_imported`（含 `extension` 摘要、`dryRun`、`message`）。
- 失败：`error.code = import_extension_failed`，`message` 人类可读（缺清单 / 路径不安全 / 非 zip 等）。
- 正式安装后 `list_extensions` 可见（`origin=user`）。

错误码（常见）：`no_project`、`session_conflict`、`revision_required`、`revision_conflict`、`editor_busy`、`production_not_found`、`run_not_found`、`workflow_host_required`、`cancel_failed`、`import_extension_failed`、`invalid_request`。校验失败另见 `WorkflowValidation.problems`。

### 入轨对账字段（`placement`）

| 字段 | 含义 |
| --- | --- |
| `placed` / `tombstones` / `positions` / `deletedOnTimeline` | 对账计数 |
| `pendingAdopt` | 正在登记的媒体数 |
| `unadoptable` | adopt 失败、待 retry 清除的路径数 |
| `placementBlocked` | ApplyGenerated 已拒绝当前签名；需 `retry_workflow` |

人类面板：`attention` 中「入轨未成功」会升级为问题卡（点重试）；缺 Key →「打开 AI 设置」。

## 4. 最小调用序列（操作卡）

```text
1. get_context                     → sessionId, revision
2. （可选）import_extension dryRun → 校验 .todoext
3. import_extension                → 写入 extensions/user
4. list_extensions                 → 选 extensionId（status=ready）
5. start_workflow { dryRun:true }  → 看 ok / problems
6. start_workflow { dryRun:false, input, params, expectedRevision }
                                   → productionId, runId
7. 轮询 get_workflow_run / get_production_context
                                   → runState、nodes、placement、error、attention
8. 入轨后必要时 apply_edits(dryRun→commit) 改时间轴
9. 失败：retry_workflow；placementBlocked 时同样 retry；tombstone 同 rev 不复活
```

### 示例：dry-run 启动

```json
{
  "requestId": "wf-dry-1",
  "sessionId": 7,
  "expectedRevision": 12,
  "command": {
    "type": "start_workflow",
    "extensionId": "studio.spongebob-koubo",
    "dryRun": true
  }
}
```

### 示例：读入轨对账

```json
{
  "requestId": "prod-1",
  "sessionId": 7,
  "command": {
    "type": "get_production_context"
  }
}
```

成功时 `result.type = production_context`，每条 `productions[]` 含 `runState`、`placement`（含 `placementBlocked` / `unadoptable`）、`recentErrors`、`attention`。

## 5. 本地验证（默认离线）

```powershell
cargo test -p todo_production --test placement --quiet
cargo test -p todo_application --lib productions::tests --quiet
cargo test -p todo_application --lib productions::session::resume_tests --quiet
cargo test -p todo_application --lib control::workflow --quiet
cargo test -p todo_app --lib adapters::production::workflow_control --quiet
```

### 真机 Bridge TCP 冒烟（刀8）

1. 启动 `todo_app`，打开工程，确认 `cache/agent-bridge.json`。
2. 另开终端：

```powershell
cargo run -p todo_app --example bridge_smoke -- list_extensions
cargo run -p todo_app --example bridge_smoke -- start_dry studio.spongebob-koubo
cargo run -p todo_app --example bridge_smoke -- get_production_context
```

期望：JSONL `status=ok`；`list_extensions` 含 `studio.spongebob-koubo`；`start_dry` 的 `validation.ok=true`。无应用时 exit 2 并提示先开应用。

付费线上 E2E（`run_workflow_online` 等）默认不跑；需明示授权。

## 6. 与 UI 同源保证

- 安装：`Host::import_extension` → `todo_workflow::install` / `validate_package`。
- 启动：`bind_canvas_workflow` + `Host::start_workflow`（见 `workflow_control::start_workflow_like_ui`）。
- 重试：同一 `retry_production`（清 rejected / unadoptable / placement attention）。
- 取消：同一 `host.stop_workflow`。
- 入轨：同一 `place_productions` / `ApplyGenerated`；Agent 不得手改 placement JSON。

## 7. 已知限制（已批准）

- 渐进 emit = 多撤销步；adopt 媒体登记不进撤销。
- 续跑若 Host 分配了新 workdir：相对路径可能失效（attention 提示；绕过：复用原 workdir）。
- 暂停（pause）：暂不支持；用 `cancel` + `retry`。
- `get_placement_summary`：已并入 `get_production_context` / `get_workflow_run` 的 `placement`，不单开命令。
