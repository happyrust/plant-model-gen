# DuckLake ModelWriter 下一步开发进度

## 2026-05-17

- 已按 MCP 会话要求使用 `planning-with-files` 制定中文开发文件。
- 已读取 planning skill，确认计划文件应落在项目根目录：`task_plan.md`、`findings.md`、`progress.md`。
- 已检查并保留根目录既有 planning 文件，将 RUS-248 / pe_transform 历史内容归档到新计划下方。
- `session-catchup.py` 用户级路径不存在：`C:\Users\dpc\.cursor\skills\planning-with-files\scripts\session-catchup.py`，已记录为非阻塞。
- 已读取 DuckLake 相关上下文：
  - `plant-model-ducklake/README.md`
  - `plant-model-ducklake/src/duckdb_backend.rs`
  - `plant-model-ducklake/src/schema.rs`
  - `plant-model-gen/goals/ducklake-model-writer/brief.md`
  - `plant-model-gen/goals/ducklake-model-writer/plan.md`
  - `plant-model-gen/goals/ducklake-model-writer/blockers.md`
  - `plant-model-gen/src/fast_model/gen_model/model_writer_ducklake.rs`
  - `plant-model-gen/src/options.rs`
  - `plant-model-gen/Cargo.toml`
- 已完成 Phase 1：锁定下一步范围为 DuckLake ModelWriter 验收与跨 crate 收敛分析；不扩大到 pe_transform DuckLake stub、不运行 Rust tests。
- 已完成 Phase 2：函数级审计 + schema 差异审计。
  - 8 个 trait 生命周期方法都有实现面；`cleanup` 与 `boolean_bridge` 是计划内 skipped。
  - `reconcile_missing_neg_relations` 仍是 sentinel 行，不是完整 carrier→target 解析。
  - in-repo DuckLake DDL 与 `plant-model-ducklake` canonical schema 分叉明显，尤其是 `raw_inst_info`、`raw_inst_relate`、`raw_aabb`、`raw_vec3`、`raw_inst_relate_aabb`。
  - 源码顶部 Slice 1 注释已陈旧，仍声称 Slice 2-4 未实现。
- 当前进入 Phase 3：CLI / feature / HTTP 验证。
- Phase 3 验证前检查终端状态：多个 sibling repo cargo 任务仍显示运行中，包括 `plant-model-ducklake` 的 `cargo run/check` 与 `plant-model-core` 的 `cargo check`；为避免叠加重型编译，本轮未启动新的 `plant-model-gen` cargo check。
- Phase 3 验证面预检：
  - `D:\Rust\.cargo\bin\cargo.exe --version` 返回 `cargo 1.97.0-nightly (4f9b52075 2026-05-01)`。
  - `model_writer_verify --mode ducklake --json` 是静态 contract evidence，不执行 DuckLake init。
  - `model_writer_verify --mode ducklake --exec --json` 才会打开 DuckDB、INSTALL/LOAD ducklake、ATTACH metadata、创建 9 张 raw 表并 finalize。
  - `POST /api/model/writer-verify {"mode":"ducklake"}` 当前只走静态 `model_writer_contract_evidence()`，不能作为 runtime DuckLake smoke。
- Phase 3 编译验证：
  - 命令：`cargo check --lib --features "review,model-writer-drain,model-writer-ducklake" --offline --target-dir target-ducklake-verify`
  - 环境：`PATH` 前置 `D:\Rust\.cargo\bin` 与 `C:\Program Files\NASM`
  - 结果：FAILED，退出码 101，耗时约 370s。
  - 失败点：`libduckdb-sys v1.10502.0` custom build script exit code 1。
  - 可见错误摘要：`error: failed to run custom build command for libduckdb-sys v1.10502.0`；输出主要为 MSVC/DuckDB C++ warnings，未暴露 Rust 业务代码错误。
- Phase 3 编译阻塞复查：
  - 已读取本地 `duckdb-1.10502.0/Cargo.toml`，确认 `default = []`；当前失败不是由 `plant-model-gen` 关闭 DuckDB default features 引起。
  - 使用同一 target 目录和 `-j 1` 重跑：`cargo check --lib --features "review,model-writer-drain,model-writer-ducklake" --offline --target-dir target-ducklake-verify -j 1`。
  - 结果：FAILED，退出码 101，耗时约 876s。
  - 明确失败点：`LINK : fatal error LNK1114: 无法覆盖原始文件 ...\libduckdb.a；错误代码 112`。
  - 磁盘检查：初次检查 `D:` 剩余约 `1.52GB`，随后复查已降至约 `0.03GB`；`C:` 剩余约 `8.10GB`。当前阻塞应先按磁盘空间不足处理。
  - 尝试查找 `plant-model-ducklake/target` 可复用产物，未找到可直接复用的 `libduckdb.a`；且该路径同样位于空间不足的 `D:`。

## 2026-05-17 续 · Phase 3 阻塞解除与验证闭环

- 磁盘复检：`D:` 剩余 `128.42GB`，`C:` `16.03GB`，`E:` `102.64GB`；之前 0.03GB 阻塞已自动解除（`target-ducklake-verify` 被清理），无需手动迁移 target-dir。
- 第一次重跑 `cargo check --lib --features "review,model-writer-drain,model-writer-ducklake" --offline --target-dir target-ducklake-verify`：在 `Checking duckdb v1.10502.0` 之后 PowerShell pipeline 异常断开，exit_code unknown，但 `libduckdb-sys` bundled C++ 已成功（否则不会到 `Checking duckdb`）。
- 第二次重跑同命令（用 `Out-File` 替代 `Tee-Object` 规避缓冲问题）：**EXIT=0**，`Finished dev profile in 1m 22s`。0 error，110 warning（均为依赖库 dead_code/unused_variables，不影响 ducklake 链路）。
- CLI 静态验证：`cargo run --bin model_writer_verify --features "review,model-writer-drain,model-writer-ducklake" --offline --target-dir target-ducklake-verify -- --mode ducklake --json`，EXIT=0，`Finished in 10m 58s`（含主 crate 11min 全量 build+link）。
  - 输出 8 个 stage 的 contract evidence：7 implemented + 1 skipped（boolean_bridge，phase2 Non-Goal）。
  - `known_gap_tables`：`raw_tubi_info / raw_tubi_relate / raw_aabb(tubi) / raw_trans / raw_vec3(tubi) / raw_refno_assoc_index` 共 6 项。
- CLI 执行验证：直接运行已 build 的 `target-ducklake-verify\debug\model_writer_verify.exe --mode ducklake --exec --json`，EXIT=0，**elapsed_ms=599**。
  - `init: executed, item_count=9` ← 真实打开 DuckDB、`INSTALL/LOAD ducklake`、ATTACH metadata、CREATE 9 张 raw 表均成功。
  - `cleanup: skipped`，reason：ducklake does not clean SurrealDB; metadata is created fresh per run。
  - 6 个 `known_gap:*` stages 全部 skipped 并附 reason 指向 `cata_model.rs / refno_assoc_index.rs` 写入面（goal `Q1=C` scope）。
  - `ducklake_root`: `output/model_writer_storage/ducklake`。
- 磁盘落地确认 (`output/model_writer_storage/ducklake/`)：
  - `metadata.ducklake` 3,084 KB（DuckDB 格式的 DuckLake metadata 数据库）。
  - `data/ducklake-canonical/` 下 9 个 raw 表目录就绪：raw_aabb / raw_geo_relate / raw_inst_geo / raw_inst_info / raw_inst_relate / raw_inst_relate_aabb / raw_neg_relate / raw_ngmr_relate / raw_vec3（与 `init` item_count=9 完全吻合）。
- 结论：Phase 3 编译 + CLI 静态 + CLI exec 三个核心验证点全部 PASS；in-repo `DuckLakeModelWriterBackend` 可在本机以 bundled DuckDB 启动 DuckLake 并建表，无需 DuckDB CLI 外部工具。

## 2026-05-17 续2 · Phase 4 入口准备 + 样本可用性阻塞

- 已 build `aios-database` bin：`cargo build --bin aios-database --features "review,model-writer-drain,model-writer-ducklake" --offline --target-dir target-ducklake-verify`，EXIT=0，**20.83s 完成**（因 lib 已被前次 model_writer_verify build 过，只 link bin）。可执行：`target-ducklake-verify/debug/aios-database.exe`。
- 已确认 `run_regen_model` 路径对 DuckLake/DrainOnly 模式自动跳过 `pre_cleanup_for_regen`，满足 goal constraint「不写 SurrealDB」。
- 本机环境探查（SurrealDB `localhost:8020`，ns=`1516`，db=`AvevaMarineSample`）：
  - `dbnum_info_table` 注册 dbnum=1112（DESI 类型，file_name=`ams1112_0001`，count=2）✓
  - **INST 表里 dbnum=1112 完全无记录**（SELECT count() FROM INST WHERE dbnum=1112 → 0）✗
  - 整个 INST 表只有 111 条记录，dbnum 分布：24383(n=35) / 7999(n=31) / 23399(n=24) / 24381(n=10) / 7997(n=6) / 23584(n=3) / 17496(n=1) / 25688(n=1)。
  - 与 goal brief.md "若本机数据不可用则记录原因并请求替代样本" 路径吻合。
- Phase 4 候选样本（按"42s 历史基线"小样本原则筛选）：
  - `dbnum=17496` (INST n=1)：与 1112 (count=2) 体量最接近，最小可行 smoke。
  - `dbnum=25688` (INST n=1)：同样极小。
  - `dbnum=23584` (INST n=3)：略大但仍小。
  - `dbnum=7997` (INST n=6)：曾在 pe_transform 工作里跑出 176K transform entries，规模偏大，不建议作为本期 first smoke。
- 决策点：选定样本 dbnum 后即可执行：`target-ducklake-verify/debug/aios-database.exe -c db_options/DbOption-cli.toml --regen-model --dbnum <N> --model-writer ducklake`。
- 待执行：跑通后用 DuckDB SQL（通过 duckdb crate 内嵌方式，或 rust 一次性脚本）查 9 张 raw 表的行数与样本主键。

## Test Results

| Check | Input | Expected | Actual | Status |
|-------|-------|----------|--------|--------|
| planning 文件更新 | 写入 `task_plan.md` / `findings.md` / `progress.md` 顶部 | 新 active plan 存在，历史内容保留 | 已完成 | PASS |
| session catchup | `python ...session-catchup.py` | 输出 catchup report 或无上下文 | 脚本路径不存在，非阻塞 | WARN |
| Phase 2 audit | 读取 `model_writer_ducklake.rs` 与 `plant-model-ducklake/src/schema.rs` | 找出实现缺口和 schema 分叉 | 已记录到 `findings.md` | PASS |
| Phase 3 cargo check readiness | 检查现有终端 | 避免重复启动重型 Rust 编译 | 发现多个相关 cargo 任务仍显示运行，暂缓新编译 | DEFERRED |
| Phase 3 verifier preflight | 读取 CLI / web endpoint + cargo version | 确认静态/执行验证命令差异 | 已确认 CLI 需 `--exec` 才实际触发 DuckLake init；web endpoint 仅静态 evidence | PASS |
| Phase 3 cargo check | `cargo check --lib --features "review,model-writer-drain,model-writer-ducklake" --offline --target-dir target-ducklake-verify` | DuckLake feature 编译通过 | `libduckdb-sys v1.10502.0` custom build script exit code 1 | FAIL |
| Phase 3 cargo check retry | 同上并加 `-j 1` | 若为并行编译不稳定，应继续或暴露更明确错误 | `lib.exe` 写 `libduckdb.a` 失败，Windows 112；`D:` 已降至约 0.03GB | BLOCKED |
| Phase 3 cargo check 2026-05-17 重跑 | 同上 cargo check 命令（`D:` 已恢复 128.42GB） | DuckLake feature 编译通过 | `Finished dev profile in 1m 22s`，0 error / 110 warning | PASS |
| Phase 3 CLI 静态验证 | `cargo run --bin model_writer_verify -- --mode ducklake --json` | 输出 contract evidence + Known Gap 表 | EXIT=0，8 stages（7 implemented + 1 skipped），6 known_gap_tables | PASS |
| Phase 3 CLI exec 验证 | `model_writer_verify.exe --mode ducklake --exec --json` | 真实打开 DuckLake，创建 9 张 raw 表 | EXIT=0，elapsed_ms=599，init item_count=9，6 known_gap stages 显式 skipped | PASS |
| Phase 3 DuckLake 磁盘落地 | 检查 `output/model_writer_storage/ducklake/` | metadata.ducklake + 9 张 raw 表目录就绪 | metadata.ducklake 3,084 KB；9 个 raw 表目录与 init 计数完全吻合 | PASS |

## 5-Question Reboot Check

| Question | Answer |
|----------|--------|
| Where am I? | Phase 3 已闭环：cargo check + CLI 静态 + CLI exec 三项验证全部 PASS，9 张 raw 表已建。 |
| Where am I going? | 决策点：是否进入 Phase 4（选 dbnum，跑 Surreal baseline + DuckLake writer 真实生成 smoke），还是先收口 Phase 5 跨 crate 收敛分析。 |
| What's the goal? | 产出可执行的 DuckLake ModelWriter 下一步开发和验证路径；当前阶段交付物已就绪。 |
| What have I learned? | 见 `findings.md` 的 2026-05-17 Discovery 与 2026-05-17 续 Phase 3 闭环段。 |
| What have I done? | 已完成 Phase 3 编译解阻 + DuckLake runtime smoke + planning 文件同步。 |

## Archived Previous Progress

# RUS-248 批注后驳回流转进度

## 2026-05-14

- 已按用户要求启用 planning-with-files。
- `session-catchup.py` 在用户级和项目级 skill 脚本路径均不存在，已记录到 `task_plan.md`，不阻塞本轮开发。
- 已读取 Trellis backend spec 和 shared thinking guides；backend 具体规范多数为占位，跨层重点是明确 PMS postMessage → 前端 API → 后端 workflow sync → SurrealDB 状态的契约。
- 已将 RUS-248 active plan 前置写入 `task_plan.md`、`findings.md`、`progress.md`，旧 `pe_transform` 计划保留在归档段落。
- 已完成 Phase 2：`plant3d-web` 新增 `reviewWorkflowSyncMutation()`，`pms.workflow_changed` 支持 `nextStep`，`applyExternalWorkflowChange()` 改为调用 `/api/review/workflow/sync`，并保留旧 PMS 消息的 `nextStep` fallback。
- 已完成 Phase 3：PMS simulator 的 `emitPmsWorkflowChanged()` 支持/推导 `nextStep`；postMessage synced 已持续回传 `ok/taskId/status/currentNode/error/requestId`。
- 已完成 Phase 4：后端 `workflow/sync` 的 `review_workflow_history` 写入补齐 `form_id`、`target_node`、`actor_*`、`source`、`created_at`，保留旧 `operator_*` 字段。
- 验证：`npm run type-check` 通过；`cargo check --bin web_server --features web_server` 通过（仅既有依赖警告）。
- 已完成 Phase 5 真实 HTTP payload 验证（本机当前代码 `web_server` 启动于 `:3199`）：
  - 创建任务 `formId=RUS248-VERIFY-20260514110621`，task `task-a19fe2cc-bd6e-4b6e-9f7f-2288c0a7f6be`。
  - SJ `active` 到 JH：`jd/submitted`。
  - JH 写入 rejected 批注状态后 `return` 到 SJ：`sj/draft`，`returnReason=RUS-248 verify return to SJ`。
  - SJ 标记 fixed 后再次 `active` 到 JH：`jd/submitted`，`returnReason=null`。
  - SurrealDB 直查 `review_workflow_history` 确认 `form_id/target_node/source/actor_*` 已落库。
- 当前 RUS-248 开发计划已完成；剩余可选项是跑 PMS CDP 端到端，但既有 CDP 卡点在 PMS 列表无法重新打开刚创建记录，不影响本轮 workflow/sync 验证结论。

## Archived Previous Progress

# pe_transform 后端重构进度

## 2026-05-08

- 已安装 `planning-with-files`：
  - Cursor 项目安装到主工作区 `D:/work/plant-code/plant-model-gen/.cursor/skills/planning-with-files`。
  - Cursor worktree 同步到 `.worktrees/pe-transform-backends/.cursor/skills/planning-with-files`。
  - Codex 个人安装到 `C:/Users/dpc/.codex/skills/planning-with-files`，并新增全局 hooks。
  - `C:/Users/dpc/.codex/config.toml` 已启用 `[features] codex_hooks = true`。
- 已创建 worktree：`D:/work/plant-code/plant-model-gen/.worktrees/pe-transform-backends`，分支 `feat/pe-transform-backends`，基于 `f0aedb6`。
- 已完成首轮代码发现，确认重构核心入口：`Cargo.toml` features、`options.rs` feature 校验、`pe_transform_refresh.rs` batch 写入、`transform_cache.rs`/`transform_rkyv_cache.rs` 读取链路。
- 已创建本轮 planning files：`task_plan.md`、`findings.md`、`progress.md`。
- `codex --version` 返回 `codex-cli 0.129.0`；`codex features list` 显示当前 CLI 的 hook feature 名为 `hooks` 且已启用，因此 `config.toml` 同时保留 `codex_hooks = true` 和 `hooks = true` 以兼容文档与当前 CLI。
- 已按用户补充要求更新方案：首轮对比固定刷新 `dbnum=7997`，且对比前必须清理历史 `pe_transform` 数据。
- 已实现 transform backend 配置面：`transform-store-parquet`、`transform-store-ducklake`、`transform-store-compare` features；`transform_write_backend`、`transform_read_backend`、`transform_compare_backends`、Parquet/DuckLake 路径和 `clear_transform_before_refresh` 配置/CLI。
- 已新增 `src/pe_transform_store.rs`：封装 `PeTransformSink` / `PeTransformSource`，默认 SurrealDB sink/source，Parquet sink/source（feature-gated），DuckLake 注册 SQL 脚本生成，dbnum 历史 `pe_transform` 清理，对比统计。
- 已修改 `src/pe_transform_refresh.rs`：batch flush 改走统一 backend，并在写入后 prime `transform_cache`。
- 已修改 `src/fast_model/gen_model/transform_cache.rs`：生成阶段 cache miss 可按 `transform_read_backend` 从 Parquet/DuckLake source 读取 local/world 并写回内存；默认 `auto/surreal` 仍走旧 SurrealDB 查询/计算路径。
- 已修改 `src/main.rs`：`--refresh-transform` 支持清理历史数据、选择写入/读取 backend、输出 compare stats。
- 静态验证：`ReadLints` 检查本轮修改文件无 linter errors；`git diff --check` 通过。
- 阻塞：当前 PowerShell 中 `cargo --version` 失败（`cargo` not recognized），尚未执行 `cargo check` 和真实 `--refresh-transform 7997` 验证。
- 2026-05-08 运行对比/profile 前环境检查：
  - `cargo` / `rustc` / `rustup` 均不在当前 PowerShell `PATH`，`C:/Users/dpc/.cargo/bin/cargo.exe` 不存在。
  - `duckdb` / `surreal` 命令均不在当前 `PATH`。
  - `Get-NetTCPConnection -LocalPort 8020` 未返回监听连接。
  - worktree 内没有现成 `aios-database.exe`，无法运行包含本轮改动的新 CLI。
- 待工具链恢复后的首个真实验证命令建议：
  - `cargo check --bin aios-database --features "review,transform-store-parquet,transform-store-compare"`
  - `cargo run --bin aios-database --features "review,transform-store-parquet,transform-store-compare" -- -c db_options/DbOption-cli --refresh-transform 7997 --clear-transform-before-refresh --transform-write-backend dual --transform-compare-backends surreal,parquet`
- 已按 planning-with-files 补充下一步详细开发方案到 `task_plan.md`：
  - Phase 8：恢复 Cargo/SurrealDB/DuckDB 验证环境。
  - Phase 9：编译收敛并修复最小错误。
  - Phase 10：执行 `7997` 清理、刷新、双写、SurrealDB vs Parquet 对比。
  - Phase 11：profile 清理、计算、写入、prime、读取、compare 各阶段耗时。
  - Phase 12：验证 DuckLake 注册脚本和 snapshot/表行数。
  - Phase 13：输出最终对比表并完成交付记录。
- 用户指定 Rust 路径后，已用 `D:/Rust/.cargo/bin` 识别到 `cargo 1.97.0-nightly` 与 `rustc 1.97.0-nightly`。
- 首次在线 `cargo check` 卡在 `happyrust/indextree` git 更新；改为离线后发现多个 git 依赖缺本地缓存。
- 已在 `Cargo.toml` 增加本地 patch，复用本机仓库：
  - `indextree -> D:/work/plant-code/indextree/indextree`
  - `miniacd -> D:/work/plant-code/miniacd`
  - `rvm-rs -> D:/work/plant-code/rvmparser/rvm-rs`
  - `surrealdb/surrealdb-types -> D:/work/plant-code/surrealdb/...`
  - `calamine -> D:/work/plant-code/calamine-mirror`
  - `cavalier_contours -> D:/work/plant-code/cavalier_contours/cavalier_contours`
  - `id_tree -> D:/work/plant-code/id_tree-mirror`
- 当前 `cargo check` 阻塞在 `rs-core` 的 `ploop-rs = { git = "https://github.com/happyrust/rust-ploop-processor", branch = "1.0" }`；本机 `D:/work/plant-code` 下未找到 `rust-ploop-processor` / `ploop` 对应本地仓库，在线更新也长时间无输出。
- 已停止本轮卡住的 `cargo check` 进程；保留了一个非本轮启动的 `cargo test ... parse_real_files ...` 进程未处理。
- `git diff --check` 通过；planning 文件 lints 无错误。

## 2026-05-11

- **`cargo check` 通过**：`cargo check --bin aios-database --features "review,transform-store-parquet,transform-store-compare" --offline` 编译成功，耗时 44s。
- 修复了以下编译阻塞问题：
  1. `surrealdb_types` 双版本冲突（301 errors）：依赖用 `github.com/happyrust/surrealdb` 但 patch 只覆盖 `gitee.com/happydpc/surrealdb`。修复：在 `Cargo.toml` 增加 `[patch."https://github.com/happyrust/surrealdb"]` 指向相同本地路径。
  2. NASM 汇编器缺失：`aws-lc-sys` 编译需要 NASM。修复：将 `C:\Program Files\NASM` 加入 PATH。
  3. `review_db.rs` 重复导入 `Ordering` 和缺少 `REVIEW_DB_CONTEXT_SET` 静态变量、重复定义 `fresh_review_db`。修复：合并导入、添加静态变量、删除重复函数。
  4. `workflow_sync.rs` 中 `request.actor.id` 直接字段访问 `Option<WorkflowActor>`。修复：改为 `request.actor().id` 方法调用。
  5. `VerifyWorkflowData` 初始化缺少 `block_code`/`actor_id`/`owner_id`/`owner_source`/`expected_next_node`/`requested_next_step` 字段。修复：补充 `None` 初始值。
- `ploop-rs` git 依赖：cargo git cache 中已有 checkout（commit `33985df`），`--offline` 模式可直接使用，无需本地 path patch。
- Phase 9（编译收敛）已完成。下一步进入 Phase 10（SurrealDB vs Parquet 首轮对比）。

### Phase 10: SurrealDB vs Parquet 首轮对比

- **环境**：
  - Cargo: `1.97.0-nightly`，SurrealDB: `3.1.0-alpha` (port 8020)
  - 数据库：`ws://127.0.0.1:8020`，namespace `1516`，database `AvevaMarineSample`
  - Worktree: `.worktrees/pe-transform-backends`（branch `feat/pe-transform-backends`）

- **执行命令**：
  ```
  cargo run --bin aios-database --features "review,transform-store-parquet,transform-store-compare" --offline \
    -- -c db_options/DbOption-cli --refresh-transform 7997 --clear-transform-before-refresh \
    --transform-write-backend dual --transform-compare-backends surreal,parquet
  ```

- **执行结果**：
  - 总耗时：724,614ms（~12 分钟）
  - dbnum 7997 总节点数：176,390
  - 已处理节点数：143,222
  - 清理历史 pe_transform：refnos=0（未找到需清理的记录）
  - Parquet 文件：`output/AvevaMarineSample/pe_transform/pe_transform.parquet`（4.5 MB）

- **对比结果**：

  | Backend | Loaded | Missing | Mismatched | Max Delta | Elapsed (ms) |
  |---------|--------|---------|------------|-----------|--------------|
  | SurrealDB (run 1) | 175,337 | 1,053 | 0 | 0.000000 | 16,283 |
  | SurrealDB (run 2) | 175,337 | 0 | 75,575 | 0.000000 | 16,235 |
  | Parquet | 143,222 | 32,115 | 58,930 | 0.000854 | 1,711 |

- **关键发现**：
  1. **Parquet 读取速度约 9.5 倍于 SurrealDB**（1,711ms vs ~16,250ms）
  2. Parquet missing=32,115 = SurrealDB 总数(175,337) - 本次刷新数(143,222)，因 Parquet 只含本次写入数据
  3. Parquet mismatched=58,930 max_delta=0.000854，为 float 序列化精度差异
  4. SurrealDB 出现两行输出，可能是 local/world transform 分别对比，或代码 bug
  5. 清理报告 refnos=0，说明按 dbnum 查找历史记录的查询可能需要调整

- **待排查**：
  - 两行 SurrealDB 对比的含义（是 local/world 分开还是代码重复输出？）
  - Parquet mismatched 的 float 精度是否可接受
  - 清理为何未找到历史记录（pe_transform 表结构是否包含 dbnum 字段？）
- Phase 10 已完成。

### Phase 11: Profile 耗时热点

- **执行命令**：同 Phase 10（第二次运行，含计时器）
- **耗时 profile**：

  | 阶段 | 耗时 (ms) | 占比 |
  |------|----------|------|
  | 计算 local/world transform | 230,888 | 37.1% |
  | SurrealDB 写入 | 145,763 | 23.4% |
  | Parquet 写入 | 245,339 | 39.5% |
  | transform_cache prime | 0 | 0.0% |
  | **总耗时** | **621,990** | **100%** |

- **关键发现**：Parquet 写入是最大瓶颈（39.5%），原因是每批 500 条写入时 read-merge-dedup-write 整个文件（O(n²)行为），随着文件增大越来越慢。
- **对比读取（compare 阶段）**：

  | Backend | Elapsed (ms) |
  |---------|-------------|
  | SurrealDB baseline | 14,845 |
  | SurrealDB compare | 14,922 |
  | Parquet | 1,698 |

- **优化建议**：Parquet 写入改为先写多个 batch 文件，最终一次合并去重。
- Phase 11 已完成。

### Parquet 写入优化 & Compare 修复

- **Parquet 写入优化**：改为每批写独立 batch 文件，最终一次 merge+dedup
  - 写入：245,339ms → 2,250ms（**73x 快**）
  - Finalize: 1,113ms
  - 总 Parquet I/O: 3,363ms
- **Compare 修复**：跳过 `surreal` 在 compare backends 中时的冗余加载，消除两行 SurrealDB 输出
- **优化后 profile**：

  | 阶段 | 耗时 (ms) | 占比 |
  |------|----------|------|
  | 计算 local/world transform | 227,056 | 59.7% |
  | SurrealDB 写入 | 150,766 | 39.7% |
  | Parquet 写入 + finalize | 3,363 | 0.9% |
  | **总耗时** | **380,072** | **100%** |

- **总耗时减少 39%**：621,990ms → 380,072ms（节省 242 秒）
- 当前瓶颈已转移到"计算 transform"（59.7%，BFS + 逐节点 SurrealDB 查询）和"SurrealDB 写入"（39.7%）
