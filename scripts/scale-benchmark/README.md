# Scene Vault 1.0 规模基准

对应 [ROADMAP.md](../../docs/ROADMAP.md) M6 的“1.0 规模守门”：在单项目
10,000 张截图、10,000 文件来源目录、100 个角色下，测量启动、历史页、Workbench/
分类列表、Face Bank 建议、目录扫描和数据库体积，输出可复现的机器可读 JSON。

## 为什么是独立脚本

基准工具完全由**新增文件**组成（`scripts/scale-benchmark/`），不改动现有源码。
它通过重放 `src-tauri/migrations/` 的真实迁移构建 SQLite fixture，并逐字复用
生产服务中的 SQL 语句（每个语句都标注了来源文件）。代价是：测量走 Python
`sqlite3`，而不是 Rust/sqlx 服务代码，因此 CPU 密集指标（Face Bank 余弦、目录
扫描）是**产品行为的代理（proxy）**，具体差异见下文“口径与限制”。

## 运行

重型运行固定到 CPU 0-3（与仓库 Windows/WSL 约定一致）：

```bash
cd /mnt/c/Profile/02_projects/sv/scene_vault
taskset -c 0-3 python3 scripts/scale-benchmark/benchmark_scale_1_0.py \
  --out docs/benchmarks/scale-1.0-report-2026-08-11.json
```

Windows PowerShell（无需 `taskset`，或自行设置进程亲和性）：

```powershell
python scripts\scale-benchmark\benchmark_scale_1_0.py `
  --out docs\benchmarks\scale-1.0-report-2026-08-11.json
```

JSON 报告同时输出到 stdout；退出码 0 表示全部门禁通过，1 表示存在未达标的门禁。

## 参数

| 参数 | 默认 | 说明 |
|---|---|---|
| `--out PATH` | 无 | 另存 JSON 报告 |
| `--fixture-dir PATH` | 临时目录 | 指定 fixture 目录（自动重建，幂等） |
| `--keep-fixture` | 关 | 保留 fixture 目录 |
| `--repetitions N` | 5 | 每个计时指标的采样次数 |
| `--items N` | 10000 | capture_items 数量 |
| `--characters N` | 100 | 角色数量 |
| `--scan-entries N` | 10000 | 来源目录条目数（0 表示跳过扫描基准） |

## 门禁阈值

| 指标 | 阈值 | 对应产品门禁 |
|---|---|---|
| `startup_db_init_ms` | < 5000 ms | 启动 < 5 s（仅 DB 初始化部分） |
| `history_backend_ms` | < 200 ms | 深分页 `OFFSET 9000 LIMIT 100` |
| `face_bank_suggestion_ms` | < 150 ms | 建议/验证单次匹配 |
| `scan_filesystem_ms`、`scan_full_ms` | < 1000 ms | 10k 目录轮询 |
| `workbench_character_grid_ms`、`category_*_ms` | < 1000 ms | Workbench 网格/分类页 |
| `db_main_bytes` | <= 100 MB | 10k 行数据库体积预算 |
| `prelabel_next_awaiting_ms` | < 100 ms | 预标注下一项查询（软门禁） |
| `fixture_build_ms` | < 300000 ms | 基准自身可重复性 |

其余指标（`home_overview_ms`、`recent_items_100_ms`、`list_session_all_ms`、
`face_bank_verify_ms`、`rebuild_refresh_estimate_ms`、`db_total_bytes`、
`sample_bank_json_bytes`）只报告不判级，供趋势观察。

## 测量内容

- **fixture 构建**：重放 19 个迁移 + 创建 10,000 文件来源目录 + 插入 10,000
  capture_items、10,000 capture_faces、8,000 Face Bank samples、10,000 基线行。
- **启动类**：全新连接 + WAL + `quick_check` + 关键表计数（`db::initialize` 代理），
  Home 总览查询、最近 100 条。
- **历史页**：`list_history` 的 COUNT + 深分页（无筛选）。
- **Workbench/分类**：`list_character_items`（最大角色 1,200 条）、
  `list_category_items`（unclassified/scene/private，非分页）、`list_items`
  （整会话 10k 行，仅报告）。
- **Face Bank**：`suggest_from_face_bank` 样本加载 + JSON 解析 + 余弦 + 按角色取
  max + 排序判阈；`compute_verification` 代理；重建刷新全量估算。
- **目录扫描**：`discover()` 的文件工作（read_dir + stat + 过滤 + 排序 +
  二次 stat + realpath）与稳态轮询（含每条基线点查）。

## 口径与限制

1. **Python 代理**：SQL 与生产一致，但执行路径是 Python `sqlite3`。Face Bank
   余弦使用 `math.fsum`/`operator.mul` 的 C 级实现避免 Python 循环放大；
   即便如此，代理通常仍比 Rust 服务慢，Windows/Rust 实机测量应视为权威口径。
2. **建议写入被排除**：`set_suggestion` 是单行 UPDATE，未计入。
3. **p95 为小样本 p95**（默认 N=5），判级用中位数。
4. **合成数据**：确定性特征向量、均匀时间戳、单项目/单会话、1 字节占位图片，
   无缩略图、标注图、归档文件。
5. **迁移校验未复刻**：不模拟 sqlx 的 checksum/dirty 校验和连接池。
6. **扫描在本地临时盘**（WSL `/tmp`）执行，不覆盖 NAS/UNC 延迟。
7. **启动 < 5 s 含 WebView2/UI**，本工具只测 DB 初始化段。

## 参考来源

- `src-tauri/src/services/capture_service.rs`（history / category / recent /
  prelabel / baseline）
- `src-tauri/src/services/recognition_service.rs`（character grid / suggestion /
  verification）
- `src-tauri/src/services/capture_discovery_service.rs`（scan）
- `src-tauri/migrations/`（schema）
- `docs/ROADMAP.md` M6、`docs/RESOURCE_USAGE.md`（口径）

报告模板见
[docs/benchmarks/scale-1.0-report.example.json](../../docs/benchmarks/scale-1.0-report.example.json)
（示例包含全部门禁指标与代表性只读指标；完整运行输出含所有指标及
`startup_quick_check_ms`、`history_count_ms`、`face_bank_verify_ms` 等明细）。
