# 系统架构

## 组件边界

```text
Vue 3 UI
  │ Tauri invoke / events
  ▼
Commands（IPC 边界）
  ▼
Rust Services ─────► Filesystem / Thumbnail Cache / Python Process
  │
  ▼
SQLite
```

- `src/` 负责展示、交互和 invoke 客户端，不直接访问 SQLite 或任意文件路径。
- `src-tauri/src/commands/` 接收参数并返回稳定的 IPC 模型，不承载业务规则。
- `src-tauri/src/services/` 负责事务、状态机、路径校验、处理队列和归档。
- `src-tauri/src/db/` 负责连接、迁移和 SQLite 基础设施。
- `python/` 是可选的单图处理引擎，不拥有主数据库，也不直接写 NAS。

主数据库位于 Tauri app data 目录；缩略图和处理中的派生产物位于 app local data 或
cache 目录。源素材保留在用户选择的位置。

## Capture Session

```text
游戏截图目录
  │ 轮询、稳定性校验、会话基线与内容去重
  ▼
capture_items(awaiting_label)
  │ 人工分类；person 选择主角色
  ▼
queued ──► processing ──► archive_pending ──► completed
              │                 │
              └─────────────────┴──────────► failed / retry
```

项目可以配置多个启用的来源目录。开始会话时，Rust 快照每个来源中已有图片的规范路径、
大小和修改时间；之后只忽略身份仍未变化的旧文件。相同内容通过哈希去重，不会因来自
不同目录而重复导入。

截图先进入待分类状态：

- `person`：必须选择主角色；视觉引擎可用时处理，否则降级为原图归档。
- `scene` / `private`：跳过 Python，原图进入对应目录。
- `unclassified`：不处理、不归档。

组合/拆分快捷键窗口、四目录和项目笔记的产品语义见
[截图分类与快速笔记决策](decisions/2026-08-05-capture-classification-and-notes.md)。

## Python AI 协议

Rust 通过版本化 JSON 请求启动 Python 单图任务。标准输出只能包含机器可读响应，日志
和带 `SVPROGRESS` 前缀的阶段进度写入标准错误；Rust 将进度转发为 Tauri 事件。

```text
Rust AI Service
  │ processScreenshot（绝对本地路径）
  ▼
Python Provider
  ├── YuNet 人脸检测
  ├── 名字标注与头像裁剪
  └── SFace / ArcFace 特征提取
```

Python 每次只处理 Rust 指定的一张图片及本地输出路径。它不扫描来源目录、不读取角色
名单、不修改 SQLite，也不把半成品写到 NAS。协议字段和错误码见
[Python AI 协议](../python/README.md)。

OCR、Caption 和通用 Embedding Provider 仍是未来能力；现有 Provider 抽象应允许它们
独立加入，但当前文档不得将其描述为已实现。

## Face Bank 与模型隔离

人工确认的 `person` 截图可以将主脸特征登记到角色样本库。新截图的主脸特征只与同一
项目、相同 `model_id`、`model_version` 和维度的活跃样本比较；阈值和第一/第二名
margin 共同决定是否产生建议。建议不会直接改变主角色，必须由用户确认。

特征是可再生派生数据：重处理或重建时先取得完整新结果，再在事务中原子替换。无脸、
低质量、缺少模型身份或角色改判时删除不再成立的旧样本；启动失败或超时等“没有得到
新结论”的错误可以保留旧样本，并显式报告其状态。

识别器可在 SFace 与 ArcFace 间切换。不同模型的 embedding 空间不可比较；切换模型后
旧样本保持可追溯但不参与匹配，用户可在工作台重建 Face Bank。ArcFace 权重由用户
自备，不随应用分发。阈值与效果以 [BENCHMARK.md](BENCHMARK.md) 为准。

当前产品仍采用“一张人物截图 = 一个主角色”。`capture_faces` 支持一张截图关联多张脸，
但当前只写主脸并只对主脸给建议；多人逐脸确认和多角色素材关联留待真实需求出现。

## 归档与改判

Python 输出先写本地缓存。Rust 校验后执行：

```text
本地结果 → 目标目录临时文件 → 大小/内容校验 → 原子重命名
```

NAS 离线或复制失败时保留本地结果和持久化状态，通过退避和人工重试继续。应用重启会
恢复待归档任务；处理中断会记录失败，避免无界循环。

已完成截图被改判角色或分类时，Rust 重新入队处理并保留旧归档引用。只有新归档成功
后，才删除旧归档文件和旧资产关系。进行中或待归档项拒绝改判，避免两个流程同时拥有
同一记录。

## 项目笔记

项目笔记内容以 SQLite 为本地工作副本，后台任务把它同步到项目归档目标目录中的
`笔记.md`。写入采用临时文件、校验和原子替换；目标离线时持续保留本地状态，检测到
外部修改时先生成时间戳备份。笔记同步与截图归档共享“本地先完成、远端可恢复”的原则。

## 路径与安全约束

- Windows 正式配置保存 `D:\...` 或 UNC 路径，不保存 WSL `/mnt/...` 路径。
- `canonicalize` 可能产生 `\\?\` 前缀；路径包含校验前必须规范化。
- Python 拒绝 URI、相对路径、UNC 输入和 Windows device namespace，只接受明确的
  本地输入/输出路径。
- SQLite 启用 foreign keys、WAL 和 busy timeout。
- 扫描发现源文件消失时标记 `missing`，不直接删除用户记录。
- 所有结构变化通过追加迁移完成；已执行迁移不可修改。

## 后续扩展边界

通用素材扫描需要支持多根目录、增量扫描、取消、进度、符号链接保护、类型识别和
缺失标记。OCR、描述、Embedding 和语义搜索必须继续保持可选 Provider，不得让网络、
模型或 Python 成为读取素材库的前置条件。
