# 开发指南

## 环境

- Node.js 与 pnpm
- Rust stable 与 Windows MSVC toolchain
- Python 3.11+（仅开发可选 AI 引擎时需要）

正式 Windows 配置必须使用 `C:\...`、`D:\...` 或 UNC 路径，不能把 WSL 的
`/mnt/...` 路径写入应用设置。

## 安装与运行

```powershell
pnpm install
pnpm dev
start-tauri-dev.bat
```

应用首次启动会在 Tauri app data 目录创建数据库并运行内置迁移。

Windows 原生开发必须使用仓库根目录的 `start-tauri-dev.bat`。该入口通过 affinity mask
将构建工具限制在 CPU 0–9（10 个逻辑 CPU，mask `0x3FF`）。清理 `target`
后的首次全量构建使用单个 Cargo job，已有可执行文件时的增量构建使用两个 job；发布
脚本保留四个 Cargo job 的编译内存预算。job 数不是可使用的 CPU 数，也不受应用图片
处理设置控制。这保留了避免多个 `rustc.exe` 与 `cl.exe` 同时运行导致内存压力和
`0xc0000005 (STATUS_ACCESS_VIOLATION)` 的构建约束。WSL 中的重型命令统一使用
`taskset -c 0-9`。启动入口不再写死 `RAYON_NUM_THREADS=4` 或 `TOKIO_WORKER_THREADS=4`。

应用内的人脸处理、缩略图生成和图片缩放/编码统一读取“后台处理并行度”（1–32，
默认 4）。该设置控制处理线程和缩略图任务并发，界面、数据库和 WebView2 不纳入此
限制；它不把整个应用所有进程的合计 CPU 用量封顶。

正常安装后启动的应用不会继承发布构建进程的亲和性。开发入口设置
`SCENE_VAULT_DEV_RESTORE_AFFINITY=1`，应用在创建运行时和 WebView/Python 子进程前
恢复系统允许的 CPU 亲和性，构建工具仍保留 CPU 0–9。因此 dev/release 的后台处理
均按同一设置执行，设置大于 10 时不会被开发构建的十核范围额外截断。只有开发入口
显式设置此标记才会恢复亲和性；直接启动应用时保留外部启动器指定的范围。恢复失败
写入启动警告，并保留继承的 CPU 范围。

## 日常检查

日常开发不新增或运行测试、隔离探针、视觉检查和规模基准。必要时只检查产品源码：
前端 `vue-tsc --noEmit` 或 `pnpm build`，Rust `cargo check`（不带 `--tests`）、格式和
差异检查，以及只读日志核对。前端日常构建只包含 `src/`，测试类型检查由
`tsconfig.test.json` 单独负责。

CPU 0–9 的约定适用于整个开发过程的高 CPU 操作，包括 `cargo check/test/build`、
前端类型检查/构建和已获授权的 Python 检查。Windows 手动执行时先将当前 PowerShell
亲和性设为 `0x3FF`，WSL/Linux 每条重型命令使用 `taskset -c 0-9`。测试和基准仍按
相应授权阶段执行。

## 版本收尾

只有用户明确要求当前版本进入“版本收尾”才执行以下流程；讨论这个词或询问流程不算
启动授权。用户单独授权某项检查时，执行该项即可。

1. 冻结本轮范围，核对需求、工作树改动和已知缺陷，处理范围内的问题。
2. 删除过时或重复测试，再为当前业务规则、数据保护和具体缺陷补充必要回归。
   删除只锁定外观或复制实现的断言；错误语义、关键提示和协议兼容等实际契约应保留。
3. 运行前端、Rust、Python 的相关回归与静态检查；测试失败先核对需求和夹具，不能
   为了旧断言改坏当前行为。记录跳过项、环境限制和已有基线失败。
4. 在 Windows 实际应用中验收本轮流程及失败恢复；浏览器、mock 和 WSL 检查分别
   标明范围，不替代游戏、WebView2、回收站或 NAS 的实机结果。只使用可丢弃夹具做
   破坏性验收。
5. 确认交付版本和包类型，统一四处版本及 Cargo.lock，再执行发布预检和本地打包。
   “测试 release”只生成可运行 exe；标准发布才按要求生成安装包或 AI 包。
6. 检查产物、清理本轮临时文件并交付结果；提交、标签、推送和发布按用户授权执行。

收尾检查命令如下。`pnpm test` 先通过测试专用类型检查，再运行 Vitest；它和
`pnpm run test:typecheck` 都留在收尾阶段。

```powershell
# Windows PowerShell 7：先限制当前进程，子进程继承亲和性。
(Get-Process -Id $PID).ProcessorAffinity = 0x3FF
$env:CARGO_BUILD_JOBS = "4"

pnpm test
pnpm build

cargo test --manifest-path src-tauri/Cargo.toml
cargo clippy --manifest-path src-tauri/Cargo.toml --all-targets -- -D warnings
cargo fmt --manifest-path src-tauri/Cargo.toml -- --check

$env:PYTHONPATH = "python/src"
python -m unittest discover -s python/tests
python -m compileall -q python/src python/tests python/sidecar.py
```

修改文档或 IPC 字符串后还应运行 UTF-8 检查：

```powershell
python scripts/check-utf8.py
```

前端测试位于 `tests/`（目录结构与 `src/` 对应）；Rust 的模块级测试文件位于
`src-tauri/tests/unit/`，通过 `#[path]` 挂到对应 service。

## 图片与应用性能排查

`read_capture_image` / `read_capture_thumbnail` 每次成功读取都写一条性能日志（module `capture.image`，event `read_timing`，
message 为 `key=value` 分段毫秒值）；前端对应记录上报 module `capture.image`、落库为 `ui.capture.image`（后端统一加 `ui.` 前缀），只有 `durationMs`，即前端观测到的
读取耗时（invoke 往返，含 IPC 传输；不含 Blob 构建与 img 解码）。两条记录都带
`captureItemId`，可按图片关联；前端也记录失败读取的往返耗时，后端分段仍以成功读取为准。

分段含义：

- `guard` / `item` / `settings`：撤销分类互斥锁、条目查询、设置读取的数据库耗时。
- `validate` 及子项 `vstat`（stat）/`vread`（读文件）/`vhash`（SHA-256）/`vdb`（写回状态）：
  源文件校验，source 变体每次读取都执行整文件哈希；非 source 变体这四项为 0。
- `thumb` 及其子项：`permit`（等待生成许可）、`lookup`（缓存指纹与哈希校验）、
  `decode`（解码原图并编码 JPEG，仅未命中时非零）、`publish`（写缓存与目录维护）。
- `kind=full` 的原图直读只有 `item` / `validate` / `read`。

性能日志写入独立文件（不进入应用内“历史 → 调试日志”面板，避免刷屏）：
`%LOCALAPPDATA%\com.scenevault.desktop\logs\perf\perf-*.jsonl`（同样按大小轮转与清理）；
设置 → 资源与缓存 → 日志策略里的“性能日志写入独立文件”开关可以整体关闭，停止性能
日志写入和后台资源采样；图片处理与文件保护流程继续运行。

聚合 JSONL（脚本同时读取 logs 主目录和它下面的 perf 子目录，旧数据也能统计）：

```powershell
pwsh -File scripts\analyze-image-timing.ps1 -Minutes 10
pwsh -File scripts\analyze-image-timing.ps1 -Csv work\image-timing.csv
```

脚本按 变体、缓存命中与否输出 count/均值/p50/p90/max。判断口径：`decode` 大或 `hit=false` 占比高
说明在读图时生成缩略图；`permit` 大是在等生成许可（并发上限为后台处理并行度，1–32）；`lookup` 大是
缓存校验慢（网络盘、杀软扫描）；`validate` 大是源文件整文件校验；前端 `durationMs` 明显高于后端 `total`
时差额包含 IPC 传输、序列化和前端调度等待。

性能通道在 release 构建中同样启用，受同一个日志开关控制。扩展记录包括：

- `ui.capture.image / load_timing`：预览的 `read_ms`、`blob_ms`、`decode_ms`、
  `publish_ms`（Vue DOM 更新）和 `total_ms`；缩略图/卡片另外记录 `load_event_ms`，
  这是 URL 发布到浏览器 load 事件的时间，包含 DOM 调度、懒加载和解码，不代表纯解码
  或屏幕实际绘制。成功、失败、取消分别统计。
- `capture.archive / archive_resize`：增加探测、读取、解码、缩放、编码、临时输出写入
  的 `metrics` 分段；未重编码的输出也记录准备耗时。
- `capture.archive / archive_copy`：源文件哈希、目标核对、复制、临时副本校验、目标
  再核对、重命名和最终校验；`archive_fingerprint_saved` 记录指纹落库，`archive_total`
  包含整次归档及失败处理耗时。失败记录的未完成分段不编造为零。
- `app.resources / process_sample`：后台每 60 秒记录应用进程树的 CPU、Working Set、
  Peak Working Set、Private Bytes、进程数及主进程线程/句柄、累计 CPU、系统物理内存。
  CPU 按记录中的 `logicalProcessors` 归一；首次或无法测量的值为 null，采样基线与
  资源页独立。
- `app.resources / storage_sample`：后台每 5 分钟核对应用自有缓存、日志、WebView2
  数据和数据库/WAL/SHM 的字节与文件数。只读、不跟随链接；不完整扫描标记
  `complete=false`，关闭开关或退出应用会中止后续遍历。

`analyze-image-timing.ps1` 同时汇总上述记录及 `vision.request` 的 AI 分段，CSV/表格
包含单位列，区分毫秒、MiB、CPU 百分比与计数。前台资源页仍按原来的可见性规则运行。
`monitor-scene-vault.ps1` 保留为手动启动的外部 CSV 对照工具，其运行与停止由调用者控制。

## Python AI 引擎

核心截图工作流不要求安装 Python。开发引擎时可创建独立虚拟环境：

```powershell
python -m venv python/.venv
python -m pip install -e "./python[vision,dev]"

$env:PYTHONPATH = "python/src"
python -m scene_vault_ai health
```

视觉依赖使用 `opencv-contrib-python-headless>=4.10,<5`。Python 的请求字段、响应、
错误码、测试和 Sidecar 入口由 [python/README.md](../python/README.md) 维护。
通用设置中的“后台处理并行度”会通过
`SCENE_VAULT_IMAGE_PROCESSING_THREADS` 传给 Python/Sidecar，并同步设置 OpenCV、OMP、
OpenBLAS 和 MKL 的线程上限；Rust 缩略图与归档图片的解码、缩放、编码共用按该设置
创建的 Rayon 线程池，不再受启动脚本固定四线程影响。截图处理队列仍保持串行，缩略图
生成按同一设置限制在 1–32 路并发。每次解码占用数 MB 到数十 MB 缓冲，提高设置会
增加并发内存。修改后新任务使用新设置，已经开始的任务保留原线程池直至结束；这不是
整个应用及所有 Python/WebView2 进程合计只能使用该数量的线程。

## Windows 一键配置

在关闭 Scene Vault 后执行：

```powershell
powershell -ExecutionPolicy Bypass -File scripts\setup-windows.ps1
```

脚本创建 `python\.venv`、安装视觉依赖、下载 YuNet/SFace 模型和可分发字体、写入本机
`capture.vision` / `capture.recognition` 设置并运行健康检查。首次执行需要联网；若应用
数据库尚不存在，先启动应用一次。

便捷入口 `scripts\setup-windows.bat` 调用同一套配置流程。模型、字体、虚拟环境和
数据库都是本机产物，不提交到 Git。

ArcFace 是可选识别器，用户需自行提供兼容的 ONNX 权重；项目不分发
`w600k_r50.onnx`。切换识别器后在人物工作台重建 Face Bank，禁止混用不同模型空间的
特征。

## 设置与运行时行为

- “视觉引擎”配置 Python、YuNet、SFace/ArcFace 和字体路径，并提供健康检查。
- “视觉处理参数”配置检测、标注、头像裁剪和主脸框选（扩边重试比例、选区内多脸策略）；
  留空字段沿用 Python 默认值。
- “自动角色建议”按模型保存建议阈值、margin、闭集校验和样本质量门槛。
- “归档命名规则”配置模板与分隔符，只影响新归档。
- AI 未配置或健康检查失败时，截图发现、三分类和人工标记仍可用；人物图降级归档后可
  重新识别。

## 人脸识别 benchmark

评测工具为 `python/tools/face_benchmark.py`，报告工具为
`python/tools/face_eval_report.py`。数据集切分、命令、指标口径、当前结果和必须重跑的
条件统一见 [BENCHMARK.md](BENCHMARK.md)，不要在开发指南复制阈值表和历史结果。

## 数据库迁移

迁移位于 `src-tauri/migrations/`，按编号顺序内嵌到应用。已在任何环境执行的迁移不可
修改；结构变化必须新增更高编号的迁移。日常核对已执行迁移未被改写；在版本收尾时
补充和运行迁移回归，确认 SQLx checksum 及从空库升级路径都正常。

本机数据库、备份、模型、缓存和测试数据不得提交。不要使用会删除 ignored 评测数据的
`git clean -fdX`。

## 版本与发布

版本号统一维护在 4 处（`package.json`、`src-tauri/Cargo.toml`、
`src-tauri/tauri.conf.json`、`python/pyproject.toml`，`Cargo.lock` 根包同步）。
发布脚本强制四处一致，不要手动单改。
发布脚本负责版本预检、构建与可选标签，不自动执行前述完整回归或实机验收；执行打包
前应完成版本收尾的检查阶段。讨论收尾和本地打包都不自动授权推送或发布。

日常开发使用 `dev`，可以频繁提交和推送；`main` 接收已完成收尾与实机验收的版本。
发布顺序为：`dev` 完成收尾、统一版本并提交 → 合入并推送 `main` → 在合入后的提交上
创建并推送 `vX.Y.Z` 注解标签 → GitHub Actions 构建双包并发布。本地标准打包也从
`main` 执行。普通分支推送不会自动打包，沿用版本标签作为正式发布触发点。

自动 CI 与发布只在 GitHub Actions 执行，Forgejo 用于同步代码。Forgejo 15 在没有自己
的工作流目录时会回退读取 `.github/workflows`，因此保留只含说明文件的
`.forgejo/workflows/` 目录，阻止该回退；不要在其中添加 YAML 工作流。此改动提交并同步
到相应分支或版本后生效，旧分支和历史标签不会自动获得该目录。
要立即关闭该 Forgejo 仓库的 Actions，在仓库设置的「Units → Overview」取消勾选
Actions 并保存；这不影响 GitHub。已有排队或运行的任务另行取消。

CI 在安装工具和构建前校验发布提交已进入 `main`；手动运行仅接受 `main` 或已经合入
`main` 的版本标签。手动从 `main` 运行时读取统一版本配置并上传安装包，发布仍由版本
标签触发。发版后将 `main` 上的收尾提交快进或合并回 `dev`，再继续开发，不重置或丢弃
已有开发提交。用户要求暂不提交时，改动留在 `dev` 工作区等待一起提交。

- **日常提交**：`.\scripts\commit.ps1 "feat(x): ..."` —— 自动把 patch 号 +1
  （1.0.0 → 1.0.1）并提交改动；`-NoBump` 跳过版本变更，`-NoStageAll` 只提交已暂存
  内容（工作区有其他进行中的改动时使用）。
- **发版**：确认统一版本已提交并合入 `main` 后执行
  `.\scripts\release-windows.ps1 -Version 1.1.0 -CreateTag` —— 校验版本、本地构建双包，
  成功后创建 `v1.1.0` 注解标签。示例版本替换为 `main` 已提交的版本；版本提升在
  `dev` 提交前使用 `bump-version.ps1`，不要给尚未提交的版本文件创建发布标签。
- **发布脚本冒烟**：运行 `release-windows.ps1 -Version 1.0.6 -SmokeTest`，只校验四处
  版本一致性和陈旧安装包清理，不构建、不下载、不创建标签；把版本替换为当前版本。
- **发布**：推送指向已合入 `main` 提交的 `v*` 标签到 GitHub，触发 Actions 构建双包挂
  Release，CI 从标签名读取版本号；patch 自动累加和 `dev` 推送都不会触发发布。

### 构建排错（本机）

- `release-windows.ps1` 的 AI sidecar 步骤复用 `%TEMP%\sv-ai-build-venv`。若该目录
  损坏（典型报错 `No pyvenv.cfg file`、pip 退出码 106），直接删除该目录后重跑，脚本会
  重新创建。
- 标签由 `release-windows.ps1 -CreateTag` 在**构建成功之后**创建，所以本地与
  GitHub Actions 默认串行。已完成回归和实机验收、且用户授权并行发布时，先统一版本并
  提交，再创建注解标签并推送 GitHub；本地脚本使用 `-Version X.Y.Z`，不再传
  `-CreateTag`。两边各自生成仅应用和 AI 包，构建成功后才有可交付安装包。
- 用户要求 build 不用监视时，以独立 Windows PowerShell 进程启动本地脚本，将输出、
  进程 ID 和退出结果留在被 Git 忽略的 `dist-release/` 中；确认已启动后交付日志路径。
  远程只确认标签推送和工作流触发，不等待构建完成，也不提前宣称打包成功。
- 清理只删除已确认的构建或测试残留，不清理用户原图、主数据库、备份或评测数据；
  保留 Cargo、pnpm 和正常构建虚拟环境以便增量构建。缺少 `pyvenv.cfg` 的
  `%TEMP%\sv-ai-build-venv` 属于损坏的专用打包环境，关闭相关进程后可删除重建。
- 本机 Windows Git 已可用；SSH 会话中的提交和推送统一在 WSL 工作区执行。

## 文档职责

- [PROJECT_CONTEXT.md](PROJECT_CONTEXT.md)：产品定位、当前能力和非目标。
- [ARCHITECTURE.md](ARCHITECTURE.md)：组件边界与系统级规则。
- [DATA_MODEL.md](DATA_MODEL.md)：表关系、字段语义和数据约束。
- [CAPTURE_WORKFLOW.md](CAPTURE_WORKFLOW.md)：当前工作流和 Windows 验收。
- [BENCHMARK.md](BENCHMARK.md)：识别评测的唯一权威文档。
- [ROADMAP.md](ROADMAP.md)：真正未完成的工作。
