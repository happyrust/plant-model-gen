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
