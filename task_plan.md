# pe_transform 后端重构计划

## Goal

在 `feat/pe-transform-backends` worktree 中，为 `pe_transform` 增加 feature-gated 的读写后端抽象，支持 SurrealDB、Parquet、DuckLake 与对比模式，并保持默认生成路径行为不变。

## Current Phase

Phase 7

## Phases

### Phase 1: Requirements & Discovery

- [x] 安装 `planning-with-files` 到 Cursor/Codex。
- [x] 创建独立 worktree：`.worktrees/pe-transform-backends`。
- [x] 确认现有 `pe_transform` 刷新、查询和 feature 校验入口。
- **Status:** complete

### Phase 2: Feature & Runtime Surface

- [x] 在 `Cargo.toml` 增加 `transform-store-parquet`、`transform-store-ducklake`、`transform-store-compare`。
- [x] 在 `DbOptionExt`/CLI 增加 `transform_write_backend`、`transform_read_backend`、`transform_compare_backend` 及输出路径配置。
- [x] 复用 `validate_model_writer_features` 的模式新增 transform backend feature 校验。
- **Status:** complete

### Phase 3: Backend Abstraction

- [x] 新增 `PeTransformSink` / `PeTransformSource` 抽象。
- [x] 将现有 SurrealDB 写入封装为默认 sink/source，不改变当前默认行为。
- [x] 支持 `dual` sink，用于 SurrealDB + Parquet 双写对比。
- **Status:** complete

### Phase 4: Parquet Backend

- [x] 定义 `pe_transform.parquet` schema，覆盖 `refno/dbnum/local/world/hash/updated_at`。
- [x] 在 refresh batch flush 后按配置写 Parquet。
- [x] 支持从 Parquet 按 refno 加载，生成阶段 cache miss 可按配置读取并 prime 到 `transform_cache`。
- **Status:** complete

### Phase 5: DuckLake Backend

- [x] 使用 DuckLake 管理 Parquet 元数据，优先走“写 Parquet + `ducklake_add_data_files` 注册”的低耦合路径。
- [x] 默认按 `project_name, dbnum` 分区，避免过细 refno 分区。
- [ ] 提供 DuckLake 原生查询入口用于加载与版本对比；当前 ducklake 读路径先复用 Parquet source。
- **Status:** in_progress

### Phase 6: Compare & Benchmark

- [x] 增加 CLI 对比模式，读取同一批 refno/dbnum 的两个 backend。
- [x] 比较 local/world 矩阵误差、缺失数量、加载耗时。
- [x] 输出结构化摘要，便于比较 SurrealDB、Parquet、DuckLake 路径。
- [ ] 固定首轮基准为刷新 `dbnum=7997` 的 transform。
- [x] 对比前清理历史 `pe_transform` 数据，避免旧 transform 污染 backend 对比。
- **Status:** in_progress

### Phase 7: Verification & Handoff

- [ ] 按项目规则优先使用 CLI/真实接口验证，不新增 test。
- [ ] 验证流程必须包含：清理 `dbnum=7997` 历史 `pe_transform` -> 刷新 `7997` -> 写入目标 backend -> 读取对比。
- [ ] 在 Rust 工具链可用时执行最小 `cargo check`。
- [ ] 记录验证命令、输入 dbnum/refno、输出耗时和剩余风险。
- **Status:** in_progress

## Key Questions

1. DuckLake 首版是否只做注册和查询，还是需要 Rust 侧直接依赖 DuckDB/DuckLake 写入？
2. Parquet schema 是否采用完全展开矩阵列，还是保留 hash + 单独 transform 表做规范化？
3. 对比基线使用哪些 dbnum/root_refno，是否固定 `DbOption-cli.toml` 当前样本？（已定：首轮使用 `dbnum=7997`）

## Decisions Made

| Decision | Rationale |
|----------|-----------|
| 默认行为保持 SurrealDB | 避免影响现有生成、Web API 和 `pe_transform` 依赖查询。 |
| feature 控制能力、CLI/配置控制本次 backend | 保持编译依赖可控，同时支持同一二进制做多种实验。 |
| 生成热路径统一 prime 到 `transform_cache` | 对比加载/预热成本，避免几何生成逻辑分叉。 |
| DuckLake 首选“外部 Parquet + add_data_files” | 与 `ducklake` 示例/测试一致，降低 Rust 侧直接集成风险。 |
| 首轮对比固定刷新 `dbnum=7997` | 用户指定该 dbnum，便于控制样本和复现实验。 |
| 对比前必须清理历史 `pe_transform` | 避免 SurrealDB 中旧 transform 与新 Parquet/DuckLake 数据混用，导致误判。 |

## Errors Encountered

| Error | Attempt | Resolution |
|-------|---------|------------|
| `cargo` not recognized | 1 | 当前 PowerShell PATH 无 Rust 工具链，已用 `ReadLints` 和 `git diff --check` 做静态检查；需在 cargo 可用环境补跑 `cargo check`。 |
| git dependency update stalled | 1 | 使用 `D:/Rust/.cargo/bin` 后 `cargo check` 卡在多个 git 依赖；已为 indextree/miniacd/rvm-rs/surrealdb/calamine/cavalier_contours/id_tree 增加本地 patch。 |
| `rust-ploop-processor` unavailable | 1 | `rs-core` 依赖 `https://github.com/happyrust/rust-ploop-processor`，本机未找到本地仓库，在线更新长时间无输出；需提供本地仓库或恢复网络。 |

## 下一步详细开发方案

### 目标

把当前“可编译性未知的主体实现”收敛成可验证、可比较、可交付的 `pe_transform` 多后端实验能力。首轮验收只要求 `dbnum=7997` 在 SurrealDB 与 Parquet 之间完成清理、刷新、双写、读取对比和耗时 profile；DuckLake 首版先验证注册脚本与文件布局，原生 time-travel 查询作为后续增强。

### Phase 8: 恢复验证环境

- [ ] 让当前终端可用 `cargo`、`rustc`。
- [ ] 确认 SurrealDB 可连接，优先使用 `DbOption-cli.toml` 当前配置。
- [ ] 如需 DuckLake 验证，安装或暴露 `duckdb` CLI，并确认 `INSTALL ducklake; LOAD ducklake;` 可执行。
- [ ] 记录实际工具路径、版本和数据库监听状态。
- **Status:** pending

### Phase 9: 编译收敛

- [x] 使用用户指定的 `D:/Rust/.cargo/bin` 恢复 Cargo/Rust 命令可用。
- [ ] 执行最小编译检查：
  `cargo check --bin aios-database --features "review,transform-store-parquet,transform-store-compare"`
- [ ] 如果 `transform-store-ducklake` 只生成 SQL 脚本，也补跑：
  `cargo check --bin aios-database --features "review,transform-store-ducklake,transform-store-compare"`
- [ ] 修复所有编译错误，不引入 test，不改无关模块。
- [ ] 再跑 `ReadLints` 与 `git diff --check`。
- [ ] 先解除 `rust-ploop-processor` 获取阻塞：提供本地 path patch 或恢复 GitHub 网络访问。
- **Status:** pending

### Phase 10: SurrealDB vs Parquet 首轮对比

- [ ] 执行清理 + 刷新 + 双写 + 对比：
  `cargo run --bin aios-database --features "review,transform-store-parquet,transform-store-compare" -- -c db_options/DbOption-cli --refresh-transform 7997 --clear-transform-before-refresh --transform-write-backend dual --transform-compare-backends surreal,parquet`
- [ ] 记录输出：处理节点数、清理 refno 数、SurrealDB loaded/missing、Parquet loaded/missing、mismatched、max_delta、elapsed_ms。
- [ ] 若 mismatch > 0，按 refno 采样定位：比较 local/world 矩阵列、hash、refno->dbnum 映射。
- [ ] 若 missing > 0，先确认 Parquet 文件是否覆盖所有 batch，再检查读取路径是否递归扫描到分区目录。
- **Status:** pending

### Phase 11: Profile 耗时热点

- [ ] 在 CLI 输出中区分并记录这些阶段：
  - 清理历史 `pe_transform`
  - 计算 local/world transform
  - SurrealDB 写入
  - Parquet 写入
  - transform_cache prime
  - SurrealDB 读取
  - Parquet 读取
  - compare 矩阵误差计算
- [ ] 如果当前代码输出粒度不够，补最小 `PerfTimer` 或 `Instant` 计时，不改业务逻辑。
- [ ] 形成耗时表，定位主要瓶颈是计算、DB 写入、Parquet 写入还是读取对比。
- **Status:** pending

### Phase 12: DuckLake 注册验证

- [ ] 使用 `--transform-write-backend ducklake` 刷新 `7997`，确认生成 Parquet 和 `register_pe_transform.sql`。
- [ ] 用 `duckdb` 执行注册脚本。
- [ ] 查询 DuckLake 表行数、分区文件和 snapshot：
  - `SELECT COUNT(*) FROM lake.pe_transform WHERE dbnum = 7997;`
  - `FROM ducklake_snapshots('lake');`
- [ ] 若 DuckLake 注册成功，再决定是否实现原生 source；否则保持“Parquet source + DuckLake metadata 管理”的首版边界。
- **Status:** pending

### Phase 13: 输出对比表与交付

- [ ] 在 `progress.md` 记录真实命令、环境版本、输出摘要。
- [ ] 在 `findings.md` 记录结论性发现：性能瓶颈、一致性风险、DuckLake 可行性。
- [ ] 生成最终对比表：
  `Backend | Write Time | Read Time | Loaded | Missing | Mismatched | Max Delta | Notes`
- [ ] 标记 Phase 7-13 完成或记录剩余阻塞。
- **Status:** pending

### 验收标准

- `cargo check` 通过。
- `dbnum=7997` 对比前会清理历史 `pe_transform`。
- SurrealDB 与 Parquet 对比输出至少包含 loaded/missing/mismatched/max_delta/elapsed_ms。
- Parquet 输出路径按 `project_name/dbnum` 分区，能被读取路径递归扫描。
- 若 DuckLake CLI 可用，注册脚本能创建/注册 `lake.pe_transform`。
- 所有验证结果写入 `progress.md`，稳定结论写入 `findings.md`。
