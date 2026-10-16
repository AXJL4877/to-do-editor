# 工程设置、稳定布局与素材树

本轮围绕编辑器 UI 集成四项修正，继续使用现有 Rust 分层和原生媒体工具。

## 工程设置

顶栏将分辨率、帧率、素材落轨缩放和工作区时长集中到“工程设置”弹窗。设置通过原有 Intent 和领域校验生效，数值连续编辑沿用编辑事务；点击完成会提交正在输入的时长。工作区仍允许完整片段超过设定边界。

弹窗打开时屏蔽底层编辑器操作、文件拖入和原生预览输入，并隐藏原生预览窗口，避免遮住弹窗。关闭弹窗的同一帧也屏蔽底层快捷键，Escape 不会同时清空时间轴选择。失败反馈显示在弹窗内。

## 窗口与面板

面板保存用户偏好尺寸，绘制时再按当前窗口可用区域限制。缩小窗口不改写偏好，最小化或无效首帧不初始化零尺寸偏好。面板偏好通过 egui 的持久化存储跨启动保留。

`todo_app::runtime::window_state` 单独记录最后正常窗口的尺寸和位置，忽略最小化与无效矩形；最大化、全屏保留正常窗口的还原尺寸。适配器使用 eframe/egui-winit 0.31.1 的窗口记录格式，测试直接用上游 `WindowSettings` 解码，升级框架时需一起检查。位置按物理像素保存，窗口尺寸按 egui 点保存。

窗口和面板偏好写入 eframe 的用户配置文件，Windows 默认是 `%APPDATA%\TO DO\data\app.ron`，不写入工程 JSON。分辨率、帧率、工作区时长等工程设置仍由工程保存。工程设置弹窗本身不跨启动重新打开。

## 鼠标焦点缩放

原有滚轮锚点公式已经包含鼠标坐标，但 UI 手动读取的 ScrollArea 状态 ID 与实际控件 ID 不一致，计算后的横向偏移没有应用到实际滚动区域。本轮统一二者使用的 salt，并把像素密度缓存绑定到对应滚动区。

同时移除密度变化必须大于 0.01 的判断。长工作区的像素密度较小，即使一次有效缩放的变化不到此值，也必须应用对应偏移。Alt/Ctrl+滚轮在 ScrollArea 消费事件前捕获，普通横向、纵向滚动保持原有行为。

回归测试通过真实 egui 输入、交互控制器、Workspace 和下一帧 UI 滚动状态验证鼠标时间点，覆盖连续放大/缩小、先滚动再缩放，以及 86400 秒工作区。

## 素材树与删除

素材箱继续使用统一树结构，文件夹和媒体行按层级缩进，显示展开箭头、展开/收起文件夹以及视频、音频、图片图标。图标由 `library/icons.rs` 绘制，不依赖字体符号。根目录行提供导入目的地和拖回根目录的目标。

素材箱获得焦点后，Del 删除选中项；输入文字和切换行的鼠标按下、松开帧不会触发删除。控制器同时阻止素材箱焦点下的旧时间轴删除命令。媒体与目录选中样式互斥，删除后对失效选择做同步。独立复查还用真实编辑器绘制复现了“跨帧点击松开并按 Del”会删除上一行的情况；对应回归先失败，补上松开帧保护后通过。

沿用项目素材管理语义：删除料箱将内容移到上一级；删除媒体使用非级联操作，已被时间轴引用的素材保留。磁盘源文件不删除。需要连同时间轴引用删除媒体时，使用原有明确标注的右键操作。

实现与测试分开放置：`library/{mod.rs,icons.rs,tests.rs}`、`layout/{mod.rs,tests.rs}`、`project_settings.rs` 与 `project_settings/tests.rs`、`runtime/window_state.rs`；时间轴集成回归位于 `timeline/board/tests/zoom.rs`。

## 验证

- `cargo test --workspace --quiet`：407 passed，0 failed。
- `cargo check --workspace`：通过。
- `cargo clippy --workspace --lib --bins --examples -- -D warnings`：通过。
- `cargo clippy -p todo_app -p todo_application -p todo_interaction -p todo_ui --all-targets -- -D warnings`：通过。
- 本轮修改的 Rust 文件通过 rustfmt 检查；`git diff --check` 无空白错误。
- 独立代码复查：Del 松开帧问题修正后，无剩余重要发现。

全量测试的环境说明：默认系统临时目录下的既有文字缓存测试报 Windows 拒绝访问（os error 5），单独重跑同样失败。诊断确认失败发生在临时目录创建；仅为当前测试进程将 TEMP/TMP 指向工作区的 `target/desktop-ui-test-temp` 后，同一测试及完整工作区测试通过。诊断用的临时日志改动已撤回，媒体缓存生产代码无本轮修改。

`build.ps1 -Configuration debug` 已成功重新生成 `dist/to_do/todo_app.exe`。打包结束后核对它与 `target/package/debug/todo_app.exe` 的 SHA-256 一致：`19A1BB4FA2EB77617E8CFCFDF543CAEFE17EAE334979D03F4CB0D19FFC23F454`。

本轮自动化覆盖 egui 输入/绘制和持久化格式；未通过原生桌面 UI 自动操作实际点击 Windows 最小化、还原按钮。
