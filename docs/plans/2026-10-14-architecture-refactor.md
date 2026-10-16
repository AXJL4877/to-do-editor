# 架构拆分与基础编辑完善实施计划

**Goal:** 按已确认的架构审查方案实际拆分职责，保留旧工程与剪辑功能，补全预览位置拖动、片段两端裁切和统一吸附。

**Architecture:** core 负责完整领域命令；application 负责工程会话、历史、保存、播放与后台任务状态；interaction 负责输入与显示投影；UI 负责绘制。storage 负责本地工程文档，media 和 preview 负责外部媒体能力，app 只装配和运行。

**Tech Stack:** Rust workspace、egui/eframe 0.31、Node 媒体服务、Windows D3D11 原生预览。

**Spec:** `docs/reports/2026-10-14-architecture-audit.md` 与用户本轮新增的预览拖动、时长手柄和吸附要求。用户已要求按该方案实施，本计划在当前会话连续执行。

## 约束

- 直接迁移当前工作区，保留已有未提交变更；源代码快照位于 `target/refactor/source-before.zip`。
- 不写入用户现有的 project.json 和媒体；测试仅使用临时目录。
- 不增加导出、特效栈或完整多轨合成等本轮之外的功能。
- 领域错误使用类型；本地化显示在 UI 投影层；窗口几何独立于工程事务。
- 预览位置使用画布宽高的归一化偏移，默认 (0,0)，缩放默认 1；旧 Clip 未带 transform 时保持原画面。
- 鼠标一次拖动对应一次撤销；Escape 取消并恢复；锁定轨道不能编辑。
- 吸附覆盖移动片段的左右边、裁切端点、素材放置、播放头与其他片段边缘；关闭吸附后只保留帧对齐。

## 实施任务

- [x] **1. 领域模型与命令。** 建立 model / edit / query / error 模块；统一关联裁切、粘贴、放置、位置变换的约束，保留旧 JSON 兼容。测试锁定、重叠、来源边界、图片延时、粘贴延长、变换保存。
- [x] **2. 本地存储。** 新增 todo_storage，提供 list/create/load/save/delete/import_json。使用临时文件与原子替换；验证版本和工程约束；删除工程移入回收目录。测试损坏文档、旧文档、重复工程与原子保存。
- [x] **3. 原生预览。** todo_preview / native 增加 transform 协议和归一化鼠标事件；位置 / 缩放改变只更新呈现，不重新解码。测试协议解析、边界与已有解码回归。
- [x] **4. 应用层。** 新增 todo_application，迁移会话、历史、剪贴板、播放协调；定义 EditorCommand、AppAction、AppEvent、Effect；任务带 operation/session ID。测试旧任务失效、保存失败、保存快照与关闭流程。
- [x] **5. 输入层。** interaction 不修改 Project，输出语义命令；保留选择、框选、焦点和手势；统一吸附函数、拖拽事务、预览画布位置拖动；旧交互测试迁移并补双端吸附。
- [x] **6. UI 与运行壳。** UI 只消费 ViewModel、发送输入；补裁切手柄、时长数值、位置 / 缩放 / 重置及吸附引导；拆分 app 的 runtime 和 adapters；删除双路预览布局及重复动作入口。
- [x] **7. 媒体服务与旧代码。** 保存队列与资源任务隔离；失败退避；Node 仅提供媒体接口；移除不再使用的 TS 领域实现与工程 HTTP 客户端；保持启动脚本健康检查。
- [x] **8. 验证与交付。** 增加依赖边界测试，运行 Rust / native / 后端检查，核对用户工程哈希；出具 `docs/architecture/responsibilities.md` 简明文件夹分工。

## 验证重点

- 老工程没有 transform / kind 字段时仍能打开。
- UI、快捷键和原生预览拖拽走相同的应用命令与历史事务。
- 保存与媒体生成互不串行阻塞，回执不会污染新会话。
- 拖动裁切不能越过来源或相邻片段；静态图片可延长。
- 调整面板尺寸不写入工程内容，不产生历史记录。

实施进度记入 `target/refactor/progress.md`；每个任务完成后运行对应检查，最后执行完整回归。
