# 常驻 Python 视觉 Worker 决策

日期：2026-08-11

## 背景

当前 Rust 每处理一张图片都会启动一次 `python -m scene_vault_ai request`。Python
进程读取一个 JSON 请求、初始化视觉处理器与模型、返回一个 JSON 响应后退出。这个边界
简单且容易恢复，但重复支付了解释器启动、模块导入和模型加载成本。

视觉能力目前由 OpenCV 的 YuNet、SFace 与 `cv2.dnn` ArcFace 实现。将相同 ONNX
模型改由 Rust 调用，并不能自然消除模型推理成本；在没有真实基准前，不以“Python 比
Rust 慢若干倍”作为迁移依据。

## 决策

- 保持 Vue、Rust Core 与可选 Python AI Engine 的混合架构。Rust 继续负责文件、
  SQLite、任务调度、失败恢复和 NAS 归档；Python 不扫描目录、不访问主数据库，也不
  直接写入 NAS。
- `capture_items` 状态机继续作为截图识别任务的唯一事实来源。当前不新增通用
  `ai_tasks` / `ai_jobs` 表；只有 OCR、Embedding、Caption 等独立异步任务出现真实
  持久化需求时再设计通用队列。
- 先保留一次性 `request` 模式并记录基线。Rust 为每次处理生成稳定 `requestId`，记录
  进程启动、Rust 总耗时；Python 返回处理器初始化、读图、检测、特征、标注、裁剪、
  写盘及总耗时。日志使用 `vision.request`，不写图片路径或特征向量。
- 下一阶段新增 `worker` 命令，以 stdin/stdout 上的逐行 JSON（JSONL）处理多个请求。
  stdout 只允许协议响应；进度和非协议输出继续走 stderr，由 Rust 归入
  `vision.python`。暂不引入 HTTP、随机端口或 gRPC。
- Worker 内按模型路径与识别器类型缓存 YuNet / SFace / ArcFace 实例。Rust 保持
  single-flight 串行调用，与现有“一张一个”的识别约束一致。
- Rust 后续新增单一 Worker Manager：惰性启动、健康握手、请求超时、崩溃后重启、
  设置指纹变化后重启和应用退出时关闭。请求失败不得丢失 `capture_items` 的恢复能力。
- 一次性模式保留为兼容、诊断和自动降级路径，直到 Windows 实机证明常驻模式稳定。

## 分阶段实施

1. **基线与协议可观测性**：加入可选 `requestId`、Python 阶段计时、Rust 调用计时与
   兼容反序列化；处理行为不变。
2. **Python Worker 与模型缓存**：新增 JSONL 循环、逐请求错误隔离和可测试的缓存；
   保留现有 `request` 命令。
3. **Rust 生命周期管理**：接入 Worker Manager、超时/崩溃恢复和设置变更重启；仍由
   Rust 串行调度。
4. **Windows A/B 验收**：在相同图片、模型和设置下比较一次性与常驻模式，分别观察
   首张、后续图片、失败恢复和内存稳定性，再决定默认模式与打包策略。

## 非目标

- 当前不把视觉管线迁移到 Rust `ort`。
- 当前不扩展 OCR、RAG 或通用素材 AI 功能。
- 当前不改变自动识别触发条件、串行顺序、Face Bank 数据或归档流程。

## 结果

常驻 Worker 的收益将由可复现的阶段数据验证，而不是依赖语言层面的估算。架构仍保留
Python AI 生态的迭代能力，同时把产品状态、恢复和交付控制留在 Rust Core。
