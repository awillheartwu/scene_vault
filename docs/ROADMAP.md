# 开发路线图

路线图只保留当前完成面和仍需推进的事项；实现细节由架构、数据模型和专题文档维护。

## M1：数据层

已完成 SQLite 初始化与迁移、Project/Asset/Tag 基础模型、项目创建和列表。

- [ ] Project 更新、删除与详情
- [ ] Vue Project API 封装和真实数据接入

## M2：通用素材索引

- [ ] Library Root 管理
- [ ] 可取消的增量扫描、MIME/类型识别和进度
- [ ] 通用素材缩略图缓存
- [ ] 文件缺失检测与重新定位
- [ ] 标签和合集 CRUD

## M3：Capture Session

已完成多来源目录、会话基线、截图发现和导入、三分类、人工角色标记、串行处理、
可配置命名、四目录可靠归档、失败恢复、组合/拆分快捷键窗口和项目 Markdown 笔记。

- [ ] Windows 原生端到端验收：真实截图、AI 处理、NAS 断线恢复、重启恢复
- [ ] Python Runtime、视觉依赖、模型和字体的 Windows Sidecar 打包

## M4：创作资料

- [ ] 世界观、角色、场景和笔记模型
- [ ] 项目内关联与反向链接
- [ ] 搜索和筛选
- [ ] 长评编写（等待用户提供“长评维度”）

## M5：可选 AI

已完成 Python Provider 骨架、Rust 进程管理、YuNet 单图视觉处理、SFace / ArcFace
识别器、模型身份隔离、Face Bank 建议与闭集校验、样本质量门槛、样本库重建和正式
benchmark。ArcFace 已接入，权重由用户自备；当前模型结论见
[BENCHMARK.md](BENCHMARK.md)。

- [x] 建立一次性 Python 调用基线：请求关联 ID、Rust 调用耗时、Python 分阶段耗时和
  `vision.request` 日志；约束见
  [常驻 Python 视觉 Worker 决策](decisions/2026-08-11-persistent-python-worker.md)
- [x] 新增 JSONL 常驻 Python Worker、YuNet / SFace / ArcFace 模型缓存和逐请求隔离
- [x] Rust Worker Manager：惰性启动、串行调用、超时/崩溃恢复、设置变更重启与
  一次性模式降级
- [x] Windows 相同图片与模型的 one-shot / worker A/B 性能、内存和恢复验收；结果见
  [BENCHMARK.md](BENCHMARK.md#常驻-worker-性能与稳定性2026-08-11)
- [ ] 用更多真实使用数据校准 ArcFace 闭集校验带宽和建议阈值
- [ ] Face Bank 每角色 1/3/5 样本的覆盖率实验
- [ ] OCR Provider
- [ ] 图片描述与自动标签 Provider
- [ ] Embedding 任务队列、向量索引和语义搜索
- [ ] 在上述能力稳定后评估 RAG

多人脸数据层已经预留；多人逐脸确认和多角色自动标注在真实需求出现前不开发。

## M6：可靠性与交付

- [ ] 数据库备份与恢复的产品入口
- [ ] 索引重建
- [ ] Windows 安装包验证
- [ ] macOS / Linux 可行性与打包验证
- [ ] 大型素材库性能测试

## M7：项目人物工作台

已完成角色总览、人物截图网格、单图详情、AI 建议确认/拒绝、角色/分类改判、重新处理、
可靠重归档、重命名 UI、Face Bank 样本管理和历史分类视图。

- [x] 接入角色合并 UI
- [x] 接入代表头像 UI
- [ ] Windows 实机验收：
  - 确认建议会实际改判角色、重新处理并替换 NAS 归档；
  - 改判分类会清除不再成立的角色和人脸样本；
  - 重命名、合并、代表头像操作符合项目约束；
  - SFace / ArcFace 切换、兼容状态和样本库重建正确。

完整 Windows 验收清单见 [Capture Session 工作流](CAPTURE_WORKFLOW.md)。

## M8：工程化与体验打磨（下一阶段）

- [x] UI / UX 自检优化：完成自适应应用框架、核心工作流宽窄布局、设置分类导航、
  对话框焦点管理和快捷窗口可读性统一；长期规则见
  [桌面界面自适应与交互规范](decisions/2026-08-09-adaptive-desktop-ui.md)
- [ ] 项目工程化自检：拆分大文件、优化项目结构、优化打包内容、优化加载时间、优化内存
  与项目体积
- [x] 建立 SFace/ArcFace one-shot/worker 与当前磁盘体积初始资源基线；结果见
  [RESOURCE_USAGE.md](RESOURCE_USAGE.md)
- [ ] 增加“资源与存储”诊断页：Rust/WebView2/Python CPU、Working Set、Private Bytes，
  以及数据库、缓存、日志、模型、字体和运行时体积
- [ ] 为已完成且成功归档的 `capture-output` 中间产物建立安全清理闭环
- [x] 应用内自定义右键菜单：通用 ContextMenu 组件（Teleport + 命中判定），第一批接入
  Home 项目卡片、History 条目、Capture 截图项；危险操作复用确认对话框；需要新增后端
  命令的删除类操作暂缓；约束见
  [自定义右键菜单决策](decisions/2026-08-10-custom-context-menu.md)
- [ ] 特征向量存储优化（JSON → BLOB），控制数据库体积（全量约 10 万级截图时可达
  400–600MB，影响备份、checkpoint 与启动）
- [x] GitHub / Forgejo 远程双重推送流程
- [x] 日志清理与可观测性：结构化本地日志、轮转保留策略、历史页调试日志查看与诊断复制；
  约束见 [本地日志与诊断信息决策](decisions/2026-08-09-local-logging-and-diagnostics.md)
- [ ] 日志覆盖扩展：LogRecord v2 关联字段、Capture/归档/笔记主链路、Python `SVLOG`、
  Vue 安全事件桥和按 operation/request/capture ID 查询
- [ ] 测试体系大更新：全量调整与对齐
- [ ] README 与项目内文档编写完善（详尽版）
