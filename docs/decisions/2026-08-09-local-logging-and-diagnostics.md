# 本地日志与诊断信息决策

日期：2026-08-09

## 背景

桌面应用的后台目录监听、快捷键窗口、可选视觉进程和文件归档会受到 Windows 环境、
网络目录与本地依赖影响。仅写终端标准错误既不方便普通用户查看，也无法在应用重启后
保留故障上下文。

## 决策

- Rust Service 使用统一结构化记录：UTC 时间、`debug` / `info` / `warn` / `error`、
  稳定模块名和单行消息。业务线程通过容量 1024 的有界队列提交记录，独立线程写盘；
  队列满时允许丢弃单条日志，不能反向阻塞截图处理。
- 日志只写入 Tauri 应用日志目录，不写入项目、截图来源目录、归档目录或 SQLite。
- 当前文件达到 5 MiB 后轮转；归档保留 14 天，最多保留 20 个。启动、轮转和 History
  中的“按策略清理”都会执行同一清理规则。
- History 的“调试日志”入口支持时间范围、级别和模块筛选。单次最多返回 1000 条，
  界面默认请求最新 500 条，防止大日志阻塞界面。
- “复制诊断信息”生成纯文本摘要，包含应用版本、系统与架构、日志策略，以及最近
  24 小时的警告和错误。它不读取数据库业务记录或图片内容。
- Python 标准输出继续只承载协议 JSON；标准错误中的进度用于事件，其他内容由 Rust
  归入 `vision.python`，避免形成第二套日志目录和清理规则。
- 常驻视觉进程生命周期记录在 `vision.worker`，单次请求及 one-shot 降级耗时记录在
  `vision.request`；两者通过 `request_id` 关联，不记录图片路径和特征向量。

## 结果

Windows 问题可以直接从应用内定位并复制给维护者，同时日志体积有明确上限。日志仍可能
包含底层错误返回的本地路径，因此用户应在发送诊断摘要前确认接收方。

## 2026-08-11 覆盖审计

日志存储、轮转、查询、清理和 History UI 已完成，但“存在统一日志系统”不等于“业务
操作已接入”。当前覆盖情况如下：

| 层 | 已覆盖 | 主要缺口 |
|---|---|---|
| Rust | app startup、快捷键/弹窗、慢 discovery、视觉 worker/request | Capture 状态机、归档、项目/角色、Face Bank、设置、笔记同步、缩略图 |
| Python | `SVPROGRESS`、未预期异常、stderr 转交 Rust | 模型加载/cache hit/miss/reload/evict、请求开始/结束、结构化等级和事件 |
| Vue | 少量 `console.error`、toast、页面 errorMessage | 不持久化、invoke 无统一耗时/失败日志、静默 catch 缺少上下文 |

当前 `LogRecord` 只有 `timestamp/level/module/message`。视觉代码把 `request_id` 和耗时
拼入 message，能人工读取，但不能稳定按请求、截图、会话和业务事件筛选。

常驻 Python worker 进一步要求区分三个生命周期：

```text
worker 生命周期
  └─ 模型缓存生命周期
      └─ request_id 对应的一次处理
```

不能继续把“一次 Python 进程”当成“一次图片处理”。

## 目标结构

Rust 继续作为唯一日志文件所有者。Python 和 Vue 不建立第二套日志目录：

```text
Vue ── record_client_event ──┐
                             ├─ Rust log_service ── JSONL / History
Python stderr ── SVLOG ──────┘
```

### LogRecord v2

在保留旧四字段的基础上增加可选字段，并为旧 JSONL 提供 serde 默认值：

```text
event
operation_id
request_id
project_id
session_id
capture_item_id
note_id
attempt
duration_ms
outcome
worker_mode
error_code
```

关联链为：

```text
operation_id → session_id → capture_item_id → request_id
```

业务 ID 用结构化字段保存；`message` 只保留简短、适合人读的摘要。

### 等级规则

- `debug`：高频成功、cache hit、正常 polling 汇总、性能阶段信息；
- `info`：用户发起的重要操作成功、worker 启停、会话启停、归档完成；
- `warn`：降级、重试、慢操作、源目录/NAS 暂时不可用、可恢复冲突；
- `error`：操作最终失败、数据/协议损坏、重试耗尽、后台循环异常退出。

不逐条持久化 progress tick，也不记录每次空轮询。

### Python SVLOG

Python stderr 增加与 `SVPROGRESS` 分离的单行 JSON 前缀：

```text
SVLOG {"level":"info","module":"vision.model","event":"model_loaded",
       "requestId":"...","durationMs":123.4}
```

Rust 解析等级和字段后写入统一日志；无法解析的第三方 stderr 仍以截断后的
`vision.python/python_stderr` 保存。建议事件：

- `model_load_started/model_loaded/model_load_failed`；
- `model_cache_hit/model_cache_miss/model_reloaded/model_evicted`；
- `request_started/request_succeeded/request_failed`；
- `unexpected_exception`。

worker 启动日志保留 PID；请求日志必须使用 request ID，模型事件记录 model ID/version，
不记录模型绝对路径。

### Vue 客户端事件

增加受限的 `record_client_event` Command 和前端封装，只允许稳定 module/event、等级、
operation ID、duration 和已脱敏摘要。首批替换：

- `PopupClassify` 加载/事件桥失败；
- `PopupWorkbench` 操作失败；
- `PopupNote` Markdown 与同步失败；
- Capture/History/Workbench/Home/Settings 的 invoke 和 reveal 失败；
- `window.unhandledrejection` 与 `window.error` 的安全摘要。

toast 继续负责即时反馈，inline error 继续负责页面状态；持久化日志不能替代用户反馈，
也不能因为 toast 和 API wrapper 同时记录而产生重复事件。

## 业务事件优先级

### P0：故障恢复主链路

- `capture.worker`：claimed、processing_started/succeeded/failed、degraded、retry、recovered；
- `capture.archive`：started、copy_failed、hash_verified、atomic_rename、completed、retry；
- `capture.discovery`：discovered、imported、duplicate_skipped、source_unavailable；
- `notes.sync`：failed、conflict_backup、retry、retry_exhausted；
- `vision.worker/request/python`：timeout、crash、fallback、模型缓存和最终结果。

重点修复当前后台吞错：notes 后台循环和 capture worker 的最终错误必须进入持久化日志。

### P1：用户高影响操作

- project：create、rename、delete、destination_changed、cover_changed；
- character：create、rename、merge、avatar_changed；
- recognition：suggestion accepted/rejected、sample status/flag、feature refresh、bank rebuild；
- settings：配置类别和变更结果，只记录字段名，不记录 Python/模型/字体绝对路径；
- frontend：invoke、popup、reveal、event bridge 和未处理异常。

业务成功/失败应在 Service 的事务提交或最终错误处记录；Command 保持薄，避免一项操作在
Command 和 Service 重复写两条日志。

### P2：诊断体验

- History 支持 event、operation/request/capture ID 和 outcome 过滤；
- 增加“单次操作时间线”，串联 Vue → Command → Service → Python → archive；
- 增加日志队列丢弃计数，而不只向 stderr 打印；
- 增加慢扫描、慢推理、慢归档阈值；
- 诊断摘要按关联 ID 输出，但继续限制条数和隐私字段。

## 脱敏规则

默认禁止写入：

- 截图、归档、Python、模型和字体完整路径；
- 项目名、角色名、笔记正文；
- 人脸框、embedding、图片内容；
- 未处理的完整 traceback。

允许写入：业务 UUID、模型 ID/version、扩展名、文件大小、目录类型、attempt、duration、
稳定错误码和截断摘要。Windows drive/UNC/`\\?\` 路径必须通过统一 sanitizer 处理，不能
只依赖每个调用方自觉。

## 实施顺序

1. **Schema 与兼容**：LogRecord v2、builder/context、安全 sanitizer、旧 JSONL 查询兼容。
2. **P0 Rust 覆盖**：Capture worker、archive、discovery、recovery、notes background；补
   状态转换、retry、NAS 和重启测试。
3. **Python SVLOG**：请求上下文、模型缓存事件、Rust 解析和等级映射；验证 stdout 不被
   污染、stderr 截断和 crash/fallback 关联。
4. **Vue bridge**：受限 Command、统一 invoke wrapper、替换 console/静默 catch；保留
   toast/inline error，加入去重规则。
5. **P1 业务覆盖**：项目、角色、识别、设置和清理操作。
6. **查询与时间线**：字段筛选、关联 ID 复制、单次操作视图和诊断摘要。
7. **Windows 验收**：worker 复用/崩溃/fallback、NAS 断线、重启恢复、日志轮转、中文与
   UNC 路径脱敏。

## 必要测试

- Rust：v1/v2 日志兼容、字段过滤、并发写入、轮转、队列丢弃、路径脱敏；
- Rust 业务：每个 Capture/归档状态转换的 event、attempt、failure stage 与关联 ID；
- Python：成功、协议错误、模型错误、异常、cache hit/miss/reload 均保持 request ID；
- Vue：invoke 失败同时有用户反馈和一条持久化事件，不重复、不包含敏感数据；
- E2E：从一次分类操作追踪到 worker、归档、重试或完成的完整时间线。

## 2026-08-11 实施结果

本轮已完成日志覆盖扩展的代码闭环：

- `LogRecord` 已增加可选关联字段，旧四字段 JSONL 仍可反序列化；查询支持 event、outcome
  和跨 operation/request/project/session/capture/note 的关联 ID；状态会报告本次运行的队列
  丢弃计数。
- Capture 会话、发现、导入、分类、处理、归档与重试，笔记同步，项目、角色、Face Bank、
  识别复核和设置变更已接入结构化业务事件；不记录名称、正文或文件路径。
- Python request 与模型缓存通过 stderr `SVLOG` 输出，worker 和 one-shot 均由 Rust 解析并
  写入同一 JSONL；stdout 的协议响应不变。
- Vue 增加受限事件 Command、全局 error/unhandledrejection 捕获和统一 Tauri invoke 失败
  记录；日志桥失败不会递归或影响原操作的错误反馈。
- History 日志面板已支持模块、事件、结果与关联 ID 组合筛选，并展示 request/capture ID、
  outcome 和 duration。

自动验证已覆盖 Rust v1/v2 兼容与字段筛选、Python SVLOG/缓存事件、Vue 筛选和既有业务
回归。仍需在 Windows 实机验收 worker 崩溃与 one-shot fallback、NAS 断线、日志轮转、
中文/UNC 路径脱敏以及资源页实时 CPU/内存读数；“单次操作时间线”和可配置慢操作阈值
属于后续诊断体验增强，不阻塞本轮日志覆盖任务完成。

## 2026-08-11 清理控制与分页补充

- 日志策略保存在 SQLite `settings/logging.policy`，设置页可调整保留天数（1–365）、单个
  JSONL 文件上限（1–100 MiB）、归档数量（1–200）以及是否在启动和轮转时自动清理。
  保存后立即更新运行中的 LogStore；关闭自动清理不会禁用手动“按策略清理”。
- 日志查询增加 `offset/limit/nextOffset/hasMore`。History 首次只渲染最新 100 条，滚动
  接近底部时继续加载；首次查询固定 `since/until` 时间窗，避免加载下一页时被新日志
  整体推移。手动刷新和修改筛选会建立新的时间窗。
- 日志页不订阅实时事件、不设置轮询定时器。切换回截图历史时组件卸载，
  `IntersectionObserver` 同时断开；后台仍只保留原有 Rust 异步日志写入线程。
- 进一步补充 startup recovery、后台 source scan 失败、归档 copy verification/atomic
  rename、笔记外部冲突备份、Face Sample 状态/标记、验证/建议刷新、缩略图生成失败与
  缓存淘汰事件。成功高频读取和空轮询继续不记录。
