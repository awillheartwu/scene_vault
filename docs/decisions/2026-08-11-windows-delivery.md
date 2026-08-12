# Windows 1.0 可交付包决策

日期：2026-08-11

## 结论

1.0 只发布 **Windows x64 NSIS**，采用**双安装包**模式：

- `scene-vault-1.0.0-setup.exe`（无 AI 小包）
- `scene-vault-1.0.0-ai-setup.exe`（含 PyInstaller sidecar、YuNet + SFace、默认字体）

同一套 `scripts/release-windows.ps1` 构建两种模式（`-SkipAI` 开关），构建后输出
两份安装包的实际体积供验收对比。暂不维护 MSI、macOS/Linux 不承诺。

## 已拍板的决策

1. **字体**：默认且随包提供得意黑（SmileySans-Oblique.ttf，约 2.5 MB，OFL-1.1
   可再分发）。其他字体（霞鹜文楷、Noto Sans SC）为 OFL 许可，但 **ZCOOL 三款
   许可存在争议**（Google Fonts 标 OFL，原始版本附带“保留所有权利”条款），不纳入
   官方分发列表。1.0 其余字体由用户手动放入应用 fonts 目录（现有机制）。
2. **体积**：无 AI 小包预计 30–45 MB（exe + 前端 + 字体）；AI 大包预计 100–160 MB
   （+ PyInstaller onefile sidecar，numpy + OpenCV 占大头）。精确数字以真实构建
   为准，发布脚本末尾输出两包体积。
3. **运行环境**：安装后只需要 Windows 10/11 x64 + WebView2。Node/Rust 仅在构建时
   需要；Python 不需要系统安装（sidecar 内嵌解释器）。系统 Python 路径保留为高级
   用户选项（现状已有，不额外做“检测本地 Python”分支）。
4. **WebView2**：不做应用内下载（应用依赖其运行，无法自举）；由 NSIS 安装器按
   `webviewInstallMode: downloadBootstrapper` 处理（Win11 预装、Win10 缺时安装时
   自动补）。完全无网络机器为 1.0 后条目。
5. **签名**：1.0 不强制。发布脚本预留签名步骤（`-SignCertificatePath` + signtool +
   时间戳），有证书则签；无证书时接受 SmartScreen“未知发布者”提示并写入验收清单。
   SignPath 免费版要求公开仓库，本项目私有，不适用；Azure Trusted Signing
   （$9.99/月）或商业 OV 证书为后续选项。
6. **ArcFace**：权重由用户自备（`w600k_r50.onnx`），永不随包分发；YuNet（~227 KB）
   与 SFace（~37 MB）随 AI 包分发。
7. **版本**：package.json / Cargo.toml / tauri.conf.json / Python engine 统一为
   1.0.0；Python 协议版本保持 `1` 独立演进。
8. **JSON → BLOB**：不先于安装包；10 万级阶段再做。
9. **CI**：先建立可重复的本机发布脚本；有可用 Windows runner 后再接 Forgejo
   自动构建。

## 实施顺序

1. 版本统一 1.0.0 + tauri.conf 打包骨架（NSIS / resources / AI 变体配置）
2. 双模式发布脚本 `scripts/release-windows.ps1`（含签名预留、体积输出）
3. PyInstaller spec 验证：在 Windows 构建 `scene-vault-ai-x86_64-pc-windows-msvc.exe`
   并冒烟（one-shot + worker 两种调用）
4. Rust sidecar 接入：设置解析优先 bundled sidecar；首次启动把资源（模型/字体）
   从 resource 目录复制到应用数据目录并自动写设置
5. 全新 Windows 环境验收：无 Python/无 AI 完成发现、分类、归档；AI 启用；
   更新/卸载数据保留；启动到可操作 P95 < 5s 实测；双包体积实测

## 验收结果（2026-08-12）

- 双包实测体积：`scene-vault-1.0.0-setup.exe` 5.7 MB；`scene-vault-1.0.0-ai-setup.exe`
  109.8 MB（sidecar 73.4 MB + YuNet/SFace 37 MB + 应用与字体）。
- 无 AI 流程、AI 内置引擎识别、更新与卸载数据保留均已实机验收通过。
- 实测数据（2026-08-12）：启动到可操作 P95 < 1 s（3 次冷启动，门禁 < 5 s）；
  安装后体积 128 MB（主程序 18 MB + sidecar 71 MB + 资源 40 MB）；sidecar 常驻内存
  sface 约 524 MB（arcface 约 2 GB，与加载模型有关）。
- 识别器调参经验：动漫/游戏立绘脸的 ArcFace/SFace 相似度普遍低于真实人脸（同角色
  约 0.2–0.3），建议阈值 0.5 时通常无建议；可按画风降低置信度阈值与 margin，并依靠
  “建议 → 人工确认”流程兜底。
- 待办：启动到可操作 P95 实机计时；Forgejo/GitHub 自动构建（有可用 Windows runner
  或 GitHub Actions 时接入）；代码签名（证书到位后启用）。

## 调整到 1.0 后

- 应用内可选字体下载（待下载/下载中状态 + SHA-256 校验；引入 HTTP 客户端后与
  AI 组件下载共用机制）
- 应用内按需下载 AI 组件（sidecar 托管于 Forgejo/GitHub Releases/NAS；未启用 AI
  时引导下载并自动配置）
- 完全无网络全新机器离线安装（捆绑 WebView2 offlineInstaller，约 +130 MB）
- 代码签名落地（Azure Trusted Signing 或商业 OV 证书）
- 10 万级素材库性能与存储压力测试
- 大组件系统性拆分（维护期）
- macOS / Linux 打包验证
