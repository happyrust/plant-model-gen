# Codex Goal Prompt: DuckLake ModelWriter Backend

After every critical document in this folder is approved (Plannotator CLI 不可用，本会话由用户在 MCP 会话中逐份 approve 代替 gate)，paste or set this goal:

```text
/goal 在 plant-model-gen 主仓的已完成 ModelWriterBackend trait 之上新增 DuckLakeModelWriterBackend：通过 Rust duckdb crate 直接挂载 DuckLake metadata 并把 trait 已覆盖的 9 张 Phase 1 raw 表写入 `ducklake-canonical` schema；默认 Surreal 与 DrainOnly 路径不受影响；不做 projection、不闭合 Phase 1 trait gap、不做 compare/dual-write。

使用 `goals/ducklake-model-writer/` 作为唯一长期计划来源：
- 先读 `brief.md`，确认 outcome、context、constraints、non-goals、ask-before、done means；本期 scope 决策已锁定为 Q1=C（trait gap 表保持 Known Gap）+ Q2=B（仅 raw 表，projection 留下个 goal）。
- 按 `plan.md` 执行 6 个 slices（Cargo feature + 连接生命周期 → write_base_batch 6 表 → mesh + inst_relate_aabb 3 表+1 upsert → reconcile + boolean_bridge stub → CLI + POST → SQL parity）；slices 必须严格按 1 → 2 → 3 → 4 → 5 → 6 顺序，前 slice cargo check 不通过不得跳到后 slice。
- 按 `verification.md` 验证：cargo check 三种 feature 组合、CLI JSON `--mode ducklake` / `--mode surreal` / `--mode drain-only` 三种模式回归、web_server POST 同三模式回归、dbnum=1112 真实生成、9 张 raw 表 SQL parity（每张表 row count + primary key set + 3 sample record id）、grep 审计 orchestrator 与 register_ducklake 边界、git diff 范围审计、SurrealDB 源未漂移。不跑 / 不编译 Rust tests。
- 每完成一个实质步骤或验证，向 `goals/ducklake-model-writer/progress.jsonl` 追加一行 jsonl 证据，含 ts、event、command/files、status、result、artifact；SQL parity 9 张表各自一行不要合并。
- 遇到 `blockers.md` 中 Stop And Ask 列出的情况必须暂停并询问用户，尤其是：git commit/push/PR；删/移/批量重命名源码；改公开 CLI 不兼容；duckdb 之外新依赖；任何 SurrealDB cleanup 语义变更；把 trait gap 表或 projection 表纳入本 goal；SurrealDB 源/Cargo patch 改动；远端连接；DuckDB bundled 编译失败或 DuckLake extension 不可用；parity 出现非 Known Gap 范围的差异；web_server endpoint 需新增公开路由；dbnum=1112 本机不可用。

完成标准（必须 100% 满足并有 progress.jsonl 证据）：
- `cargo check --lib --features "review,model-writer-drain,model-writer-ducklake"` 通过。
- `ModelWriterMode::DuckLake` 在 src/options.rs 与 src/fast_model/gen_model/model_writer.rs 中存在；feature 未启用时给出明确编译错误不 silent fallback。
- `DuckLakeModelWriterBackend` 实现 ModelWriterBackend 8 阶段；`run_boolean_bridge` 显式 skipped(reason="phase2 boolean tables out of scope")。
- `model_writer_verify --mode ducklake --json` 与 POST `/api/model/writer-verify {"mode":"ducklake"}` 两条路径输出一致，且含 known_gap_tables 列出 4 类（raw_tubi_info / raw_tubi_relate / raw_aabb(tubi) / raw_trans / raw_vec3(tubi) / raw_refno_assoc_index）。
- dbnum=1112 真实跑通，DuckLake `ducklake-canonical` schema 下 9 张 raw 表 row count > 0。
- 9 张 raw 表 SQL parity vs SurrealDB 同 dbnum 全部 PASS（row count + key set + 3 sample record id）或差异在已批准 Known Gap 范围内。
- 默认 surreal 与 drain-only 路径无可观察行为变化（grep 审计 + CLI/POST 回归 + 真实 surreal 生成行数对比）。
- pe_transform_store::register_ducklake 与 transform-store-ducklake feature 完全未被本 goal 修改（git diff 不含）。
- 改动文件范围限定在：Cargo.toml / src/options.rs / src/fast_model/gen_model/model_writer.rs / src/fast_model/gen_model/model_writer_ducklake.rs（新增）/ src/web_server/model_writer_verify.rs / src/web_server/mod.rs / src/bin/model_writer_verify.rs / goals/ducklake-model-writer/**。

不要在所有验收项都有 progress.jsonl 真实证据前宣称完成。SQL parity 9 表全部 PASS 是本 goal 的硬 finish line；任何一表未 PASS 且未列入 Known Gap，goal 即未完成。
```
