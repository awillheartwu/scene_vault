# 人脸识别基准测试（Face Recognition Benchmark）

## 目的与定位

Scene Vault 的人脸识别评测工具，已从"临时脚本"升级为 recognition 的
**回归测试套件（regression suite）**。它的作用：

- 换模型（SFace / ArcFace / 未来其他）、换 OpenCV 版本、换 alignment 实现后，
  必须重跑，防止静默退化；
- 为每个识别器确定建议阈值 / margin / 校验带宽（benchmark 校准）；
- 用 leave-one-game-out 验证"全局 profile 到新游戏上是否还成立"；
- 历史上已经抓出两个重大问题：cv2 5.0.0 特征污染、alignCrop 跨进程漂移。

数据来自用户自建的游戏截图集（按角色名组织），不是公开数据集——评测结论只对当前
数据集覆盖的画面风格负责，不能直接外推到所有游戏。

## 数据集

### 布局

```text
<root>/<game>/<character>/bank/*.png    -- Face Bank 样本（train）
<root>/<game>/<character>/query/*.png   -- 查询图（test）
```

- **train/query 严格分离**：同一个角色的 bank 与 query 不得有内容相同的文件。
- 真实数据可以在本地或 NAS 上以“游戏目录 + 角色名文件名”组织（如
  `\\NAS\Pictures\Games\<游戏>\<角色>.png`，重复图带数字后缀）。
  `scripts/build_face_eval_manifest.ps1` 扫描并按角色分组、切分
  （bank ≤ 5 张/角色，其余进 query），生成清单
  `python/tests/fixtures/face_eval/dataset.json`（**只引用路径，不复制图片**）。
- 数据集目录已 gitignore，不入库。

### 过滤规则

- **只统计单脸图**：YuNet 检出恰好 1 张脸的图才参与评测。多脸图的文件名标签
  是"场景主角"而非主脸人物（实测案例：Dalia 的 3 人同框图主脸是 Edwin），
  标签不可靠。2026-08-08 数据集：283 张 → 82 张多脸 + 15 张无脸被过滤，
  有效查询 71 张。
- **SHA-256 防泄漏**：bank 与 query 内容完全相同的文件（包括复制改名）会在
  评测开始时报错（`--skip-leak-check` 可关闭）。

## 评测口径（2026-08-08 定版）

### 匹配与排名

- 每个角色的得分 = **max（样本相似度）**（topk_mean(k=1)），与产品建议规则
  一致；网格仍会扫 topk=1/2/3 用于策略对比，但头版指标固定用 max。
- **零样本角色**（提取后 bank 为空）：不入排名，按"不可匹配"计入 rank4+。
  它们不可能被建议，排名给 0 分名次是误导。

### 两套指标

| 指标集 | 统计范围 | 回答的问题 |
|---|---|---|
| 全部查询（all） | 所有单脸查询 | 真实工作流整体能帮我多少（零样本计入沉默/误推荐） |
| 可识别子集（eligible） | GT 角色 bank ≥ 1 样本 | 模型认不认识 Face Bank 已知的人 |

### 输出指标

- Top-1 / Top-3（max 排名下 GT 的 rank）；
- 真值 rank 分布（1/2/3/4+）与中位 rank；
- 同角色 / 跨角色分数分布（分位数，衡量可分性）；
- threshold × margin 网格：每个格子的建议精度（precision）、覆盖率
  （coverage）、错误建议率（false-suggest rate）；
- 产品关键指标：**精度 ≥90% / ≥95% 时能达到的覆盖率**；
- 每 query 明细（queryDetails：trueRank + Top-5 + eligible 标记），用于人工
  抽查错例。

### Leave-One-Game-Out

`--leave-one-game-out`：13 折。每折在其余 12 个游戏的可识别子集上网格调参
（≥95% 精度下覆盖率最高的 threshold/margin），再以该**固定点**评测第 13 个
游戏（全部 + 可识别子集）。汇总 13 折的留出结果。这回答：

> “0.50/0.10 这套全局 profile，到一个没参与调参的新游戏上还能不能维持
> 高 precision？"

## 运行方法

需要 Windows 侧的 venv（cv2 已固定 <5）。**务必保留 `--threads` 限制**，
默认 8 个逻辑 CPU（4 个 P 核），否则会卡死机器。

```powershell
$env:PYTHONPATH = "python\src"
$modelRoot = Join-Path $env:LOCALAPPDATA "SceneVault\models"

# SFace（含 LOOGO 与防泄漏）
python python\tools\face_benchmark.py `
  --manifest python\tests\fixtures\face_eval\dataset.json `
  --yunet-model "$modelRoot\face_detection_yunet_2023mar.onnx" `
  --sface-model "$modelRoot\face_recognition_sface_2021dec.onnx" `
  --recognizer sface --leave-one-game-out `
  --out python\tests\fixtures\face_eval\sface.json

# ArcFace R50（模型需自备 w600k_r50.onnx，non-commercial 许可，不入库）
python python\tools\face_benchmark.py `
  --manifest python\tests\fixtures\face_eval\dataset.json `
  --yunet-model "$modelRoot\face_detection_yunet_2023mar.onnx" `
  --sface-model "$modelRoot\face_recognition_sface_2021dec.onnx" `
  --recognizer arcface --arcface-model "$modelRoot\w600k_r50.onnx" `
  --leave-one-game-out `
  --out python\tests\fixtures\face_eval\arcface.json
```

报告渲染（HTML + 总览图，图表内嵌 base64，可离线打开）：

```powershell
python python\tools\face_eval_report.py `
  --root python\tests\fixtures\face_eval `
  --out-dir <输出目录>
```

新截图入库后重跑 `build_face_eval_manifest.ps1` 即可增量更新数据集。

## 回归测试

- **alignCrop 跨进程确定性**（`test_aligncrop_is_deterministic_across_processes`）：
  两个独立进程提取同一张图，断言特征余弦 ≈ 1。需要环境变量：
  `SCENE_VAULT_TEST_MODELS`（含 yunet + sface onnx 的目录）与
  `SCENE_VAULT_TEST_FACE_IMAGE`（一张含单脸的截图）。
- 纯评分逻辑单测（无 cv2）：`python/tests/test_face_eval.py`。

## 结果存档与当前结论（2026-08-08）

报告 JSON：`python/tests/fixtures/face_eval/{sface,arcface}.json`。

核心对比（71 个单脸查询，全部 / 可识别 56）：

| 指标 | SFace | ArcFace R50 |
|---|--:|--:|
| Top-1 | 67.6% / 85.7% | **71.8% / 91.1%** |
| Top-3 | 73.2% / 92.9% | **77.5% / 98.2%** |
| 精度 ≥95% 时覆盖率 | 49.3% / 76.8% | **62.0% / 82.1%** |

结论：

- ArcFace 全面优于 SFace（可分性、Top-1、高精度覆盖率），已接入 runtime；
- 16 个 rank4+ 里 15 个是零样本角色——瓶颈是 **Face Bank 覆盖**，不是模型；
- LOOGO：ArcFace 校准点全部落在 0.45 家族，留出可识别子集 95.7% / 80.4%，
  全局 profile 思路成立；
- 运行时默认值 **provisional**：SFace 0.50/0.05、ArcFace 0.50/0.10
  （校验带宽 SFace 0.65/0.40/0.10、ArcFace 0.55/0.35/0.10 为占位）。

## 何时必须重跑

- 换 recognizer 模型 / 换 OpenCV 版本 / 改 alignment；
- Face Bank 样本显著扩充后（样本数量实验：1/3/5 张/角色的覆盖率曲线）；
- 校准新游戏的建议阈值 / margin / 校验带宽。

## 常驻 Worker 性能与稳定性（2026-08-11）

原始报告：[vision-worker-windows-2026-08-11.json](benchmarks/vision-worker-windows-2026-08-11.json)。
在 Windows 11、Python 3.12.3、4 个 P 核上，对同一张真实截图、同一 YuNet 与 SFace
权重连续运行 20 次；不生成标注图或头像，比较完整 Python 进程、JSON 协议和模型处理
链路。RSS 统计 venv 启动器及其 Python 子进程的进程树。

| 指标 | one-shot | worker |
|---|---:|---:|
| 首次请求 | 786.598 ms | 610.115 ms |
| worker 健康握手 | — | 120.924 ms |
| worker 冷启动合计 | — | 731.038 ms |
| 后续请求中位数 | 771.167 ms | **346.338 ms** |
| 后续请求 P95 | 782.176 ms | **349.307 ms** |
| 20 次总耗时 | 15,451.895 ms | **7,174.550 ms** |
| 峰值 RSS | 631.043 MiB | 775.672 MiB |
| 首次后 / 末次后 RSS | — | 520.391 / 521.109 MiB |

结论：常驻模式后续请求约 **2.227 倍**快，20 次总耗时减少约 53.6%；20 次处理后的
常驻 RSS 仅增长 0.719 MiB，没有观察到持续增长。两种模式的检测、特征和警告响应完全
一致；强制杀死 worker 后重启处理成功。常驻进程会长期保留约 521 MiB 工作集，且模型
加载期间峰值高于 one-shot，这是用内存换取吞吐的明确代价。

因此默认模式保持 `worker`，保留 `oneshot` 诊断开关和自动降级。Windows 交付仍采用
Python AI sidecar/运行时与模型资源，不因本次结果迁移到 Rust 推理；低内存场景可在启动
前设置 `SCENE_VAULT_VISION_MODE=oneshot`。基准可用
`python/tools/benchmark_worker.py` 重跑。

## 历史教训（不要重蹈）

- 2026-08-06：cv2 5.0.0 的 `FaceRecognizerSF.feature()` 产生"黑图特征"，
  统计全部作废；依赖已固定 <5。
- 2026-08-08：`alignCrop` 只传 1x4 边界框时跨进程漂移（同图特征余弦可低到
  0.3），单进程内自检通过但整跑两次对不上；修复为传 YuNet 5 关键点（1x14），
  并固化跨进程回归测试。**换 alignment 后第一件事是跑确定性回归测试。**
