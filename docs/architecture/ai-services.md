# AI 服务接入（todo_ai / todo_production）

更新：2026-10-18。AI 由 Rust 直连，模块通过 `AiRouter` 按能力调用，供应商只存在于 `todo_ai` 的适配器层。当前公开能力为语言、图像、视频、多模态、音频；用户在 AI 设置里绑定每类能力的模型即可使用自己的账号或兼容网关。

## 分层

| crate | 职责 | 依赖 |
| --- | --- | --- |
| `todo_production` | 纯领域：能力请求 `TaskRequest`、任务快照 `TaskSpec` 与任务指纹 `TaskKey`；后续放分段、排期、入轨对账规则 | serde、serde_json、sha2 |
| `todo_ai` | `AiRouter`、能力绑定、配置、密钥、HTTP、错误分类、重试、熔断、任务日志、产物仓库、供应商适配、后台执行器 | todo_production、tokio、reqwest(native-tls)、image… |
| `todo_process` | 子进程监督（FFmpeg、日后的扩展烘焙器共用） | thiserror |

application 只依赖 `todo_production` 的类型，不依赖 `todo_ai`；app 负责装配。`crates/todo_application/tests/dependency_boundaries.rs` 是这些规则的准绳。

## 配置与密钥

- 非密钥设置在 app 目录的 `config/ai.json`（AI 设置面板写入，模板 `config/ai.example.json`）：能力模型、虎妞渠道、适配器地址、音色、并发与代理；字段都可省略，未知字段拒绝。当前默认值：虎妞渠道 `XUE`、语言 `geminiflash3.1`、图像 `gpt-image-2`、视频 `sora-2`、多模态 `gemini-3.8-flash`、音频 `seed-tts-2.0` + 咪仔。
- 密钥只存在 Windows 凭据管理器（`todo-editor/huniu.api_key`、`todo-editor/volc.api_key`），由设置面板写入；环境变量 `TODO_HUNIU_API_KEY` / `TODO_VOLC_API_KEY` 可覆盖（开发、CI）。密钥不进工程、配置、日志或安装包；`Secret` 的 Debug 输出为 `***`。
- 自检（不花钱）：`cargo run -p todo_ai --example ai_check`。开发者一次性导入旧 `.env`：`cargo run -p todo_ai --example import_env -- <路径>`（app 运行时从不读取这类文件）。

### AI 设置里的模型列表与测试连接

- **模型列表（免费，自动）**：打开 AI 设置或保存后，`ProductionHost::list_models` 通过语言能力绑定发起 `GET /v1/models`，按语言、图像、视频和多模态分类后填回弹窗；音频供应商没有统一目录时保留手动模型字段。
- **测试连接（计费，手动）**：只有用户点击「测试连接」才会先保存表单，再由 `ProductionHost::test_connection` 发送一条带时间 nonce 的最小对话请求（nonce 保证不会命中任务 journal 的旧结果）。结果以 `HostReport::Connection` 直接显示在弹窗里（成功 / Key 无效 / 余额不足 / 网络问题），30 秒无结果视为超时并取消。它不调用生图或配音，也不写入成片的 `ProductionOutput`。
- 错误类别统一用 `todo_ai` 的类名（`auth`、`billing`、`transient` …），UI 只显示翻译后的原因。

## 执行模型

`AiRouter` 是应用和扩展的唯一 AI 入口，内部把能力绑定成供应商适配请求；`AiService` 在独立线程上跑单线程 tokio 运行时，通过 channel 与 app 通信（与现有 workers 一致）。

1. **任务即指纹**：`TaskSpec::key()` = SHA-256（版本号 + 规范 JSON）。同一请求只有一条日志记录；已完成的直接返回结果，不再请求、不再计费。
2. **先记账再联网**：发送前把状态写为 `Submitting`；生图拿到结果 URL 后立刻写为 `Downloading`。日志是 `cache/ai/tasks/<key>.json`，临时文件 + fsync + 原子改名。
3. **重启恢复**：`Submitting` → 免费请求重跑；付费请求消耗一次「结果未知重试预算」（默认生图 2 次），超出记为 `Ambiguous`（可能已扣费，需人工一键重试）。`Downloading` 只续下载（HTTP Range）。
4. **错误分类**（`net/classify.rs`）：连接 / TLS 阶段失败 = 肯定没送达，可重试；发出后的超时 / 断连对付费请求是 `Ambiguous`；401/403 → 暂停对应适配器等待新密钥；402 / 余额 → 暂停等待充值；400 类内容审核 → `ContentPolicy`；其余 4xx → `Invalid` 立即失败。
5. **重试**：指数退避 + 完全抖动（1 s → 60 s），遵守 `Retry-After`；截止时间：对话 / 配音 10 分钟，生图 20 分钟。
   **生图超时重提**（`service.rs::submit_image_hedged`）：一张图发出后 `limits.imageResubmitSec`（默认 300 s）还没回来，就再发一次，**之前的请求不取消**，哪个先回来用哪个，其余丢弃。每次重提都算一次付费重试，总共最多 1 + `imagePaidRetries`（默认 3 次）。实测 97 张图中位数 48–86 s、P90 175–245 s、超过 300 s 的约 3%，所以重提很少触发。
6. **熔断**：同一服务商 60 s 内 5 次网络失败 → 30 s 后放一个探测请求；期间所有任务显示「等待网络」而不是失败，恢复后自动继续。
7. **限流**：对话 / 生图 / 配音各自的并发上限；生图提交间隔 ≥ 1/2 s，在途 ≤ 40（密匙实测：突发提交会排队）。
8. **产物**：`cache/ai/blobs/<sha前2位>/<sha256>.<ext>`；先写 `.parts/<task>.part`，图片解码校验 / WAV 头校验通过才发布，截断文件永远不会被当成结果。

## 多个程序同时运行（编辑器 + 命令行出片）

两边各自启动一个 `AiService`，但共用 `cache/ai/tasks`，共用 `data/`。

- **任务日志**：提交和重试前先从磁盘重读这条记录（`Journal::refresh`），别的程序做完的结果直接复用。真正执行时拿这条请求的独占锁（`Journal::claim`，即 `cache/ai/tasks/.claims/<key>.lock`，操作系统文件锁，进程退出时自动释放）。拿不到锁的程序显示「另一个程序正在生成，等它的结果」（`WaitReason::Elsewhere`），每秒从磁盘重读一次，等对方写出结果，不会再付一次钱。
- **工程**：一个工程同一时间只归一个程序（`ProjectStore::lock`，即 `data/.locks/<hash>.lock`，持有者的进程号写在旁边的 `.owner` 文件里）。编辑器打开、新建、导入工程时拿锁，切换工程时换锁；被别的程序占着的工程打不开，会提示是哪个程序占着。`save` 和 `delete` 遇到别的进程持有的锁一律拒绝。`produce`、`direct` 命令行工具开始前也拿锁，`--fresh` 删不掉别人开着的工程时直接退出。

2026-10-19 的事故就是因为缺了这两条：编辑器开着 v9-herd，命令行用 `--fresh` 把工程移进了回收站并重建。编辑器打开工程时会读取并继续执行 `productions/` 下所有生成任务，于是收进了命令行那一份的 28 个素材，自己原来引用的素材反而找不到了。编辑器的任务日志还停留在启动时那一刻，继续执行没做完的任务时又重新请求了文案和配音，多出了不知名的配音，也多付了钱。

## 服务商契约（2026-10-17 线上探测确认）

| 适配器能力 | 请求 | 结果 |
| --- | --- | --- |
| 火山 TTS | `POST {volc.baseUrl}/api/v3/tts/unidirectional`（默认 `https://openspeech.bytedance.com`），头 `X-Api-Key`、`X-Api-Resource-Id`（克隆音 `seed-icl-2.0`）；`audio_params.format=pcm`、`enable_subtitle=true` | NDJSON：`data` 为 base64 PCM，`sentence.words[{word,startTime,endTime}]`（秒），`code=20000000` 结束；本地写 WAV 头。实测 pcm 模式同样返回逐字时间戳 |
| 虎妞对话 | `POST /v1/chat/completions`，`stream=true`，可选 `response_format={"type":"json_object"}` | SSE `choices[0].delta.content`；网关不流式时兼容整包 JSON。`gpt-6-astra` 支持 JSON 模式 |
| 虎妞生图 | `POST /v1/images/generations`：`model,prompt,n=1,size="WxH",quality?,background="transparent"+output_format="png"?,strategy?` | `data[0].url` 或 `b64_json`；URL 与网关同域才带鉴权头。读超时 300 s 不重发 |

视频和多模态请求已经进入 `TaskRequest`、模型发现和路由协议；在供应商适配器接入前会返回明确的 `Invalid`，不会误报为已生成。Agent 的语言规划走 `Language`，节目帧审核接入 `Multimodal` 后仍通过同一条 `Workspace::control` 闭环执行修正。

线上探测（花费不足一分钱 / 次）：`cargo run -p todo_ai --example ai_probe -- tts|chat|chat-json|image`，产物在 `target/ai-probe/`。

## 验证

```powershell
cargo test -p todo_ai -p todo_production -p todo_process
cargo test -p todo_ai -- --ignored   # 真实凭据管理器读写
```

`crates/todo_ai/tests/service.rs` 用本地故障注入服务器覆盖：缓存命中零请求、生图只提交一次且下载断点续传、付费请求按预算停止并标记可能扣费、502/503 熔断后自愈、401 暂停后换密钥继续、重启后只续下载、配音流式解析。
