# 数据模型

## 核心关系

```text
projects ──< project_assets >── assets ──< asset_tags >── tags
    │                                 └──< collection_assets >── collections
    ├──< characters
    ├──< project_source_directories
    ├──1 project_notes
    └──< capture_sessions ──< capture_items ──< capture_faces
                                      │              │
                                      │              └──► characters
                                      ├──► characters
                                      └──► assets

characters ──< character_face_samples
library_roots ──< assets（路径归属；后续可增加显式 root_id）
```

## 通用素材表

### `projects`

创作项目。封面通过可空 `cover_asset_id` 引用素材。最近使用的来源和归档目录仅用于
预填；Capture Session 使用会话快照，不能依赖项目上的可变值恢复历史。

### `assets` 与 `project_assets`

`assets` 是全局素材索引，不强制属于单一项目；`project_assets` 表达素材与项目的
多对多关系。数据库保存路径、类型、状态和元数据，不保存素材二进制。源文件消失时将
状态标记为 `missing`，不直接删除记录。

### `tags`、`asset_tags`、`collections`、`collection_assets`

标签为全局、大小写不敏感唯一；素材与标签、合集均为多对多。合集可独立存在，也可
通过可空 `project_id` 归属项目。

### `library_roots`

保存用户选择的通用素材扫描根目录、启用状态和最后扫描信息。

## Capture Session 表

### `project_source_directories`

保存项目级截图来源目录及启用状态。一次会话覆盖该项目所有启用来源，并为每个来源建立
快照。路径只用于定位；跨来源去重以内容指纹为准。

### `capture_sessions` 与 `capture_session_baseline_files`

`capture_sessions` 表示一次连续采集，保存项目、开始/结束时间和状态；归档目标由项目
持有。`session_source_directories` 是会话启动时对启用来源的快照，之后修改项目来源
不会重写历史会话。
`capture_session_baseline_files` 保存会话开始时每个来源内已有图片的规范路径、大小和
修改时间，用于排除旧文件，同时允许同名文件被覆盖后作为新截图发现。

### `capture_items`

一张被发现截图及其完整生命周期。主要字段语义：

- `source_path`：源文件定位；源图不因归档被移动或删除。
- `classification`：`unclassified` / `person` / `scene` / `private`。
- `character_id`：人物截图的主角色；场景和收藏必须为空。
- `status`：`awaiting_label` / `queued` / `processing` / `archive_pending` /
  `completed` / `failed`。
- 处理与归档路径：本地派生产物、最终大图、头像和关联 `asset_id`。
- 失败恢复：`failure_stage`、尝试次数、下次重试时间、warning 和阶段时间。
- 识别建议：`suggested_character_id`、置信度、来源和复查状态。建议只是候选，确认后
  通过改判状态机更新主角色并重新处理，而不是仅修改复查状态。
- 闭集校验：选定角色分数、其他角色最佳分数和 `unverified` / `ok` / `flagged` 状态。
- `face_count`：检测到的人脸数量；当前建议仍只基于主脸。
- 文件身份：`source_file_state`、`destination_file_state` 和
  `destination_avatar_file_state` 分别记录原图、大图归档和头像归档的
  `available` / `missing` / `replaced` / `unavailable` 状态；路径重新定位不改变人物、
  分类、归档和人脸关系。

已完成项改判时保留旧归档，待新归档成功后再替换旧文件和资产关系。改判为 `scene` 或
`private` 会清空角色、建议和对应 Face Bank 样本。旧的 item 级特征列已废弃，运行时
只读写 `capture_faces`。

### `ignored_capture_contents`

保存用户从 Scene Vault 删除的截图内容哈希，作用域为项目。来源目录仍存在相同内容时，
发现和导入流程保持忽略；只有内容哈希变化才会登记为新的 Capture Item。该表不保存图片
二进制，也不授权应用删除原图。

### `capture_faces`

一张截图可有多个人脸行，保存索引、主脸标记、边界框、特征、模型身份、清晰度、面积
比例和可空的人工确认角色。当前只写一条主脸记录，但表结构为未来多人支持保留空间。

数据库约束：

- `(capture_item_id, face_index)` 唯一；
- 每个 `capture_item_id` 最多一条 `is_primary = 1`；
- 删除 Capture Item 时级联删除人脸；删除角色时确认角色引用置空；
- 特征必须携带 `model_id`、`model_version` 和维度，匹配禁止跨模型进行。

特征是可再生数据。重处理和样本库重建应在取得新结果后原子替换整组人脸记录，不能把
未知来源或不兼容的旧向量迁移到新模型空间。

### `characters` 与 `character_face_samples`

角色名在项目内大小写不敏感唯一，别名保存于 JSON；`avatar_asset_id` 引用用户确认的
代表头像。重命名只影响之后的归档文件名。角色合并会迁移 Capture Item、建议、素材
关系和样本，去重后删除源角色。

人物工作台只允许从当前角色已完成人脸处理和归档的截图中设置代表头像；清除后，角色
卡片回退显示最近一张可用头像。设置和清除都不修改源截图或已归档文件。

`character_face_samples` 表示“这张具体主脸被确认属于该角色”。同一角色、截图最多一
条，保存特征、模型身份、质量数据以及 `active` / `revoked`、`flagged` 状态。只有模型
身份匹配且处于可用状态的样本参与建议。无脸、低质量、缺模型身份或角色改判时删除不再
成立的旧样本；暂时性引擎失败不会被误当成新的无脸结论。

### `asset_characters`

素材与角色的多对多关系，为未来多角色标注和搜索保留。当前 Capture 工作流的归档命名
仍以 `capture_items.character_id` 的主角色为准。

### `project_notes`

每个项目最多一条笔记记录。`content` 是本地 Markdown 工作副本，`remote_path` 指向
项目归档目标目录中的 `笔记.md`；状态记录同步、失败和重试，最后同步内容用于检测外部
冲突。完整产品语义见
[截图分类与快速笔记决策](decisions/2026-08-05-capture-classification-and-notes.md)。

## 设置

`settings` 以键值 JSON 保存非敏感设置；凭据应进入系统凭据存储。主要键：

- `app.general`：分类/笔记快捷键、收藏默认显示、组合/拆分窗口、笔记失焦保存和空队列
  自动关闭。
- `capture.vision`：Python、模型、字体和识别器路径。
- `capture.processing`：检测、标注和头像裁剪参数。
- `capture.recognition`：提取时机、按模型区分的建议阈值、margin、闭集校验和样本质量
  门槛。
- `capture.archive_naming`：模板与分隔符。`{character}` 永远指主角色；设置只影响
  新归档。

## 全局约定

- ID 使用 UUID 字符串；时间使用 UTC RFC 3339 文本。
- SQLite 启用 foreign keys、WAL 和 busy timeout。
- 路径保存 Windows 原生路径或 UNC 路径，不保存 WSL 映射路径。
- 文件路径只用于定位；去重使用文件大小、修改时间和内容哈希组合。
- 所有结构变化通过 `src-tauri/migrations/` 中的新编号迁移完成，已执行迁移不可修改。
- AI 建议必须可被人工确认、覆盖或撤销；AI 不拥有业务真值。
