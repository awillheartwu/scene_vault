# Scene Vault AI Engine

这是 Scene Vault 的可选本地处理引擎。桌面应用与素材历史不依赖 Python 启动；
Python 只处理 Rust 指定的一张本地图片，不扫描目录、不操作 SQLite，也不直接写入
NAS。

当前引擎版本为 `0.5.0`，协议版本为 `1`，已提供：

- 严格的 JSON 请求校验和机器可读错误；
- Windows Unicode 路径图片读取；
- YuNet 人脸检测和可独立测试的主脸评分；
- 中文/Unicode 角色名标注；
- 头像裁剪；
- SFace / ArcFace 主脸特征提取与模型身份返回；
- 人脸数量、主脸清晰度和面积比例等样本质量信息；
- 同目录唯一临时文件写入和原子替换；
- 无脸时保留标注大图的降级结果。

## 安装与源码运行

核心协议没有第三方运行时依赖：

```powershell
python -m pip install -e "./python[dev]"
```

安装单图视觉处理能力：

```powershell
python -m pip install -e "./python[vision,dev]"
```

未安装包时，在仓库根目录运行：

```powershell
$env:PYTHONPATH = "python/src"
python -m scene_vault_ai health
'{"protocolVersion":1,"action":"health","payload":{}}' |
  python -m scene_vault_ai request
```

`request` 从标准输入读取一个 JSON object。标准输出始终只有一行 JSON；日志只写入
标准错误。成功退出码为 `0`，协议、配置或处理错误的退出码为 `1`。`request` 每次处理
一个请求后退出；常驻 worker 的分阶段方案见
[常驻 Python 视觉 Worker 决策](../docs/decisions/2026-08-11-persistent-python-worker.md)。

## 常驻 worker

`worker` 在同一个进程内从 stdin 逐行读取 JSON，并为每行向 stdout 写出一行响应：

```powershell
$requests = @(
  '{"protocolVersion":1,"requestId":"health-1","action":"health","payload":{}}',
  '{"protocolVersion":1,"requestId":"health-2","action":"health","payload":{}}'
) -join "`n"
$requests | python -m scene_vault_ai worker
```

- 输入输出均为 UTF-8 JSON Lines；空行忽略，单行最大 1 MiB。
- 单个请求的 JSON、协议或处理错误只返回该请求的错误响应，不终止 worker。
- stdin 到达 EOF 时正常退出；桌面端后续负责超时、崩溃重启与关闭生命周期。
- 请求串行执行。YuNet 按完整检测配置缓存，SFace / ArcFace 按模型路径分别缓存；配置
  或路径变化会替换对应旧实例，加载失败不会进入缓存，推理异常会淘汰对应实例以便
  下一请求重新加载。
- 进度继续写 stderr，格式为 `SVPROGRESS` JSON，并在请求提供 ID 时包含
  `requestId`。stdout 不允许写日志或非协议内容。

当前 Rust 主程序尚未切换到 `worker`，仍使用 `request` 作为稳定路径。这样可以先独立
验证 Python 多请求与缓存语义，再在下一阶段接入 Rust Worker Manager。

## processScreenshot

请求顶层只允许 `protocolVersion`、可选 `requestId`、`action` 和 `payload`：

```json
{
  "protocolVersion": 1,
  "requestId": "7d73d5f2-5536-4eb1-8fc3-f89496588a25",
  "action": "processScreenshot",
  "payload": {
    "inputPath": "D:\\Screenshots\\001.png",
    "annotatedOutputPath": "D:\\SceneVaultCache\\001.png",
    "avatarOutputPath": "D:\\SceneVaultCache\\avatars\\001.png",
    "characterName": "星见 Aurora",
    "detectFace": true,
    "annotate": true,
    "cropAvatar": true,
    "yunetModelPath": "D:\\SceneVaultModels\\face_detection_yunet.onnx",
    "recognizer": "sface",
    "sfaceModelPath": "D:\\SceneVaultModels\\face_recognition_sface.onnx",
    "annotation": {
      "fontPath": "D:\\SceneVaultResources\\NotoSansCJK-Regular.ttc",
      "fontSize": 48,
      "faceTextPosition": "above",
      "fallbackPosition": "top_left"
    },
    "crop": {
      "aspectRatio": "1:1",
      "scaleX": 1.8,
      "scaleTop": 1.3,
      "scaleBottom": 1.8,
      "minSize": 224
    }
  }
}
```

三个处理开关默认均为 `true`。字段约束：

- 所有路径必须是绝对、本地文件系统路径；拒绝相对路径、URI、UNC/NAS 和 Windows
  device namespace。
- 输入、标注输出和头像输出必须互不相同。
- `annotate=true` 时必须传入 `annotatedOutputPath` 和 `characterName`。
- `cropAvatar=true` 时必须启用检测并传入 `avatarOutputPath`。
- `detectFace=true` 时必须传入 `yunetModelPath`；模型由 Rust/安装包提供，不在
  Python 包中硬编码。
- `recognizer` 可选 `sface`（默认）或 `arcface`。SFace 只有在提供
  `sfaceModelPath` 时提取特征；ArcFace 必须提供 `arcfaceModelPath`。
- 输出支持 PNG、JPEG、WebP 和 BMP。

检测参数可通过可选的 `detection` object 调整；未知字段、错误类型、非有限数字和
越界参数会被拒绝。

标注位置可通过可选的 `annotation` 字段调整：

- `faceTextPosition`：检测到人脸时名字优先放置的方向，取值 `above` / `right` /
  `below` / `left` / `custom`，默认 `above`。前四者放不下时按固定顺序
  （上 → 右 → 下 → 左）降级。
- `textOffsetX` / `textOffsetY`：仅当 `faceTextPosition` 为 `custom` 时生效；
  名字中心相对人脸框中心的位置，单位为脸框宽度/高度的倍数，取值范围
  [-3, 3]。放不下时同样按固定顺序降级。
- `fallbackPosition`：未检测到人脸时名字落在画面的角落，取值 `top_left` /
  `top_right` / `bottom_left` / `bottom_right`，默认 `top_left`。

成功响应：

```json
{
  "protocolVersion": 1,
  "requestId": "7d73d5f2-5536-4eb1-8fc3-f89496588a25",
  "ok": true,
  "action": "processScreenshot",
  "data": {
    "inputPath": "D:\\Screenshots\\001.png",
    "imageWidth": 1920,
    "imageHeight": 1080,
    "annotatedPath": "D:\\SceneVaultCache\\001.png",
    "avatarPath": null,
    "faceDetected": false,
    "faceBox": null,
    "faceFeature": null,
    "faceFeatureModelId": null,
    "faceFeatureModelVersion": null,
    "faceCount": 0,
    "faceSharpness": null,
    "faceAreaRatio": null,
    "warnings": ["face_not_detected", "avatar_not_generated"],
    "timings": {
      "processorInitMs": 18.2,
      "readMs": 12.4,
      "detectMs": 35.6,
      "featureMs": 0.0,
      "annotateMs": 22.1,
      "cropMs": 0.0,
      "writeMs": 9.8,
      "processTotalMs": 80.7,
      "serviceTotalMs": 99.1
    }
  },
  "error": null
}
```

`faceFeature` 是可选特征向量。成功提取时，响应同时返回
`faceFeatureModelId` / `faceFeatureModelVersion`；Rust 必须按这两个字段和向量维度
隔离特征空间，不能跨模型比较。`faceCount` 是 YuNet 检出数量，建议仍基于主脸；
`faceSharpness` 和 `faceAreaRatio` 供 Rust 判断样本是否适合登记到 Face Bank。
`requestId` 由调用方生成并由引擎原样返回，用于关联 Rust 与 Python 日志；旧调用方可
省略。`timings` 是诊断基线，不属于业务判断依据，单位均为毫秒。

SFace 使用 `opencv-contrib-python-headless` 中的 `FaceRecognizerSF`；ArcFace 使用
用户提供的 ONNX 模型和 5 点对齐。特征提取失败只追加识别器对应的 warning，不影响
标注和头像输出。模型权重不随 Python 包分发。

没有检测到人脸是成功降级：标注大图仍会生成，头像路径为空。模型缺失、图片损坏或
写入失败才返回失败：

```json
{
  "protocolVersion": 1,
  "requestId": "7d73d5f2-5536-4eb1-8fc3-f89496588a25",
  "ok": false,
  "action": "processScreenshot",
  "data": null,
  "error": {
    "code": "image_decode_failed",
    "message": "screenshot could not be decoded",
    "details": {
      "path": "D:\\Screenshots\\001.png"
    }
  }
}
```

Rust 应根据 `error.code` 分类处理，不要解析英文 `message`。当前稳定错误码包括：

```text
invalid_json
invalid_request
invalid_payload
unsupported_protocol_version
unsupported_action
capability_unavailable
input_not_found
input_read_failed
resource_not_found
image_decode_failed
face_detection_failed
output_write_failed
internal_error
```

## 测试

```powershell
$env:PYTHONPATH = "python/src"
python -m unittest discover -s python/tests -v
python -m compileall -q python/src python/tests python/sidecar.py
```

常规测试使用 fake detector 验证无脸降级、主脸结果和头像裁剪，因此不需要提交模型
权重。持有 YuNet ONNX 模型时可额外运行真实模型 smoke test：

```powershell
$env:SCENE_VAULT_TEST_YUNET_MODEL = "D:\Models\face_detection_yunet.onnx"
python -m unittest python.tests.test_vision_processing.RealYuNetSmokeTests -v
```

## Windows Sidecar 交接

`sidecar.py` 是 PyInstaller 入口。构建环境需要安装：

```powershell
python -m pip install -e "./python[vision,packaging]"
pyinstaller --noconfirm --clean --onefile `
  --name scene-vault-ai `
  --paths python/src `
  --collect-all cv2 `
  --collect-all PIL `
  python/sidecar.py
```

可交付安装包还必须提供：

1. 经过真实 smoke test 的 YuNet ONNX 模型；
2. 允许再分发、覆盖中文字符的 Unicode 字体；
3. 与目标 Windows 架构一致的 Python/OpenCV/NumPy/Pillow runtime。

源码会优先使用请求中的 `annotation.fontPath`，未提供时才尝试 Windows 常见系统
字体。正式安装包不应仅依赖系统字体，因为精简版或非中文 Windows 不保证存在合适
字体。模型和字体最终路径由 Rust 解析为本地绝对路径并随每个请求传入。模型权重和
未确认许可的字体不会提交到仓库。
