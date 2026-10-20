# AiRouter 能力接口

`AiRouter` 是模块、扩展和内置 Agent 的 AI 入口。调用方只声明能力和请求，不声明供应商；供应商适配、模型绑定、重试、任务日志和产物落盘由 `todo_ai` 处理。

```text
模块 / 扩展 / Agent
        │ TaskRequest
        ▼
     AiRouter
        │ CapabilityBindings
        ▼
  provider adapter
        │
  AiService journal / retry / artifacts
```

`TaskRequest` 的公开能力是：

| 能力 | 请求 | 当前用途 |
| --- | --- | --- |
| `Language` | `LanguageRequest` | 分段、分镜、Agent 规划和文本生成 |
| `Image` | `ImageRequest` | 图片素材和候选图 |
| `Video` | `VideoRequest` | 视频素材请求，等待视频供应商适配器 |
| `Multimodal` | `MultimodalRequest` | 节目帧、图片和视频审核，等待多模态适配器 |
| `Audio` | `AudioRequest` | 配音、语音、BGM 和音效入口 |

`CapabilityBindings` 为每种能力保存默认模型和内部供应商。模型为空的请求由路由器补齐；扩展只有在确实需要覆盖默认模型时才在请求中填写 `model`。`TaskSpec` 只保存能力请求和稳定指纹，不再保存 `Provider`，所以同一套模块协议可以迁移到另一个服务商。

当前 Agent 闭环使用 `Language` 生成结构化编辑计划，编辑通过 `Workspace::control` 执行；节目帧渲染仍走 Agent Bridge。下一步把渲染帧作为 `MultimodalRequest.media` 送入审核模型，再把审核意见转换为同一套 `ControlEdit`。

视频和多模态请求已经完成类型、模型发现、路由和任务生命周期接入。对应供应商适配器完成前，执行器返回明确的 `Invalid`，不会创建虚假的媒体结果。
