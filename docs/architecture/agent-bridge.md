# Agent Bridge v1

TO DO 启动后会在本机回环地址打开一个 JSONL bridge。外部 Agent 用**短令牌**定位窗口与工程，无需口述软件路径。

## 令牌（推荐）

工具栏点「令牌」复制一段以 `TODO_BRIDGE todo_…` 开头的**接管指令**（含令牌与步骤），整段粘贴给 Cursor / 外部 Agent 即可。单独的 `todo_<hex>` 也是 Bridge 令牌，**不是**待办 ID。

登记文件（打开工程后带上 session / 工程名）：

- `%LOCALAPPDATA%\TO_DO\bridges\<token>.json`
- `%LOCALAPPDATA%\TO_DO\bridges\current.json`（当前窗口）
- 仓库 `cache/agent-bridge.json`（兼容冒烟脚本）

```json
{
  "apiVersion": 1,
  "token": "todo_…",
  "transport": "tcp",
  "host": "127.0.0.1",
  "port": 43127,
  "protocol": "jsonl",
  "sessionId": 7,
  "revision": 12,
  "projectName": "未命名",
  "projectPath": "D:/…/data/未命名"
}
```

Agent：解析 `todo_*` → 读登记 JSON → TCP 连接 → `get_context` 核对 `sessionId` / `projectName` → 再编辑。

### 分层入轨（硬规则）

- 时间轴上必须是**多层**：背景、元素、花字、配音等各自为 clip（扩展 `emit` 多条 `todo-items/1`，再由宿主 adopt 入轨）。
- **禁止**把背景和元素烤成一条视频/一张整图后 `import_media` + `place_media`。
- 推荐：`start_workflow` 使用会分层 emit 的扩展（如 `studio.spongebob-koubo`）。见 [extension-spec](extension-spec.md)「可编辑优先」。

冒烟：

```text
cargo run -p todo_app --example bridge_smoke -- todo_<粘贴的令牌> get_context
cargo run -p todo_app --example bridge_smoke -- current get_context
```

## 传输

Agent 每行发送一个 JSON 请求，应用每行返回一个 JSON 响应。连接只绑定 `127.0.0.1`，请求仍在 eframe 所属线程执行，避免 Agent 持有一份会和界面分叉的工程副本。

## 工程上下文与编辑

普通编辑请求沿用 [`todo_application::control`](shared-control.md) 的 JSON 协议：

```json
{
  "requestId": "edit-1",
  "sessionId": 7,
  "expectedRevision": 12,
  "command": {
    "type": "apply_edits",
    "dryRun": true,
    "edits": [
      {
        "type": "place_media",
        "mediaId": "media-1",
        "trackId": null,
        "timelineStart": 0,
        "sourceRange": { "sourceIn": 0, "sourceOut": 5 }
      }
    ]
  }
}
```

先用 `get_context` 读取当前 `sessionId`、`revision`、素材 ID 和轨道 ID，再使用 `dryRun` 检查；正式提交必须带读取到的 `expectedRevision`。成功后应用会把返回的 `AppUpdate` 交给现有 Effect 管线，时间轴和节目窗会走同一条预览更新路径。导入使用 `import_media`，返回任务 ID，再用 `get_import_job` 等待素材 ID。

## 节目帧审核

Agent 可以发送：

```json
{
  "requestId": "review-1",
  "sessionId": 7,
  "command": {
    "type": "render_preview",
    "times": [0, 2.5, 5]
  }
}
```

`times` 省略时使用当前播放头，最多请求 16 个非负时间点。响应返回与节目窗使用同一套图层、解码器和着色器生成的 PNG 路径：

```json
{
  "apiVersion": 1,
  "requestId": "review-1",
  "sessionId": 7,
  "revision": 12,
  "status": "ok",
  "result": {
    "type": "preview_frames",
    "frames": [
      { "time": 2.5, "png": "D:/.../cache/agent-frames/review-1/frame-00.png", "width": 1920, "height": 1080 }
    ]
  }
}
```

Agent 可读取这些 PNG，结合 `describe` 返回的 z 序、矩形、可见性和警告进行审核，然后带着同一个 session 的最新 revision 再提交修正。渲染在后台线程执行，不阻塞编辑器帧循环。

## 当前边界

- Bridge 是本机回环接口，没有公网监听，也没有自动登录或远程凭据转发。
- `render_preview` 返回文件路径；Agent 需要能访问当前机器的工作区文件。
- 预览审核是“渲染 → Agent 判断 → 再提交”的工具闭环，应用不会替 Agent 自己判断画面是否合格。
