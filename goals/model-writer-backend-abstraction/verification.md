# Verification: ModelWriter 完整后端抽象

## Commands

| Command | Purpose | Expected pass condition | Evidence location |
| --- | --- | --- | --- |
| `cargo check --lib --features "review,model-writer-drain"` | 静态检查主库和 writer feature 组合 | 编译通过；若 NASM/toolchain 阻塞，记录完整错误并使用后续 CLI/POST 补充验证 | `progress.jsonl` |
| `cargo run --bin <model-writer-verify-bin> --features "review,model-writer-drain" -- --mode drain-only --json` | CLI JSON 验证 DrainOnly backend 生命周期 | JSON 中 backend=`drain-only`，持久化阶段均为 skipped，且无 cleanup/write 副作用 | `progress.jsonl` |
| `cargo run --bin <model-writer-verify-bin> --features "review,model-writer-drain" -- --mode surreal --json` | CLI JSON 验证 Surreal backend 生命周期/调用顺序 | JSON 中 backend=`surreal`，阶段顺序完整；可在无真实写库的 dry/record 模式下输出 contract evidence | `progress.jsonl` |
| `cargo run --bin web_server --features "review,model-writer-drain"` | 启动 web_server，用于 POST 验证 | 服务成功监听；日志无启动错误 | `progress.jsonl` |
| `curl.exe -X POST <local-generation-or-admin-endpoint> -H "Content-Type: application/json" -d "<payload>"` | 按仓库约束通过 POST 验证 web_server 运行时行为 | HTTP 成功响应；日志显示 writer backend、阶段执行和 finalize | `progress.jsonl` |
| `rg -n "save_inst_relate_aabb_rows|save_aabb_to_surreal|save_pts_to_surreal|reconcile_missing_neg_relate|run_boolean_worker\\(|run_bool_worker_from_tasks\\(" src/fast_model/gen_model/orchestrator.rs` | 静态确认 orchestrator 不再直接拥有模型持久化职责 | 无匹配，或仅剩非持久化/注释引用且在记录中解释 | `progress.jsonl` |
| `rg -n "runs_downstream_pipeline" src/` | 确认死接口已处理 | 要么无匹配，要么存在实际调用点 | `progress.jsonl` |

## Manual Checks

- 审查 `src/fast_model/gen_model/model_writer.rs`：trait 方法是否覆盖完整生命周期；request/report 是否避免泄漏 Surreal 专属类型到通用接口。
- 审查 `src/fast_model/gen_model/orchestrator.rs`：是否只保留编排、并发和错误传播，不再直接执行模型持久化细节。
- 检查 DrainOnly 日志：必须明确说明 skipped 阶段和原因，不能静默跳过。
- 检查 Surreal 日志：默认路径仍显示与当前生成流程一致的关键阶段。
- 如果 web_server POST endpoint 或 payload 需要根据现有 admin/generation API 选择，先记录选择依据再执行。

## Evidence Rules

- Record verification results in `progress.jsonl`.
- Include command, status, timestamp, and artifact path when available.
- Do not rely on Rust tests; this repo explicitly asks not to use or compile tests.
- 对失败验证必须记录：失败命令、错误摘要、判断是代码问题还是环境问题、下一步处理。
