# AI 服务接入（todo_ai / todo_production）

更新：2026-10-17。虎妞（对话、生图）与火山（配音）由 Rust 直连，AI 能力是 app 的一部分，不依赖任何外部工具目录；用户在 AI 设置里填写服务地址与 key 即可使用自己的账号或兼容网关。

## 分层

| crate | 职责 | 依赖 |
| --- | --- | --- |
| `todo_production` | 纯领域：`TaskSpec`（要向哪个服务商发什么请求）与任务指纹 `TaskKey`；后续放分段、排期、入轨对账规则 | serde、serde_json、sha2 |
| `todo_ai` | 基础设施：配置、密钥、HTTP、错误分类、重试、熔断、任务日志、产物仓库、服务商适配、后台执行器 | todo_production、tokio、reqwest(native-tls)、image… |
| `todo_process` | 子进程监督（FFmpeg、日后的扩展烘焙器共用） | thiserror |

application 只依赖 `todo_production` 的类型，不依赖 `todo_ai`；app 负责装配。`crates/todo_application/tests/dependency_boundaries.rs` 是这些规则的准绳。

## 配置与密钥

- 非密钥设置在 app 目录的 `config/ai.json`（AI 设置面板写入，模板 `config/ai.example.json`）：虎妞 / 火山的服务地址、模型、音色、并发与代理；字段都可省略，未知字段拒绝。当前默认值：对话 `geminiflash3.1`、生图 `gpt-image-2`、配音 `seed-tts-2.0` + 咪仔。
- 密钥只存在 Windows 凭据管理器（`todo-editor/huniu.api_key`、`todo-editor/volc.api_key`），由设置面板写入；环境变量 `TODO_HUNIU_API_KEY` / `TODO_VOLC_API_KEY` 可覆盖（开发、CI）。密钥不进工程、配置、日志或安装包；`Secret` 的 Debug 输出为 `***`。
- 自检（不花钱）：`cargo run -p todo_ai --example ai_check`。开发者一次性导入旧 `.env`：`cargo run -p todo_ai --example import_env -- <路径>`（app 运行时从不读取这类文件）。

### 设置里的“测试连接”

点击后先保存当前表单并让 `ProductionHost` 重新读取配置，同时发起两个独立探测：一个是免费的 `GET /v1/models`，用来返回并分类当前可用的文本、图片、视频模型；另一个是带时间 nonce 的最小虎妞对话请求，用来确认当前模型和 Key 真能完成调用。nonce 保证每次点击都发起新请求，不会因为任务 journal 命中上一次成功结果。模型列表会填回设置弹窗，当前对话模型或生图模型不可用时自动选取同能力的第一个模型。它不调用生图或配音，也不写入成片的 `ProductionOutput`；成功、鉴权失败、缺 key 和网络错误都会作为设置提示返回。30 秒没有最终结果会主动取消并提示超时。文本探测是真实的对话请求，可能消耗少量对话额度。

## 执行模型

`AiService` 在独立线程上跑单线程 tokio 运行时，通过 channel 与 app 通信（与现有 workers 一致）。

1. **任务即指纹**：`TaskSpec::key()` = SHA-256（版本号 + 规范 JSON）。同一请求只有一条日志记录；已完成的直接返回结果，不再请求、不再计费。
2. **先记账再联网**：发送前把状态写为 `Submitting`；生图拿到结果 URL 后立刻写为 `Downloading`。日志是 `cache/ai/tasks/<key>.json`，临时文件 + fsync + 原子改名。
3. **重启恢复**：`Submitting` → 免费请求重跑；付费请求消耗一次「结果未知重试预算」（默认生图 2 次），超出记为 `Ambiguous`（可能已扣费，需人工一键重试）。`Downloading` 只续下载（HTTP Range）。
4. **错误分类**（`net/classify.rs`）：连接 / TLS 阶段失败 = 肯定没送达，可重试；发出后的超时 / 断连对付费请求是 `Ambiguous`；401/403 → 暂停该服务商等待新密钥；402 / 余额 → 暂停等待充值；400 类内容审核 → `ContentPolicy`；其余 4xx → `Invalid` 立即失败。
5. **重试**：指数退避 + 完全抖动（1 s → 60 s），遵守 `Retry-After`；截止时间：对话 / 配音 10 分钟，生图 20 分钟。
6. **熔断**：同一服务商 60 s 内 5 次网络失败 → 30 s 后放一个探测请求；期间所有任务显示「等待网络」而不是失败，恢复后自动继续。
7. **限流**：对话 / 生图 / 配音各自的并发上限；生图提交间隔 ≥ 1/2 s，在途 ≤ 40（密匙实测：突发提交会排队）。
8. **产物**：`cache/ai/blobs/<sha前2位>/<sha256>.<ext>`；先写 `.parts/<task>.part`，图片解码校验 / WAV 头校验通过才发布，截断文件永远不会被当成结果。

## 服务商契约（2026-10-17 线上探测确认）

| 能力 | 请求 | 结果 |
| --- | --- | --- |
| 火山 TTS | `POST {volc.baseUrl}/api/v3/tts/unidirectional`（默认 `https://openspeech.bytedance.com`），头 `X-Api-Key`、`X-Api-Resource-Id`（克隆音 `seed-icl-2.0`）；`audio_params.format=pcm`、`enable_subtitle=true` | NDJSON：`data` 为 base64 PCM，`sentence.words[{word,startTime,endTime}]`（秒），`code=20000000` 结束；本地写 WAV 头。实测 pcm 模式同样返回逐字时间戳 |
| 虎妞对话 | `POST /v1/chat/completions`，`stream=true`，可选 `response_format={"type":"json_object"}` | SSE `choices[0].delta.content`；网关不流式时兼容整包 JSON。`gpt-6-astra` 支持 JSON 模式 |
| 虎妞生图 | `POST /v1/images/generations`：`model,prompt,n=1,size="WxH",quality?,background="transparent"+output_format="png"?,strategy?` | `data[0].url` 或 `b64_json`；URL 与网关同域才带鉴权头。读超时 300 s 不重发 |

线上探测（花费不足一分钱 / 次）：`cargo run -p todo_ai --example ai_probe -- tts|chat|chat-json|image`，产物在 `target/ai-probe/`。

## 验证

```powershell
cargo test -p todo_ai -p todo_production -p todo_process
cargo test -p todo_ai -- --ignored   # 真实凭据管理器读写
```

`crates/todo_ai/tests/service.rs` 用本地故障注入服务器覆盖：缓存命中零请求、生图只提交一次且下载断点续传、付费请求按预算停止并标记可能扣费、502/503 熔断后自愈、401 暂停后换密钥继续、重启后只续下载、配音流式解析。
