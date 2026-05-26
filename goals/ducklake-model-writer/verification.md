# Verification: DuckLake ModelWriter Backend

## Commands

| Command | Purpose | Expected pass condition | Evidence location |
| --- | --- | --- | --- |
| `cargo check --lib --features "review"` | 静态检查：未启用 ducklake feature 时主库仍兼容 | 编译通过；无新增 warning 来自 model_writer 模块 | `progress.jsonl` |
| `cargo check --lib --features "review,model-writer-drain"` | 静态检查：drain-only feature 组合不受 ducklake 改动影响 | 编译通过；与 `model-writer-backend-abstraction` goal 的基线一致 | `progress.jsonl` |
| `cargo check --lib --features "review,model-writer-drain,model-writer-ducklake"` | 静态检查：完整 ducklake feature 组合 | 编译通过；首次会拉 duckdb crate 源（bundled）耗时较长 | `progress.jsonl` |
| `cargo run --bin model_writer_verify --features "review,model-writer-drain,model-writer-ducklake" -- --mode drain-only --json` | 回归验证：drain-only backend 不受影响 | JSON 与 `model-writer-backend-abstraction/progress.jsonl@2026-05-11T13:53` 一致：backend=drain-only、5 阶段 skipped、init/base_batch/finalize implemented | `progress.jsonl` |
| `cargo run --bin model_writer_verify --features "review,model-writer-drain,model-writer-ducklake" -- --mode surreal --json` | 回归验证：surreal backend 不受影响 | JSON 与 baseline 一致：backend=surreal、8 阶段 implemented | `progress.jsonl` |
| `cargo run --bin model_writer_verify --features "review,model-writer-drain,model-writer-ducklake" -- --mode ducklake --json` | 主验证：ducklake backend 生命周期完整 | JSON 含 backend=ducklake、writes_to_surreal=false、init/cleanup/write_base_batch/persist_mesh_results/persist_inst_relate_aabb/reconcile_missing_neg_relations/finalize=implemented、run_boolean_bridge=skipped(reason="phase2 boolean tables out of scope")、known_gap_tables 列出 4 类（raw_tubi_info / raw_tubi_relate / raw_aabb(tubi) / raw_trans / raw_vec3(tubi) / raw_refno_assoc_index） | `progress.jsonl` |
| `cargo check --lib --features "review,model-writer-ducklake"` 不带 `model-writer-drain` | 边界：feature 互斥 / 独立检查 | 编译通过（ducklake 不依赖 drain-only） | `progress.jsonl` |
| `cargo run --bin <generation-bin> --features "review,model-writer-drain,model-writer-ducklake" -- -c db_options/DbOption-cli --refresh-transform 1112 --model-writer ducklake` 或同等真实生成 CLI | 真实生成：dbnum=1112 通过 ducklake backend 落库 | 命令成功结束；DuckLake metadata.ducklake 与 data/ 目录在 `output/AvevaMarineSample/model_writer_storage/ducklake/` 下生成 | `progress.jsonl` + `output/.../ducklake/` 路径 |
| `WEB_SERVER_PORT=3199 cargo run --bin web_server --features "review,model-writer-drain,model-writer-ducklake" -- --config db_options/DbOption-cursor` | 启动 web_server 用于 POST 验证（参照 `model-writer-backend-abstraction/progress.jsonl@2026-05-11T15:32` 的 NASM / target 锁定经验） | 服务监听 3199；启动日志无错误；`/api/version` 返回当前代码版本 | `progress.jsonl` |
| `Invoke-RestMethod -Method Post -Uri http://127.0.0.1:3199/api/model/writer-verify -ContentType 'application/json' -Body '{"mode":"ducklake"}' \| ConvertTo-Json -Depth 8` | POST 验证：web_server 路径与 CLI 等价 | 响应 JSON 与上面 `model_writer_verify --mode ducklake --json` 字段一致 | `progress.jsonl` |
| `Invoke-RestMethod -Method Post -Uri http://127.0.0.1:3199/api/model/writer-verify -ContentType 'application/json' -Body '{"mode":"drain-only"}'` 与 `... -Body '{"mode":"surreal"}'` | POST 回归 | 与 baseline 一致 | `progress.jsonl` |
| `rg -n "save_inst_relate_aabb_rows\|save_aabb_to_surreal\|save_pts_to_surreal\|reconcile_missing_neg_relate\|run_boolean_worker\\(\|run_bool_worker_from_tasks\\(" src/fast_model/gen_model/orchestrator.rs` | 回归：orchestrator 没被 ducklake 引入回退到直调 | 与 `model-writer-backend-abstraction` 基线一致（应为无匹配） | `progress.jsonl` |
| `rg -n "register_ducklake\|transform-store-ducklake" .` | 边界：未污染 pe_transform 的 DuckLake stub | 仅命中既有文件（`src/pe_transform_store.rs`, `Cargo.toml`, `src/options.rs`），无新调用点 | `progress.jsonl` |
| `git diff --stat` | 改动范围审计 | 改动文件限定在：`Cargo.toml` / `src/options.rs` / `src/fast_model/gen_model/model_writer.rs` / `src/fast_model/gen_model/model_writer_ducklake.rs`（新增）/ `src/web_server/model_writer_verify.rs` / `src/web_server/mod.rs` / `src/bin/model_writer_verify.rs` / `goals/ducklake-model-writer/**` | `progress.jsonl` |
| `rg -n "gitee.com/happydpc/surrealdb" Cargo.toml Cargo.lock` | 硬约束：SurrealDB 源未漂移 | 无匹配 | `progress.jsonl` |
| `rg -n "github.com/happyrust/surrealdb" Cargo.toml` | 硬约束：SurrealDB 源仍正确 | 至少 1 个匹配 | `progress.jsonl` |
| `duckdb output/AvevaMarineSample/model_writer_storage/ducklake/metadata.ducklake -c "SELECT table_name, estimated_size FROM duckdb_tables() WHERE schema_name='ducklake-canonical' ORDER BY table_name"` 或 `cargo run --bin model_writer_verify ... -- --mode ducklake --dump-counts` 等价命令 | DuckLake 9 表落库行数采样 | 9 张表均 count > 0；行数与 SurrealDB 同 dbnum 误差为 0 或差异在 Known Gap 范围内 | `progress.jsonl` |
| `goals/ducklake-model-writer/sql/parity_<table>.sql` × 9（新增文件）通过 DuckDB CLI / Rust duckdb crate 执行 | SQL parity：row count + key set + 3 sample record ids per 表 | 每张表的 parity SQL 输出 PASS（或显式 Known Gap 行） | `goals/ducklake-model-writer/sql/` + `progress.jsonl` |

## Manual Checks

- 审查 `src/fast_model/gen_model/model_writer_ducklake.rs`：trait 实现是否完整 8 阶段、批事务是否覆盖整个 stage、错误抛出是否带 batch id + table 名（按 03-writer-architecture 错误处理要求）。
- 审查 `Cargo.toml`：`duckdb` 依赖 version 是否冻结（非 floating `*`）、`bundled` feature 是否启用、`optional = true` 是否正确、`model-writer-ducklake` 是否 `dep:duckdb` only（不附带其它非必要 feature）。
- 审查 `src/options.rs`：`ModelWriterMode::DuckLake` 字面值解析与显示是否对称、`validate_model_writer_features` 在 feature 未启用 + `--model-writer ducklake` 同时出现时是否返回明确错误（不 silent fallback 到 surreal）。
- 审查 DuckLake metadata 路径选择：默认是否落在 `output/<project>/model_writer_storage/ducklake/`、`DbOptionExt::ducklake_root` 是否可覆写但不暴露公开 CLI 参数。
- 检查 `finalize` 报告 JSON 结构：是否含稳定的 `known_gap_tables: ["raw_tubi_info", "raw_tubi_relate", "raw_aabb(tubi)", "raw_trans", "raw_vec3(tubi)", "raw_refno_assoc_index"]`；是否含 `rows_by_table` 9 表行数。
- 检查 SurrealDB 默认路径未被改动：`SurrealModelWriterBackend` 与 `DrainOnlyModelWriterBackend` 源文件 `git diff` 应只在新增 ModelWriterMode::DuckLake 处出现（match arm 添加），不应改动它们的方法实现。
- 检查 `pe_transform_store::register_ducklake` 与 `transform-store-ducklake` feature 完全未被本 goal 修改（`git diff src/pe_transform_store.rs` 应为空）。
- 如果 DuckDB bundled feature 在本机 Rust 工具链编译失败（参考 RUS-248 NASM 阻塞经验），停下并按 `blockers.md` 报告，不要 silent 切换到非-bundled。
- 如果 web_server `target/debug/web_server.exe` 占用阻止重建，按 `model-writer-backend-abstraction/progress.jsonl@2026-05-11T14:21` 的恢复步骤：等占用进程释放 / 用独立 target 目录 / 升级 NASM PATH，然后重跑；不要绕过 POST 验证。
- 如果 `dbnum=1112` 本机不可用，需先检索 `db_options/DbOption-cli` 实际配置的 namespace/db 是否含 1112；不可用时按 blockers.md Stop And Ask 报告并询问用户替换 dbnum。

## Evidence Rules

- 所有验证结果按 jsonl 行追加 `goals/ducklake-model-writer/progress.jsonl`，每行至少含：`ts`（ISO8601 +08:00）、`event`、`command` 或 `files`、`status`（passed/failed/blocked/skipped）、`result`（简短文字）、`artifact`（关键 stdout snapshot 或路径）。
- 失败验证必须记录：失败命令、错误摘要、判断是代码问题还是环境问题、下一步处理。
- POST 响应体与 CLI stdout JSON 必须保留完整 snapshot（可放在 `goals/ducklake-model-writer/evidence/` 子目录引用），不要只写"PASS"。
- SQL parity 9 张表的输出必须按 table 各自留 evidence 行，便于 diff 与回查；不要合成单条聚合 PASS 行。
- 不依赖 Rust tests / 不编译 test target；按 AGENTS.md 与 brief.md Constraints 执行。
- progress.jsonl 是 append-only，不删历史行；若某次结论失效，新增一行 `event: superseded` 并引用旧 ts，不修改旧行。
