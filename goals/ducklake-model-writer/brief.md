# DuckLake ModelWriter Backend

## Outcome

让 `ModelWriter` 拥有一个 DuckLake 后端实现 `DuckLakeModelWriterBackend`，通过 Rust `duckdb` crate 直接打开 DuckDB / 挂载 DuckLake metadata，把当前 `ModelWriterBackend` trait 已覆盖的 8 个生命周期阶段产出的 canonical raw 行写入 `ducklake-canonical` schema 下的 raw 表；默认 Surreal 路径与已完成的 `model-writer-backend-abstraction` goal 行为按字节级兼容，drain-only 模式不被影响。

## Context

- `model-writer-backend-abstraction` goal 已经完成（`goals/model-writer-backend-abstraction/progress.jsonl` final_acceptance_audit=passed @ 2026-05-11T15:34）：`src/fast_model/gen_model/model_writer.rs::ModelWriterBackend` trait 已覆盖 `init / cleanup / write_base_batch / persist_mesh_results / persist_inst_relate_aabb / reconcile_missing_neg_relations / run_boolean_bridge / finalize` 8 个生命周期阶段；`SurrealModelWriterBackend` 与 `DrainOnlyModelWriterBackend` 是已交付的 2 个 backend；`orchestrator.rs` 不再直接调持久化 helper。
- mission 文档已就位于 `.factory/mission-docs/model-writer-storage/`（00 mission overview，01 surreal contract，02 canonical schema，03 writer architecture，04 ducklake writer，05 parquet writer，06 orchestrator integration，07 validation plan，08 phase roadmap，09 phase-1 tubi migration）。
- `04-ducklake-writer.md` 明确方向：DuckLake 最终架构必须走 Rust DuckDB binding 直接写 canonical 表，不接受 temp-Parquet-plus-SQL 作为最终路径；并要求挂载 DuckLake metadata、创建 `ducklake-canonical` schema、批事务写 raw、SQL 刷新 projection、暴露 CLI + SQL 验证。
- `02-canonical-schema.md` 列出 Phase 1 raw 表 13 张（`raw_inst_info / raw_inst_relate / raw_inst_geo / raw_geo_relate / raw_tubi_info / raw_tubi_relate / raw_neg_relate / raw_ngmr_relate / raw_aabb / raw_trans / raw_vec3 / raw_inst_relate_aabb / raw_refno_assoc_index`）与 projection 表 9 张；`ducklake-canonical` 为 DuckLake 命名空间。
- `03-writer-architecture.md` 第 4 节「Canonical tables × backend stages」标出 5 类 Phase 1 raw 表（`raw_tubi_info / raw_tubi_relate / raw_aabb(tubi) / raw_trans / raw_vec3(tubi) / raw_refno_assoc_index`）当前由 `cata_model.rs` 和 `refno_assoc_index.rs` 直接调 SurrealQL，绕过 trait；这是已知 Phase 1 trait gap，关联 candidate goal `09-phase-1-tubi-trait-migration.md`。
- `pe_transform_store.rs::register_ducklake` 当前是空 stub（`Ok(())`），是历史遗留的占位入口；与本 goal 的 `DuckLakeModelWriterBackend` 不同源，不复用其代码路径。
- 当前主仓 `feat/collab-api-consolidation` 分支已合入 ModelWriter trait 抽象；`Cargo.toml` 已存在 `model-writer-drain` feature 和 `transform-store-ducklake` feature（后者用于 pe_transform，不复用）。
- 仓库 AGENTS.md 硬约束：不跑 / 不编译 Rust tests；`aios-database` 验证用 CLI + JSON；`web_server` 验证用启动服务 + POST；SurrealDB 依赖源必须保持 `github.com/happyrust/surrealdb`。
- 本机环境历史阻塞：`target/debug/web_server.exe` 占用、独立 target 编译需 NASM 在 PATH（`C:\Program Files\NASM`），DuckDB CLI 未安装。Rust 工具链路径为 `D:/Rust/.cargo/bin`，cargo nightly 1.97。

## Constraints

- 默认 `ModelWriterMode::Surreal` 行为保持兼容，本 goal 不修改 `SurrealModelWriterBackend` 的可观察行为。
- DrainOnly 模式保持现状：所有持久化阶段 NoOp + skipped report；引入 ducklake 后此模式不受影响。
- 新增的 `DuckLakeModelWriterBackend` 必须实现 trait 上的全部 8 个阶段方法，并对落库失败 fail-fast，错误内含 batch id 与 table 名（按 `03-writer-architecture.md` 错误处理要求）。
- DuckLake 数据生命周期：在 `init` 创建 / 挂载，在 `cleanup` 与 `finalize` 关闭句柄；不可在进程内泄漏 DuckDB 连接。
- 不修改 `cata_model.rs::gen_cata_geos` 与 `refno_assoc_index.rs` 的直调链路——5 类 Phase 1 trait gap 表（tubi/transforms/refno_assoc）本期不写入 DuckLake，且 `finalize` 报告必须显式列出 Known Gap 表与原因。
- 不引入 temp-Parquet-plus-SQL 路径（不调 `ducklake_add_data_files`、不依赖 `transform-store-ducklake` 的空 stub）。
- 不变更 SurrealDB 依赖源；`Cargo.toml` 中 SurrealDB 必须保持 `github.com/happyrust/surrealdb`；不引入 `gitee.com/happydpc/surrealdb`。
- 验证不使用 Rust tests / 不编译 test target；只用 `cargo check`、CLI JSON、web_server POST、SQL parity。
- 新增依赖必须 feature-gated（`model-writer-ducklake` 或同名），默认编译不拉 DuckDB。
- 不做远端 push、PR、SurrealDB schema 破坏性迁移、公开 CLI 不兼容变更，除非用户明确批准。

## Non-Goals

- 不实现 projection 表（02-canonical-schema 的 9 张）以及 projection 刷新 SQL；本期只做 raw 表写入。projection 拆下个 goal。
- 不闭合 Phase 1 trait gap（tubi/transforms/refno_assoc 留给 `09-phase-1-tubi-trait-migration` 候选 goal）。
- 不做 compare / dual-write 模式；与 SurrealDB 的 parity 验证靠离线 SQL 对账，不依赖运行时同时写两端。
- 不修改 Parquet writer、pe_transform 多 backend、`register_ducklake` 空 stub；这些是独立分支或独立 goal。
- 不实现 Phase 2 boolean 结果表（`inst_relate_bool` / `inst_relate_cata_bool`）。
- 不优化 DuckDB 写入性能（列编码、分区、Z-order 等）；本期只要正确性 + 接口对齐。
- 不替换默认 `ModelWriterMode::Surreal`；DuckLake 是 opt-in 第三模式。
- 不改 web_server / web_api 既有路由；只复用 model-writer-backend-abstraction 已加的 `/api/model/writer-verify` 或同等入口扩展 `ducklake` mode 路径。
- 不引入 plant-model-core 抽离改动；本期所有改动都在 `plant-model-gen` 主仓。

## Ask Before

- git commit / push / 创建 PR。
- 删除、移动、批量重命名现有源码或配置文件。
- 修改公开 CLI 参数且造成不兼容（含改 `--model-writer` 取值集合的语义）。
- 引入 `duckdb` crate 以外的新外部依赖，或启用其他重型 feature。
- 任何会改写 / 删除 / 清理 SurrealDB 中现有模型数据的 cleanup 语义变更。
- 把 5 类 Phase 1 trait gap 表的写入纳入本 goal（即与 09-phase-1-tubi-trait-migration 合并）。
- 把 projection 表 / projection refresh SQL 纳入本 goal。
- 修改 SurrealDB 依赖源、Cargo patch 节、或 `Cargo.toml` 中本 goal 不需要的 feature。
- 验证需要连接远端服务器、修改运行环境（除 NASM/PATH 临时设置外）、或安装 DuckDB CLI 之外的系统依赖。
- 删除或重写 `pe_transform_store.rs::register_ducklake` 空 stub。

## Done Means

`DuckLakeModelWriterBackend` 在 `cargo check --lib --features "review,model-writer-drain,model-writer-ducklake"` 通过；通过 `model_writer_verify --mode ducklake --json` 与 web_server `POST /api/model/writer-verify {"mode":"ducklake"}` 两条路径输出 8 阶段 `implemented` + Known Gap 表列表；用 `dbnum=1112` 真实跑一次生成后，DuckLake `ducklake-canonical` schema 下的 9 张 raw 表与 SurrealDB 对账 SQL（行数 + key 集 + 采样 record id）结果为「9 表 parity 通过 / 4 表 Known Gap」；默认 Surreal 与 DrainOnly 路径无可观察行为变化；所有验收项与异常都在 `goals/ducklake-model-writer/progress.jsonl` 留下命令、时间、结果、artifact 路径证据。
