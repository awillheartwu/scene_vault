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
将构建和应用进程限制在 CPU 0–3。清理 `target` 后的首次全量构建使用单个 Cargo job，
已有可执行文件时的增量构建使用两个 job；运行时仍设置 `RAYON_NUM_THREADS=4` 和
`TOKIO_WORKER_THREADS=4`。这是为了避免多个 `rustc.exe` 与 `cl.exe` 同时运行导致内存
压力和 `0xc0000005 (STATUS_ACCESS_VIOLATION)`。WSL 中的重型 Rust 命令统一使用
`taskset -c 0-3`。

## 验证

```powershell
pnpm test
pnpm build

taskset -c 0-3 cargo test --manifest-path src-tauri/Cargo.toml
taskset -c 0-3 cargo clippy --manifest-path src-tauri/Cargo.toml --all-targets -- -D warnings
cargo fmt --manifest-path src-tauri/Cargo.toml -- --check

$env:PYTHONPATH = "python/src"
python -m unittest discover -s python/tests
python -m compileall -q python/src python/tests python/sidecar.py
```

修改文档或 IPC 字符串后还应运行 UTF-8 检查：

```powershell
python scripts/check-utf8.py
```

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
- “视觉处理参数”配置检测、标注和头像裁剪；留空字段沿用 Python 默认值。
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
修改；结构变化必须新增更高编号的迁移。修改迁移后运行 Rust 测试，确认 SQLx checksum
和从空库升级路径都正常。

本机数据库、备份、模型、缓存和测试数据不得提交。不要使用会删除 ignored 评测数据的
`git clean -fdX`。

## 版本与发布

版本号统一维护在 4 处（`package.json`、`src-tauri/Cargo.toml`、
`src-tauri/tauri.conf.json`、`python/pyproject.toml`，`Cargo.lock` 根包同步）。
发布脚本强制四处一致，不要手动单改。

- **日常提交**：`.\scripts\commit.ps1 "feat(x): ..."` —— 自动把 patch 号 +1
  （1.0.0 → 1.0.1）并提交改动；`-NoBump` 跳过版本变更，`-NoStageAll` 只提交已暂存
  内容（工作区有其他进行中的改动时使用）。
- **发版**：`.\scriptselease-windows.ps1 -BumpVersion 1.1.0 -CreateTag` —— 升
  版本号（此处升 minor）、本地构建双包、成功后创建 `v1.1.0` 注解标签；也可以用
  `-Version 1.1.0` 只校验不升文件。
- **发布脚本冒烟**：运行 `release-windows.ps1 -Version 1.0.6 -SmokeTest`，只校验四处
  版本一致性和陈旧安装包清理，不构建、不下载、不创建标签；把版本替换为当前版本。
- **发布**：推送 `v*` 标签到 GitHub 即触发 Actions 构建双包挂 Release，CI 从标签名
  读取版本号；patch 自动累加不会触发任何发布。

## 文档职责

- [PROJECT_CONTEXT.md](PROJECT_CONTEXT.md)：产品定位、当前能力和非目标。
- [ARCHITECTURE.md](ARCHITECTURE.md)：组件边界与系统级规则。
- [DATA_MODEL.md](DATA_MODEL.md)：表关系、字段语义和数据约束。
- [CAPTURE_WORKFLOW.md](CAPTURE_WORKFLOW.md)：当前工作流和 Windows 验收。
- [BENCHMARK.md](BENCHMARK.md)：识别评测的唯一权威文档。
- [ROADMAP.md](ROADMAP.md)：真正未完成的工作。
- [决策记录](decisions/2026-08-05-capture-classification-and-notes.md)：已确认且长期有效的
  截图分类与快速笔记语义。
