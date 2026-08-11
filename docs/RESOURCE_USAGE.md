# 资源占用与存储调研

日期：2026-08-11

## 目的与口径

本文建立 Scene Vault 的性能、进程内存、CPU 和磁盘体积口径，为以下决策提供依据：

- one-shot 与常驻 worker 的长期默认模式；
- SFace 与 ArcFace 的资源代价；
- Tauri/Rust、WebView2 与 Python 的整应用预算；
- 应用内“资源与存储”诊断页；
- 缩略图、日志、处理中间产物、数据库、模型和运行时的清理边界。

当前数据是单机 Windows 11 快照，不代表最低配置或所有图片。Working Set 包含共享页，
不能直接等同于私有内存；短时峰值仍可能被轮询漏掉。后续正式验收必须在固定电源计划、
相同模型、相同图片集合和 4 个 P 核约束下重跑。

## 当前进程结构

```text
SceneVault.exe（Tauri / Rust）
├─ WebView2 browser / renderer / GPU / utility 进程
├─ Rust discovery 与 capture async worker（主进程内）
└─ Python
   ├─ scene_vault_ai worker（默认，惰性启动后常驻）
   └─ scene_vault_ai request（诊断或自动降级，处理后退出）
```

Rust 已能取得常驻 Python PID，但当前只写入 `vision.worker` 日志；one-shot PID 和
WebView2 进程树尚未通过 Tauri Command 暴露给前端。

## 视觉模式实测

环境：Windows 11、Python 3.12.3、同一张真实截图、同一 YuNet、每组 20 次、4 个 P 核。
工具：[benchmark_worker.py](../python/tools/benchmark_worker.py)。

| 指标 | SFace one-shot | SFace worker | ArcFace one-shot | ArcFace worker |
|---|---:|---:|---:|---:|
| 首次请求 | 786.598 ms | 610.115 ms | 1,345.361 ms | 1,151.965 ms |
| 健康握手 | — | 120.924 ms | — | 135.110 ms |
| 冷启动合计 | 786.598 ms | 731.038 ms | 1,345.361 ms | 1,287.074 ms |
| warm median | 771.167 ms | **346.338 ms** | 1,341.646 ms | **381.961 ms** |
| warm P95 | 782.176 ms | 349.307 ms | 1,386.070 ms | 401.534 ms |
| 20 次总耗时 | 15.452 s | **7.175 s** | 26.817 s | **8.435 s** |
| warm 吞吐提升 | — | **2.227×** | — | **3.513×** |
| 峰值 Working Set | 631.043 MiB | 775.672 MiB | 1,261.805 MiB | 1,464.566 MiB |
| 首次后 Working Set | — | 520.391 MiB | — | 1,210.223 MiB |
| 末次后 Working Set | — | 521.109 MiB | — | 1,210.012 MiB |
| 20 次增长 | — | +0.719 MiB | — | -0.211 MiB |

原始数据：

- [SFace](benchmarks/vision-worker-windows-2026-08-11.json)
- [ArcFace](benchmarks/vision-worker-arcface-windows-2026-08-11.json)

结论：

- 常驻模式对两种识别器都有明确收益，主要来自复用解释器、OpenCV 和模型。
- ArcFace worker warm 延迟只比 SFace worker 高约 10%，但稳态 Working Set 多约
  689 MiB；ArcFace 的主要工程代价是内存，不是 warm 推理时间。
- worker 的加载峰值高于 one-shot，但 20 次内未观察到持续增长。仍需 100/500 次耐久
  测试和 CPU/private bytes 数据，才能判定长期稳定性。
- 当前基准只统计 Python 进程树，没有把 Rust/WebView2 与 Python 合并成产品总预算。

## Rust 与 Vue/WebView2 空闲快照

在当前开发构建、主窗口已打开且 Python worker 未运行时，对
`scene_vault.exe` 及其 WebView2 子进程树采样 2 秒：

| 指标 | 当前快照 |
|---|---:|
| Rust 主进程 Working Set | 71.6 MiB |
| Rust 主进程 Private Bytes | 35.1 MiB |
| Rust + WebView2 总 Working Set | 562.7 MiB |
| Rust + WebView2 总 Private Bytes | 367.4 MiB |
| 归一化 CPU | 0.073%（32 逻辑 CPU） |
| 进程构成 | 1 个 Rust + 6 个 WebView2 |

这只是一次空闲快照，不是正式基准。WebView2 Working Set 含共享页，也可能与系统中其他
WebView2 实例共用代码页。正式资源页应同时展示 Working Set 和 Private Bytes，不应只
显示一个“内存占用”总数。

## 当前磁盘体积快照

### 开发工作区

| 类别 | 当前体积 | 说明 |
|---|---:|---|
| `src-tauri/target` | 33.18 GB | 开发构建产物，不属于安装包 |
| `target/debug` | 31.37 GB | 当前最大的开发磁盘占用 |
| `target/release` | 1.81 GB | 含依赖中间产物，不等于最终 exe |
| `node_modules` | 217.24 MB | 前端开发依赖 |
| `python/.venv` | 212.32 MB | 当前本机 Python 视觉环境 |
| `dist` | 0.77 MB | 当前前端生产静态资源 |

`target` 的 33 GB 是开发环境问题，应提供开发清理说明或脚本，但不能放入普通用户资源
页面，也不能由运行中的应用自动删除。

### 用户数据与运行时

| 类别 | 当前体积 | 清理现状 |
|---|---:|---|
| 模型目录 | 213.31 MB | 无自动清理；ArcFace 为用户自备 |
| `capture-output` | 200.69 MB | **没有常规清理闭环** |
| WebView2 用户数据 | 75.98 MB | 由 WebView2 管理 |
| 字体 | 61.30 MB | 无自动清理 |
| 缩略图 | 1.77 MB | 有上限，写入 64 次/启动时检查并回收到 75% |
| SQLite 主库 | 1.77 MB | 433 × 4 KiB，freelist 为 0 |
| 日志 | 0.08 MB | 5 MiB 轮转、14 天、最多 20 个归档 |

数据库当前有 83 条 capture、81 条 face、31 条 Face Bank sample。SQLite 体积统计必须
包含 `.db`、`.db-wal`、`.db-shm`，并显示 `page_count/page_size/freelist_count`。

`capture-output` 当前包含 40 个 item 目录、76 个文件，是最明确的运行时清理缺口。只能
在确认 capture 已完成、归档结果已校验且不再需要重试后删除；failed、processing、
archive_pending 和待重新处理记录必须保留。

## 正式测试矩阵

### 视觉隔离基准

每种模式至少执行：冷启动 5 次、warm 100 次、耐久 500 次。

```text
SFace one-shot
SFace worker
ArcFace one-shot
ArcFace worker
```

每组记录：

- 冷启动、warm median/P95/P99、总耗时；
- CPU user/kernel time、平均和峰值 CPU；
- Working Set、Peak Working Set；
- Private Bytes、峰值 Private Bytes；
- 20/100/500 次后的增长；
- worker 崩溃、超时、重启与 fallback；
- 响应和特征一致性。

### 整应用场景

```text
应用启动后空闲
Home / History / Capture / Workbench 各页面稳定后
打开 classify / note / workbench popup
导入 100 / 1,000 张截图但不开始识别
SFace worker 正在处理
ArcFace worker 正在处理
NAS 断线、重试和恢复
应用连续运行 1 / 4 / 24 小时
```

应把 Rust、WebView2、Python 分角色展示，并另给整应用汇总。CPU 必须说明是“单核心
百分比”还是“全机归一化百分比”。

## Windows 采样实现建议

后端使用 Windows API，而不是在产品中调用 PowerShell：

- Toolhelp32：构造 PID/Parent PID 进程树；
- `GetProcessTimes`：累计 user/kernel CPU time，两次采样计算 CPU%；
- `GetProcessMemoryInfo(PROCESS_MEMORY_COUNTERS_EX)`：Working Set、Peak Working
  Set、PrivateUsage、PagefileUsage；
- Rust 递归 `read_dir`：目录体积和文件数，不跟随符号链接；
- SQLite PRAGMA：数据库页、空闲页和 WAL 状态。

采样失败、权限拒绝和进程中途退出应返回局部结果，并标记 `approximate=true`，不能让
资源页失败影响 Capture worker。

## “资源与存储”页面方案

建议放在 Settings 下的新分类，而不是 History 日志页。

### 运行资源

- Scene Vault Rust；
- WebView2 合计及进程数；
- Python worker/one-shot；
- 整应用 Working Set、Private Bytes、CPU；
- worker 模式、PID、运行时间、当前是否加载模型；
- 采样时间和“近似值”说明。

页面可见时每 2 秒采样；窗口后台时 10 秒；页面卸载后停止。不要把资源采样接入现有
500 ms capture 轮询。

### 存储资源

- 数据库及 WAL/SHM；
- 缩略图；
- capture-output；
- 日志；
- 模型；
- 字体；
- Python runtime/venv 或未来 sidecar；
- WebView2 用户数据。

磁盘目录首次进入页面扫描一次，之后 30 秒或用户手动刷新；清理完成后立即刷新。

第一版只允许清理：

- 缩略图；
- 过期日志；
- 已完成且已归档的 capture-output。

数据库、模型、字体、Python runtime、WebView2 数据和用户归档目录只显示，不默认删除。

## 分阶段落地

1. **测量基础**：新增 Rust Windows 进程树、CPU、Working Set、Private Bytes；扩展
   Python benchmark 的 CPU/private bytes；完成 100/500 次 SFace/ArcFace 基准。
2. **存储统计**：统一数据库、缩略图、capture-output、日志、模型、字体和 runtime
   体积模型；先做只读命令。
3. **资源页面**：新增 `ResourceStatus`、`get_resource_status` 和 Settings 页面，动态
   指标与磁盘扫描分频刷新。
4. **安全清理**：归档成功后回收中间产物，增加缩略图/日志/已完成中间产物清理命令和
   清理结果审计日志。
5. **交付预算**：在 sidecar 与安装包完成后记录下载体积、安装体积、首次启动体积和
   更新增量，建立发布门槛。
