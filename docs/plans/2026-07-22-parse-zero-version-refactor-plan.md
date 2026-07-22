# 解析零版本重构方案：版本仅由增量产生，单源 Surreal MVCC + 锚点

> grill-with-docs 定案产物（2026-07-22）。决策记录：ADR-0007（supersede ADR-0002）、ADR-0008（supersede ADR-0003）；术语变更已入根 `CONTEXT.md`。
> **实施规格已转 speckit：`specs/027-version-single-source-refactor/`（spec/plan/tasks 三件套，为实施口径之准）；本文保留定案过程与 M6 伴生加固轨道。**
> 原则一句话：**全量解析不写任何版本痕迹；数据版本与模型版本都只由增量链路以增量生成的方式产生；版本真相单源 = SurrealDB(RocksDB) MVCC + `sesno_version_anchor`。**

## 定案汇总（Q1–Q6）

| # | 决策 | 结论 |
|---|------|------|
| Q1 | 解析路径版本痕迹边界 | **彻底零痕迹**：删解析期 DuckLake 重链与 `source='full'` 锚点；只留 `dbnum_info`/`db_meta` 的 `latest_sesno`（增量起点既有回退源） |
| Q2 | 数据版本权威归属 | **DuckLake 整体退役**，单源 = Surreal MVCC + sesno 锚点；`model_unit_commit` 台账搬 Surreal（ADR-0005 语义不变） |
| Q3 | 模型生成读路径 | **主表直读 + 保留领域查询 trait**；增量运行单一 `VERSION AT`（取数据提交锚点 `anchored_at`）钉住，全量活读；`generation_replica_*` 五表全删 |
| Q4 | 模型水位起点握手 | **全量生成成功收尾发 `model_gen` 锚点**（sesno=解析基线）；即现有 `publish_model_gen_anchors_after_generation` 行为，固定为原则 |
| Q5 | 存量站点/配置切换 | **硬切分级拒绝**：行为键（`parse_storage_backend=ducklake`、`generation_read_backend=ducklake\|compare`）启动硬错误；惰性 `ducklake_*`/`duckdb_*` 键仅警告；磁盘 `.ducklake` 遗物只读保留不迁移 |
| Q6 | 台账存量数据 | **零迁移**：Surreal 新表空表起步；旧账随遗物封存；外置 duckdb CLI 脚本作可选救济（不进主二进制） |

## 改造面（里程碑）

### M1 解析零痕迹
- 删除全量解析成功路径上的 `source='full'` 锚点固化（specs/022 T010 引入点）。
- 删除解析期 DuckLake 链调用（`sync_pdms_to_ducklake` / parse_staging 入口，`versioned_db/database.rs` 等）。
- 校验 `committed_watermark()` 回退链在"无 full 锚点"下的新站首增量路径（watermark=dbnum_info 基线 → ContinuityGap 门禁自然衔接）。

### M2 DuckLake 退役删除面
- `src/version_store/`（authority/bootstrap/legacy_bridge/parse_staging/replica/schema）整体删除；`model_unit_commit.rs` 单独走 M4。
- `src/generation_read/`：删 `ducklake.rs`、`compare.rs`、factory 的 ducklake/compare 分支、catalog 的 snapshot 绑定；保留 `traits.rs`、`hierarchy.rs` 等共享领域逻辑与 `surreal.rs`（M3 改造基础）。
- `Cargo.toml`：删 `generation-read-ducklake` feature 与 `dep:duckdb`；同步清理 full/打包脚本 FEATURES 列表与 DuckDB 扩展资产（`build-windows-bundle.ps1`、deploy 脚本）。
- `version_commit.rs` 删 `committed_watermark` 的 ducklake fork 与 `publish_authority_after_apply` 的 feature 分支。
- CLI 删 `bootstrap-generation-read` 等子命令；web 删 `model_runtime.rs` 的 `ducklake_config()` 调用点。
- `options.rs`：删 `GenerationReadBackendMode::Ducklake|Compare`、`ParseStorageBackend::Ducklake`、`ducklake_*`/`duckdb_*` 字段与默认值函数；在 `get_db_option_ext` 校验链（`validate_data_source_mode` 旁）加启动检测：行为键残留=硬错误（报修复指引），惰性键残留=警告。
- 删 `scripts/smoke/generation_read_perf_gate.ps1` 及两个 `.fixture`。
- **前置**：全仓删除清单盘点（已定义任务，待工作组恢复后派 plant-model-gen-2 产出 `docs/plans/2026-07-22-ducklake-retirement-deletion-manifest.md` 核对本节遗漏）。

### M3 生成读路径
- 新适配器实现既有 trait：直读主 PE/ATT 表（走长期生产验证的 query 层）。
- 增量生成：会话打开取本次提交锚点 `anchored_at`，全部读加 `VERSION AT T`；全量生成活读。
- 输入版本清单降级为观测记录：写进运行结果 summary，删除 fail-closed 覆盖校验。
- 删 `generation_replica_*` 五表 schema 与复制链引用。

### M4 交付台账搬家
- 模型同库新建 Surreal `model_unit_commit` 表（字段对齐现 DuckLake 版：unit 身份 `(dbnum, unit_refno, sesno)` + impact kind + 导出物指针）。
- 改造调用方：`version_management/cli.rs`（6 处）、`web_server/model_runtime.rs`（7 处）。
- 新增可选救济脚本 `scripts/migrate_unit_ledger_from_ducklake.ps1`（外置 duckdb CLI 导 JSON → CLI 导入），文档标注"仅确需旧账的站点手工执行"。

### M5 文档与规格
- ✅ ADR-0007/0008 已写入；ADR-0002/0003 已标 superseded；CONTEXT.md 术语已收敛（权威版本库/读副本/版本读取会话/两种 snapshot 绑定 → 废除区；输入版本清单降级；新增"生成读取时刻"）。
- specs/024（unified-rocksdb-versioning）、specs/025（versioned-generation-read-session）加状态注记指向 ADR-0007/0008；specs/022 ops-notes 与 AGENTS.md 的增量说明段核对措辞（Committed Watermark 语义未变）。
- ADR-0006 不动（水位/欠账闭环不受影响，其 depends_on 是历史记录）。

### M6 伴生加固（独立可执行，来自 2026-07-22 brooks-review）
- **锁进 seam**：`run_increment` 在 `persist_data || generate_model` 时自持 `ProjectMutationLock`（支持调用方声明已持有），封死 HTTP 生成路径（`incremental_update_handlers.rs`、`stream_generate.rs`）绕锁并发窗口。
- **活动 db 文件解析收敛**：删 `sesno_increment.rs` 中 `inactive_db_path`/`db_candidate_rank`/`discover_active_db_file_for_dbnum` 三份拷贝，统一走 db_index（discover 仅作索引 miss 显式回退且与预扫描同规则）。
- 其余 findings（错误子串匹配→结构化 kind、pe 行 builder 收敛、水位查询去重、unclassified 计数）按 review 报告顺序择机处理；其中"committed_watermark ducklake fork"与"状态端点水位二次实现"两条随 M2 自然消灭。

## 验证门（仓库规则：禁 cargo test）
1. 构建组合：默认 build、`--features full`、`powershell -File scripts/build-sync-cli.ps1`。
2. 冒烟：`scripts/smoke/unified_versioning_e2e.ps1`、`anchor_continuity_audit.ps1`、`pe_owner_incr_shapes_smoke.ps1`。
3. web_server 起服务后 HTTP 实测：`/api/incremental/*` 状态与 sync/detect、`execute_incremental_update`（ParseOnly 与 ParseAndModel 各一次）。
4. 配置检测手测：带 `generation_read_backend=compare` 的 toml 启动必须硬错误且指引明确；带惰性 `ducklake_*` 键启动必须仅警告。
5. 新站剧本：全量解析 → 确认零版本痕迹（无 full 锚点）→ 全量生成 → 确认 model_gen 起点锚点 → 首次增量 → watermark 从 dbnum_info 基线衔接、欠账闭环不误报。

## 风险与顺序
- **当前工作区未检测到 git 仓库**：M2 删除面动工前先初始化版本控制或做目录快照，这是本方案唯一的回滚保障。
- 顺序建议：盘点清单 → M2（删除面，编译门通过）→ M1 → M3 → M4 → M5 收尾 → M6 随时插队。M2 先行是因为删除面最大、编译器会把 M1/M3 的遗漏点暴露出来。
