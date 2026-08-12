# Scene Vault Release Notes

## v1.0.4（2026-08-13）

- 应用标识符由 `com.scenevault.app` 调整为 `com.scenevault.desktop`（规避与 macOS
  `.app` 应用包扩展名冲突的警告；应用数据目录随标识符变更，不迁移）。

## v1.0.3（2026-08-13）

- 新增「显式标注人物」开关（设置 → 通用设置）：关闭后识别、脸向量入库与 Face Bank
  建议照常，仅跳过标注图生成；人物截图归档到「人物图（原图）」并按角色命名，不产生
  带名字的副本。降级/重新识别以识别输出为信号，不受开关影响。
- 应用标识符改为 `com.scenevault.app`（不再包含个人姓名）；应用数据目录随标识符变更，
  旧目录数据不自动迁移。
- 修复：打开尚不存在的应用缓存目录时自动创建再打开；目录打开失败的原因写入应用日志
  （`ui.opener`）。

## v1.0.2（2026-08-12）

- 修复：设置页「资源与缓存」8 个目录打开按钮（opener scope 白名单、capability 权限、
  Python 运行环境路径）；不存在的位置显示「尚未配置」；失败原因记入日志。

## v1.0.0（2026-08-12）

首次正式发布。发布页附件：`scene-vault-1.0.0-setup.exe`（5.7 MB）与
`scene-vault-1.0.0-ai-setup.exe`（110 MB）。

## 概述

Scene Vault 是面向各类游戏的 **Windows 本地优先**截图管理与创作辅助桌面应用：监听
一个或多个游戏截图目录、内容去重登记、人工分类与角色标记、可选本地 AI 角色识别建议、
可靠归档到本地目录或 NAS 共享。不绑定特定游戏、引擎或截图工具。

## 主要特性

- **本地优先**：截图留在原位置，数据库只存索引与业务关系；无网络、无 AI 时核心
  流程（发现、分类、归档）完整可用。
- **截图采集**：多来源目录监听、内容哈希去重（相同截图只登记一次）、稳定写入判定、
  批量导入（点击显式按钮后才识别与生成缩略图，严格一次一张）。
- **分类与角色**：人物/游戏截图/收藏/待分类；人物角色标记、AI 建议（低置信度人工
  确认）、角色重命名/合并/代表头像、Face Bank 样本管理。
- **可靠归档**：四目录本地或 NAS（UNC）归档、失败自动重试 4 次后标记并可手动重试、
  项目 Markdown 笔记同步与冲突备份、源文件永不移动。
- **内置 AI（AI 包）**：PyInstaller sidecar（内嵌 Python 3.12 + numpy + OpenCV +
  YuNet/SFace + 得意黑字体），首次启动自动解包模型与字体，无需安装 Python；未配置
  时自动回退无 AI 流程。ArcFace 权重由用户自备，未配置自动回退 SFace。
- **数据安全**：完整性预检、一致性备份（SHA-256 清单）、两阶段恢复与失败回滚、
  迁移前自动备份、数据体检与索引维护、恢复模式页面。
- **规模与性能**：服务端分页；1.0 守门为单项目 1 万张截图 / 1 万文件源目录 /
  100 角色；启动到可操作 P95 < 1 s。
- **体验**：自定义右键菜单、结构化日志与诊断复制、资源与存储诊断页、慢扫描警告、
  会话中途挂载新目录、待分类跨会话可见。

## 安装包

只发布 Windows x64 NSIS 双包，均含应用内简体中文（得意黑）渲染字体：

| 安装包 | 内容 | 实测体积 |
|---|---|---|
| [scene-vault-1.0.0-setup.exe](https://github.com/awillheartwu/scene_vault/releases/download/v1.0.0/scene-vault-1.0.0-setup.exe) | 应用（无 AI） | 5.7 MB |
| [scene-vault-1.0.0-ai-setup.exe](https://github.com/awillheartwu/scene_vault/releases/download/v1.0.0/scene-vault-1.0.0-ai-setup.exe) | 应用 + 内置 AI 引擎（YuNet/SFace/字体） | 110 MB |

环境要求：Windows 10/11 x64 + WebView2（Win11 预装；Win10 安装器自动补装）。
无需安装 Node、Rust 或 Python。

## 已知限制

- 安装包尚未代码签名，首次运行可能出现 SmartScreen“未知发布者”提示（1.0 后接入
  Azure Trusted Signing 或商业 OV 证书）。
- 动漫/游戏立绘脸的 SFace/ArcFace 相似度普遍低于真实人脸，默认阈值下常无建议；
  可按画风降低置信度阈值，并依靠“建议 → 人工确认”流程兜底。
- 完全无网络的全新机器离线安装（捆绑 WebView2 离线包）为 1.0 后能力。
- 仅 Windows x64；macOS/Linux 未承诺。

## 实测数据（2026-08-12）

- 双包体积 5.7 / 109.8 MB；安装后占用约 128 MB（主程序 18 MB + sidecar 71 MB +
  资源 40 MB）。
- 启动到可操作 P95 < 1 s（3 次冷启动，验收门禁 5 s）。
- sidecar 常驻内存：sface 约 524 MB；arcface 约 2 GB（与加载模型有关）。
- 12 项 Windows 原生端到端验收全部通过，含 NAS 断线模拟恢复与全新环境安装/卸载；
  明细见 `docs/decisions/2026-08-11-windows-delivery.md` 与
  `docs/CAPTURE_WORKFLOW.md`。

## 致谢与许可

- 视觉模型：YuNet、SFace（随 AI 包分发）；ArcFace 权重 `w600k_r50.onnx` 由用户
  自备，不随包分发。
- 字体：得意黑（SmileySans，OFL-1.1 可再分发）。
- 完整许可与数据存放说明见 README「数据与许可」。
