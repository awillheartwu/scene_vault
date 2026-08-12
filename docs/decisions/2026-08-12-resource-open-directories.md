# 资源与缓存目录打开修复决策

日期：2026-08-12

## 背景

设置页“资源与缓存”的 8 个“打开位置”按钮点击后报错。查日志发现这些错误只出现在
前端 toast，Rust JSONL 日志没有任何记录：opener 插件调用绕过 Tauri invoke 包装，
前端 catch 也没有写客户端日志，因此问题无法从日志定位。

静态检查还发现两个具体缺陷：

- “Python 运行环境”在未配置 pythonExecutablePath 时回退到
  app_local/ai-runtime，该目录在打包安装中不存在；opener 插件的 open_path
  对不存在的路径直接返回 IO 错误。
- 主窗口 capability 依赖 opener:default 隐式授予 reveal 权限，未显式声明
  opener:allow-reveal-item-in-dir。

## 决策

- src-tauri/capabilities/default.json 显式声明 opener:allow-open-path 与
  opener:allow-reveal-item-in-dir，不依赖 opener:default 的隐式展开。
- resource_paths 中 python_runtime 优先使用配置的 Python 可执行文件目录；
  未配置时使用内置 sidecar（scene-vault-ai.exe）所在目录；两者都不可用时才回退
  ai-runtime。
- resource_storage_service::scan 对不存在的路径返回 path 为 null，界面显示
  “尚未配置”并隐藏打开按钮，避免点击必错的死按钮。
- 前端打开目录失败时通过 record_client_event 写入 ui.opener 模块，与现有
  ui.ipc 客户端日志一致；路径内容仍由客户端日志脱敏。

## 结果

修复后所有“打开位置”按钮要么打开真实存在的目录，要么不渲染；点击失败的原因可以
从应用日志直接看到。需要重新构建并安装后才能验证。

## 补充：opener 插件的 scope 限制与 open_directory 命令（2026-08-12）

实机验证发现上述修复后按钮仍报 “Not allowed to open path <路径>”：tauri-plugin-opener
2.5.4 的 `open_path` 命令除 ACL 权限外还有一层 **scope 白名单**（`opener:allow-open-path`
权限自带说明即 “without any pre-configured scope”，scope 为空时所有路径都被拒绝）。
scope 只能通过 capability 权限参数静态配置，插件没有运行时注册 API；而应用的模型目录、
Python 目录可由用户配置到任意位置（本机模型目录在 `D:\09_temps\...`），静态白名单
无法覆盖，把用户路径写进仓库更不可接受。

因此新增第一方命令 `commands::system::open_directory`：校验 `path.is_dir()` 后用
`explorer`（Windows）打开目录本身，文件仍走插件 `reveal_item_in_dir`（该命令无 scope
校验）。前端 `openPathExternal` 更名为 `openDirectoryExternal` 并改调新命令，所有目录
打开按钮（资源条目/备份/缓存/日志/恢复页数据目录）不再依赖插件 scope。`opener:allow-open-path`
权限因此已无调用方，保留无害；后续不要再从前端调用插件 `open_path`。
