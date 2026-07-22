# DuckLake 退役删除清单（specs/027 T001）

> 产出：2026-07-22，specs/027-version-single-source-refactor T001。逐项经 grep/读文件核实（行号为当日基线 `338b546e`，不代表当前完成状态）。
> 用途：退役范围基线；实施前必须按 specs/027 T004 重新核对路径/符号并补状态栏，删除顺序以当前 `specs/027-version-single-source-refactor/plan.md` 为准。

## A 整文件删除

| 路径 | 说明 / 依赖影响 |
|------|----------------|
| `src/version_store/authority.rs` | DuckLakeAuthority：权威 snapshot 提交/读取、duck 版 committed_watermark；被 cli.rs/model_runtime.rs/database.rs 引用 |
| `src/version_store/parse_staging.rs` | 解析期 staging→指纹 seal 链；被 database.rs 解析路径引用 |
| `src/version_store/replica.rs` | `generation_replica_element/hierarchy/reference/transform/db_catalog` 五表 schema 与原子复制链；被 database.rs:61-63、generation_read/surreal.rs:19 引用 |
| `src/version_store/legacy_bridge.rs` | legacy applied state 发布桥（version_commit.rs `publish_authority_after_apply` ducklake 分支的实现体） |
| `src/version_store/bootstrap.rs` | current-state bootstrap（`bootstrap-generation-read` 核心） |
| `src/version_store/schema.rs` | DuckLake catalog/alias schema（`DUCKLAKE_CATALOG_ALIAS`） |
| `src/version_store/model_unit_commit.rs` | **暂留**：T007 台账搬 Surreal 完成后随 `mod.rs` 一并删除 |
| `src/version_store/mod.rs` | 收尾删除（model_unit_commit 搬走后目录清空） |
| `src/generation_read/ducklake.rs` | DuckLake 读适配器（feature-gated） |
| `src/generation_read/compare.rs` | 双后端对比适配器（`ComparingVersionedReadBackend/Session`、`TRANSFORM_ABS_TOLERANCE`） |
| `scripts/smoke/generation_read_perf_gate.ps1` | 双后端 perf gate |
| `scripts/smoke/generation_read_perf_gate_baseline.fixture` / `..._candidate.fixture` | perf gate 夹具 |

## B 文件内局部删改

| 路径:位置 | 动作 |
|-----------|------|
| `Cargo.toml:170` | 删 `duckdb = { version = "~1.10504.0", features = ["bundled"], optional = true }` |
| `Cargo.toml:312` | 删 `generation-read-ducklake = ["dep:duckdb"]`（`full`(247) 与 `sync-cli`(195) 均不含它，无连带） |
| `src/options.rs:21-27` | 删 `default_generation_read_backend` / `default_parse_storage_backend` |
| `src/options.rs:29-58` | 删 `ducklake_*` 五个默认值函数 + `duckdb_memory_limit/threads/pool_size` 默认值函数 |
| `src/options.rs:208-294` | 删 `GenerationReadBackendMode`、`ParseStorageBackend` 两枚举及 `parse_generation_read_backend` / `parse_parse_storage_backend` |
| `src/options.rs:497-531` | 删 `generation_read_backend`、`parse_storage_backend`、`generation_input_manifest`、`ducklake_*`×5、`duckdb_*`×3 字段 |
| `src/options.rs:774-826` | 删 manifest/needs_ducklake/uses_ducklake 三段校验；**同点位新增分级检测**：行为键（`parse_storage_backend=ducklake`、`generation_read_backend=ducklake\|compare`）硬错误+修复指引，惰性键（`ducklake_*`/`duckdb_*`/`generation_input_manifest`）仅警告（raw TOML 检测，serde 已不识别这些键） |
| `src/options.rs:833-8xx` | 删 `ducklake_config()`（返回 `version_store::DuckLakeConfig`） |
| `src/options.rs:894-895/1110-1117/1304-1305/1376-1381/1422` | 删 From 默认、raw 提取、打印、test 导入中的对应项 |
| `src/versioned_db/version_commit.rs:347-365` | 删 `publish_authority_after_apply` 的 ducklake 分支，保留 no-op（或内联移除调用点） |
| `src/versioned_db/version_commit.rs:877-885` | 删 `committed_watermark` 的 ducklake fork，保留 Surreal 实现（897+） |
| `src/versioned_db/database.rs:61-63` | 删 `version_store::replica`/`version_store` use |
| `src/versioned_db/database.rs:899-904/1091-1093` | 删 `parse_storage_backend.uses_ducklake()` 分流与 `sync_pdms_to_ducklake` 调用 |
| `src/versioned_db/database.rs:2153-2417+` | 删 staging counts/rows 构造与 `sync_pdms_to_ducklake` 函数体 |
| `src/versioned_db/database.rs:702-770 及调用点` | **T005（解析零痕迹）**：删 `write_full_version_anchors`（`source='full'` 锚点固化，specs/022 T010）；保持 join 语义与 `pe_owner_version_meta` 写入不动（后者是 pe_owner 可信起点，非版本痕迹——若随 full 锚点绑定写入需拆开） |
| `src/version_management/cli.rs:248-282/402/1130-1238` | 删 `bootstrap-generation-read` 子命令（定义/分发/实现/无 feature bail） |
| `src/version_management/cli.rs:501-506/710/922/962` | **T007**：`ModelUnitCommit/ModelUnitImpactKind/DuckLakeAuthority` 调用改 Surreal 台账 |
| `src/version_management/mod.rs:1` | 删 feature gate |
| `src/main.rs:1392` | 删 bootstrap-generation-read 匹配臂 |
| `src/web_server/model_runtime.rs:188-190/231-233/281-283/320` | **T007**：`DuckLakeAuthority::open_readonly` 三处 + `model_unit_commit_json` 改读 Surreal 台账 |
| `src/generation_read/mod.rs:7-8/17-21` | 删 ducklake mod/pub use 与 compare pub use |
| `src/generation_read/factory.rs`（6 处） | 删 ducklake/compare 分支；`resolve_input_version_manifest` 改为观测记录构造（T006） |
| `src/generation_read/catalog.rs`（5 处） | 核对并删 snapshot 绑定引用，保留 CatalogResolver 领域逻辑 |
| `src/generation_read/surreal.rs:19` | 删 `ReplicaSnapshotBinding/SurrealReplicaStore` use；改造为主表直读适配器（T006） |
| `src/fast_model/gen_model/orchestrator.rs:269-289/1026` | 删按 backend 分流的 pe_transform 模式；summary 字段改观测记录 |
| `src/fast_model/gen_model/config.rs:124/604` | 生成契约去 read_backend 维度；删 test 中 DuckLake 值 |
| `src/fast_model/gen_model/cata_model.rs:814`、`context.rs`、`gen_pipeline.rs` | 保留 `open_generation_read_session` trait 调用，实现随 T006 换主表直读 |

## C 配置与脚本

| 路径:位置 | 动作 |
|-----------|------|
| `db_options/DbOption.toml:33-44` | **行为键** `parse_storage_backend="ducklake"` + 惰性键组；重构后此样例会触发硬错误——随 T004 改为无该键版本 |
| `db_options/DbOption-ams7997-gen.toml:33-45` | 同上；另含 `generation_input_manifest = "db-data/ams7997-generation-manifest.json"`（惰性键，删行） |
| `scripts/package/build-windows-bundle.ps1:10-14/37-39/49-55/57` | 删 DuckDB 扩展参数、core 版本钉、扩展缓存段；`$Features` 去 `generation-read-ducklake`；核对后续 bundle 拷贝段的扩展资产复制 |
| `scripts/smoke/unified_versioning_e2e.ps1:423-427` | 「必须不存在」守卫清单更新：加入 `src/version_store/`、`src/generation_read/{ducklake,compare}.rs` |
| **保留** `scripts/package/verify-offline-viewer.ps1`（duckdb-wasm 资产） | 前端离线 parquet 查看器依赖，与版本权威无关，不在退役范围 |
| **保留** `src/options.rs:134/150` transform_write/read_backend=ducklake 拒绝文案 | 更早退役的另一开关，语义独立，保持现状 |

## D 文档与规格

| 路径 | 动作 |
|------|------|
| `docs/adr/0002`、`docs/adr/0003` | ✅ 已标 `superseded by ADR-0007/0008` |
| `CONTEXT.md` | ✅ 词条已收敛（权威版本库/读副本/版本读取会话/两种 snapshot 绑定→废除区；输入版本清单降级；新增生成读取时刻） |
| `specs/024-unified-rocksdb-versioning/`、`specs/025-versioned-generation-read-session/` | T009：头部加状态注记指向 ADR-0007/0008 |
| `specs/022-versioned-pe-att-storage/ops-notes.md`、`AGENTS.md` | T009：核对 DuckLake/full 锚点提及（Committed Watermark 语义未变） |
| `docs/guides/MIGRATION_GUIDE.md`、`CHANGELOG.md` | T009：硬切+配置检测+解析零痕迹三处行为变化与运维口径 |

## 编译验证门

1. 默认 `cargo build`；2. `cargo build --features full`；3. `powershell -File scripts/build-sync-cli.ps1`（sync-cli 不含退役 feature，应零波动）；4. `cargo tree -i duckdb` 无结果；5. 带行为键/惰性键两个最小 toml 的启动检测手测（禁 cargo test，走 CLI 实测）。

## 关键风险点

- `write_full_version_anchors` 删除时**只去锚点、不动 join 语义**；`pe_owner_version_meta`（full_reload 可信起点）若与锚点同一收尾块需拆开保留——否则 pe_owner 完整性证据（specs/023 M3/T8）失去 full 侧起点。
- `resolve_input_version_manifest` 有非 ducklake 调用方（factory 对 Surreal 分支同样走它）；T006 改造为观测记录时保持 summary 字段形状（`orchestrator.rs:1026`）向后兼容。
- spec-026（欠账追赶）实施若与 T006 并行，生成入口 VERSION AT 注入点会撞（`open_generation_read_session` 周边），后合方 rebase。
- `generation_read/mod.rs` 的 boundary_tests 是 `#[cfg(test)]`，不进构建门（仓库规则禁编译 test），删改 adapters 不需要动它。
