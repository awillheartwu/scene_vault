# Scene Vault Agent Guide

本文件适用于整个仓库。开始修改前，先阅读 `docs/PROJECT_CONTEXT.md`、
`docs/ARCHITECTURE.md` 和与任务有关的文档。

## 产品原则

- Scene Vault 是本地优先的桌面应用，不是云端网盘。
- 用户文件默认留在原位置；数据库保存索引和业务关系，不擅自移动源文件。
- 核心素材管理必须在没有网络、没有 Python AI 引擎时仍然可用。
- AI 是可选增强能力。模型、供应商和运行方式必须可替换。
- 当前优先级是数据层和素材索引，不主动扩展 UI Demo。

## 架构边界

- `src/`：Vue 展示、交互和 Tauri invoke 客户端，不直接访问 SQLite。
- `src-tauri/src/commands/`：Tauri IPC 边界，只做参数接收和结果返回。
- `src-tauri/src/services/`：业务规则和数据库操作。
- `src-tauri/src/db/`：连接、迁移和数据库基础设施。
- `src-tauri/src/models/`：IPC/数据库共享的数据结构。
- `python/`：可选 AI 引擎。通过版本化 JSON 协议与 Rust 集成，不拥有主数据库。

## Capture Session 边界

- Rust 负责截图目录监听、任务队列、历史记录、失败恢复和 NAS 归档。
- Python 只处理 Rust 指定的本地输入与本地输出路径，不直接扫描游戏目录或写入 NAS。
- Vue 负责人工角色标记、处理状态和历史浏览，不通过浏览器 API 直接读取任意文件。
- 用户可以继续使用游戏内置、平台或第三方截图工具，应用只监听新文件；不要默认向游戏
  进程注入按键或依赖特定截图方式。
- 归档必须采用“本地生成 → 复制临时文件 → 校验 → 原子重命名”的流程。
- 自动角色识别只能提供建议，低置信度结果必须允许人工确认和纠正。

## 数据约束

- 主数据库位于 Tauri app data 目录，不放在仓库或素材目录。
- 所有建表和结构变化必须通过 `src-tauri/migrations/` 迁移。
- ID 使用 UUID 字符串；时间使用 UTC RFC 3339 字符串。
- SQLite 必须启用 foreign keys、WAL 和 busy timeout。
- 素材可以被多个项目复用，使用 `project_assets`，不要给 `assets` 增加强制
  `project_id`。
- 扫描发现源文件消失时优先标记 `missing`，不要直接删除用户的素材记录。
- 文件路径仅用于定位；后续去重应使用文件大小、修改时间和内容哈希组合。

## 开发规则

- 保持 Commands 薄，将业务逻辑放进 Services。
- 不在 Rust 中硬编码 AI 厂商或模型。
- 不提交数据库、模型权重、缓存、虚拟环境、`target/` 或 `dist/`。
- 添加功能时同步更新相关文档和测试。
- 除非任务明确要求，不做与任务无关的大规模 UI 或依赖升级。

## 完成前验证

```powershell
pnpm build
cargo test --manifest-path src-tauri/Cargo.toml
$env:PYTHONPATH = "python/src"
python -m unittest discover -s python/tests
```
