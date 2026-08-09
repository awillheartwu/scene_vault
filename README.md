# Scene Vault

Scene Vault 是面向各类游戏的 Windows 本地优先截图管理与创作辅助桌面应用。它使用
Tauri 2、Rust、Vue 3 和 SQLite 管理本地截图与项目资料；可选的 Python 视觉引擎用于
截图人脸检测、角色名标注、头像裁剪和 Face Bank 角色建议。

当前最完整的工作流覆盖通用游戏截图采集：监听一个或多个游戏截图目录，发现稳定写入
的新图片，完成人工分类和角色标记、可选视觉处理、可靠归档到本地目录或 NAS，以及
项目级 Markdown 快速笔记。它不依赖特定游戏类型、游戏引擎或截图工具；没有网络或
Python 引擎时，核心发现、分类、索引和归档流程仍然可用。

OCR、图片描述、通用自动标签、Embedding、语义搜索和 RAG 是后续路线图能力，当前
尚未实现。

## 仓库结构

```text
src/                 Vue 前端
src-tauri/           Tauri/Rust 后端与 SQLite 迁移
python/              可选的本地 AI 引擎
docs/                产品、架构、决策与开发文档
AGENTS.md             AI Agent 工作约束
```

## 开发命令

```powershell
pnpm install
pnpm test
pnpm build
start-tauri-dev.bat

cargo test --manifest-path src-tauri/Cargo.toml

$env:PYTHONPATH = "python/src"
python -m unittest discover -s python/tests
python -m scene_vault_ai health
```

环境配置和 Windows 一键安装说明见[开发指南](docs/DEVELOPMENT.md)。

## 文档

- [项目背景](docs/PROJECT_CONTEXT.md)
- [系统架构](docs/ARCHITECTURE.md)
- [数据模型](docs/DATA_MODEL.md)
- [Capture Session 工作流](docs/CAPTURE_WORKFLOW.md)
- [人脸识别基准测试](docs/BENCHMARK.md)
- [开发指南](docs/DEVELOPMENT.md)
- [开发路线图](docs/ROADMAP.md)
- [截图分类与快速笔记决策](docs/decisions/2026-08-05-capture-classification-and-notes.md)
- [Python AI 协议](python/README.md)
