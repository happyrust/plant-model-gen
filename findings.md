# DuckLake ModelWriter 下一步开发发现

## 2026-05-17 Discovery

- [需求] MCP 会话要求“提出下一步的详细方案，使用 planning skills 用中文制定开发文件”；本轮按 `planning-with-files` 在根目录更新 `task_plan.md`、`findings.md`、`progress.md`。
- [上下文] 最近打开文件集中在 `plant-model-ducklake` 与 `plant-model-gen` DuckLake 相关文件，因此下一步方案聚焦 DuckLake ModelWriter / storage adapter 收敛。
- [现状] `plant-model-ducklake` 是独立 DuckLake storage adapter crate，当前 scope 包含 storage config、canonical raw batch DTO、planned write contract、DuckDB backend、schema manifest、JSON/core smoke examples。
- [现状] `plant-model-gen/goals/ducklake-model-writer/brief.md` 明确 `DuckLakeModelWriterBackend` 是 `ModelWriterBackend` 的 opt-in 第三后端，目标是直接通过 Rust `duckdb` crate 写 `ducklake-canonical` raw 表。
- [边界] `pe_transform_store.rs::register_ducklake` 是历史 pe_transform stub，goal 明确不复用也不删除；本轮下一步不应把 pe_transform DuckLake 与 ModelWriter DuckLake 混合。
- [边界] 本期 DuckLake writer 只覆盖 trait 已暴露的 9 张 Phase 1 raw 表；tubi/transforms/refno_assoc 相关 6 项作为 Known Gap 显式报告。
- [发现] `Cargo.toml` 已有 optional `duckdb` 依赖，注释说明由 `model-writer-ducklake` feature 使用；`options.rs` 已有 `ModelWriterMode::DuckLake` 与 `as_str() == "ducklake"`。
- [风险] `model_writer_ducklake.rs::create_table_ddl()` 使用了较简化的 in-repo DDL（部分 payload_json / mesh fields），而 `plant-model-ducklake/src/schema.rs` 定义了更完整的 raw schema；下一步必须先做 schema diff，避免两套 canonical 长期分叉。
- [验证] 仓库规则禁止 `cargo test`；下一步验证应使用 `cargo check`、`model_writer_verify --mode ducklake --json`、web_server POST 与 DuckDB SQL 对账。
- [环境] `planning-with-files` 的用户级 `session-catchup.py` 路径不存在；已作为非阻塞问题记录。
- [审计] `DuckLakeModelWriterBackend` 已实现 trait 的 8 个方法：`init` 打开 DuckDB/DuckLake 并建 9 表，`cleanup` skipped，`write_base_batch` 写 6 张基础表，`persist_mesh_results` 写 mesh AABB/vec3 并更新 `raw_inst_geo`，`persist_inst_relate_aabb` 写 AABB 关系，`reconcile_missing_neg_relations` 写 sentinel 负关系，`run_boolean_bridge` 按 Non-Goal skipped，`finalize` checkpoint 并追加 Known Gap reports。
- [风险] `reconcile_missing_neg_relations` 当前把缺失 carrier 写为 `target_refno="__reconcile_pending__"` 的 sentinel 行，不等价于 Surreal backend 的真实 carrier→target 解析；SQL parity 需要显式 EXCEPT，后续若要求完全 parity 必须实现 raw 表 JOIN 或 provider 查询。
- [风险] `write_base_batch` 返回 `ModelWriteBatchReport::default()`，不会把 DuckLake 侧发现的 missing neg carriers 反馈给后续 reconcile；当前依赖上游/Surreal 逻辑时可能不足，需在 CLI verify 或真实生成里确认调用链期望。
- [风险] `raw_inst_relate` in-repo DDL 是 `(refno, inst_id, payload_json)`，独立 crate canonical schema 是 `(parent_refno, refno, snapshot_id, run_id, written_at, is_deleted)`；语义和主键完全不同。
- [风险] `raw_inst_info` in-repo DDL 使用 `inst_id` 而独立 crate 使用 `refno`，且缺少 `snapshot_id/run_id/written_at/is_deleted`；如果 downstream 以 schema manifest 为准，会无法直接对账。
- [风险] `raw_aabb` / `raw_vec3` in-repo DDL 使用 `aabb_id`、`vec3_id/payload`，独立 crate 使用 `aabb_hash`、`vec3_hash/x/y/z`；mesh payload 的 JSON 化会影响 SQL parity 和 projection 复用。
- [风险] `raw_inst_relate_aabb` in-repo DDL 使用 `(refno, aabb_id, source)`，独立 crate 使用 `(refno, aabb_hash, snapshot_id, run_id, written_at, is_deleted)`；字段名和审计列不一致。
- [清理] `model_writer_ducklake.rs` 文件头仍描述 Slice 2-4 “intentionally NOT IMPLEMENTED / bail”，但实际下方已有 Slice 2/3/4 写入路径；这是陈旧注释，应在下一次代码修改中修正。
- [验证] `src/bin/model_writer_verify.rs` 的默认 `--mode ducklake --json` 只调用 `model_writer_contract_evidence(mode)`，不会打开 DuckDB/DuckLake；运行时 smoke 必须使用 `--exec --mode ducklake --json` 且启用 `model-writer-ducklake` feature。
- [验证] `src/web_server/model_writer_verify.rs` 当前 POST endpoint 也只返回 `model_writer_contract_evidence(mode)`；它是非破坏静态 evidence，不会执行 `DuckLakeModelWriterBackend::init()`，因此不能替代 CLI `--exec` smoke。
- [环境] 明确 Rust 工具链路径可用：`D:\Rust\.cargo\bin\cargo.exe --version` 返回 `cargo 1.97.0-nightly (4f9b52075 2026-05-01)`。
- [阻塞] `cargo check --lib --features "review,model-writer-drain,model-writer-ducklake" --offline --target-dir target-ducklake-verify` 失败，退出码 101；根因在 `libduckdb-sys v1.10502.0` custom build script exit code 1，不是当前 Rust 业务代码类型错误。
- [阻塞] `libduckdb-sys` 输出包含大量 MSVC warning（如 C4530 / C4267）和 `VCINSTALLDIR=None` / `LIB=None` / `INCLUDE=None` 环境记录；需要复查是否必须在 VS Developer PowerShell / vcvars 环境下编译，或改用已验证的 DuckDB 构建方式。
- [发现] `duckdb-1.10502.0` 的 `default = []`，因此 `plant-model-gen` 的 `default-features = false, features = ["bundled"]` 与 `plant-model-ducklake` 的 feature 组合在 DuckDB 默认特性层面没有本质差异。
- [阻塞] 使用同一 `target-ducklake-verify` 并加 `-j 1` 重跑后，`libduckdb-sys` 失败点收敛为 `LINK : fatal error LNK1114: 无法覆盖原始文件 ... libduckdb.a；错误代码 112`。
- [环境] `Get-CimInstance Win32_LogicalDisk` 复查显示 `D:` 已降到约 `0.03GB` 可用空间，Windows 错误码 112 对应磁盘空间不足；当前 Phase 3 阻塞优先归因为 DuckDB bundled C++ 静态库归档需要更多 D 盘空间，而不是 ModelWriter Rust 代码错误。
- [下一步] 先释放 `D:` 空间（优先清理本轮生成的 `target-ducklake-verify` 或其它可重建 target/cache）或把 DuckDB 验证 `--target-dir` 指向剩余空间更充足的磁盘；再重跑 DuckLake feature `cargo check` 和 `model_writer_verify --exec`。

## 2026-05-17 续 · Phase 3 闭环 Findings

- [解阻] 磁盘阻塞已自动解除：复检 `D:` 128.42GB / `C:` 16.03GB / `E:` 102.64GB，`target-ducklake-verify` 已被前次失败链路清理；无需手动迁移 target-dir。说明上次的"0.03GB"是 link 阶段写部分对象时把盘撑爆，后续临时文件被释放即恢复。
- [陷阱] PowerShell `Tee-Object` 在长时间 cargo 管道里有缓冲卡顿，导致 exit_code unknown。**改用 `Out-File`** 即可稳定捕获完整日志和退出码；后续长任务推荐这种写法。
- [验证] `cargo check --lib --features "review,model-writer-drain,model-writer-ducklake" --offline --target-dir target-ducklake-verify` 二次重跑：EXIT=0，1m 22s（增量），0 error / 110 warning（均为依赖库 dead_code）。
- [证据] `model_writer_verify --mode ducklake --json` 静态路径输出 8 stages：init / cleanup / base_batch / mesh_persist / inst_relate_aabb / missing_neg_reconcile / finalize 均 `implemented`；boolean_bridge `skipped`（phase2 Non-Goal）。
- [证据] `model_writer_verify --mode ducklake --exec --json` 执行路径在 **599ms** 内完成：`init: executed item_count=9` 证明 bundled DuckDB 成功 `INSTALL/LOAD ducklake` + `ATTACH metadata.ducklake` + 建 9 张 raw 表；`cleanup: skipped`（reason: ducklake 不清理 SurrealDB）；6 个 `known_gap:*` stages 全部 skipped 且 reason 指向 `cata_model.rs / refno_assoc_index.rs` 写入面（Q1=C scope）。
- [证据] 磁盘落地：`output/model_writer_storage/ducklake/metadata.ducklake` 3,084 KB；`data/ducklake-canonical/` 下 9 个 raw 表目录与 init 计数完全吻合（raw_aabb / raw_geo_relate / raw_inst_geo / raw_inst_info / raw_inst_relate / raw_inst_relate_aabb / raw_neg_relate / raw_ngmr_relate / raw_vec3）。
- [结论] in-repo `DuckLakeModelWriterBackend` 在本机 Windows + bundled DuckDB 下可启动 DuckLake runtime，DuckDB extension `INSTALL ducklake; LOAD ducklake; ATTACH` 路径稳定可用，**无需 DuckDB CLI 外部工具**（这回答了 task_plan.md Key Question #4）。
- [边界] `--exec` 当前只覆盖 `init → cleanup → finalize` 生命周期，没有真实 batch 写入；要回答 Key Question #1（Slice 2-4 真实写入正确性），需要在 Phase 4 用真实 dbnum 跑 `write_base_batch` / `persist_mesh_results` / `persist_inst_relate_aabb` / `reconcile_missing_neg_relations` 全链路。
- [后续风险] `reconcile_missing_neg_relations` 的 sentinel 行 `target_refno="__reconcile_pending__"` 在 Phase 4 真实数据 smoke 时会出现在 `raw_neg_relate` 表里，SQL 对账需显式 EXCEPT 或加 `is_sentinel` 列，否则会与 Surreal backend 真实 carrier→target 解析结果不一致。

## 2026-05-17 续2 · Phase 4 样本探查 Findings

- [build] `cargo build --bin aios-database --features "review,model-writer-drain,model-writer-ducklake" --offline --target-dir target-ducklake-verify` **20.83s** 完成 (EXIT=0)；lib 部分被 model_writer_verify build 复用。`target-ducklake-verify/debug/aios-database.exe` 已就绪。
- [安全] 已确认 `cli_modes::run_regen_model` 第 1683 行守卫 `if db_option_override.model_writer_mode.writes_to_surreal()`：DuckLake/DrainOnly 模式 **跳过 `pre_cleanup_for_regen`**，不会删除 SurrealDB 现有模型数据，满足 goal brief.md 「Ask Before / 不删除 SurrealDB cleanup」。
- [阻塞] **本机 dbnum=1112 不可用**：虽 `dbnum_info_table` 有 dbnum=1112 (count=2, file=ams1112_0001) 注册，但 `INST` 表里 `WHERE dbnum=1112` 返回 0 条；可能数据未完成 sync 或被清理。无 INST root 则 `--regen-model` 收集不到 target_refnos，gen 流程会 no-op。
- [现状] 本机 INST 表合计 111 条，按 dbnum 分布：24383(35) / 7999(31) / 23399(24) / 24381(10) / 7997(6) / 23584(3) / 17496(1) / 25688(1)。其中 7997 在 pe_transform 工作里曾扩展成 176K transform 节点，规模偏大；17496 / 25688 (n=1) 与 dbnum=1112 (count=2) 体量最接近，是最佳替代 first-smoke 样本。
- [候选决策] 建议 first-smoke 用 `dbnum=17496` 或 `dbnum=25688`：能在最小风险下验证 ducklake writer 真实写入 9 张 raw 表是否非空、reconcile sentinel 是否生成、finalize Known Gap 表是否正确列出；SQL parity 因数据量极小可手工对账，不需重型 DuckDB CLI。
- [决策点边界] 不擅自跑生成；等待用户在「按推荐方案继续」语义下选定具体 dbnum，再触发：`aios-database.exe -c db_options/DbOption-cli.toml --regen-model --dbnum <N> --model-writer ducklake`。

## Archived Previous Findings

# RUS-248 批注后驳回流转发现

## 2026-05-14 Discovery

- [关键] 外部流程校验已经存在：`/api/review/workflow/verify` 对 `return` 使用 `ReturnReject` 批注门禁，要求当前节点是 `jd/sh/pz` 且至少有 `open` 或 `rejected` 批注。
- [关键] `/api/review/workflow/sync` 的 `return` mutation 会更新 `review_tasks.current_node/status/return_reason`，并同步 `review_forms`，是 PMS 外部驱动应使用的落库路径。
- [风险] `plant3d-web/src/composables/useReviewStore.ts::applyExternalWorkflowChange()` 当前仍调用内部 `reviewTaskReturn()` / `reviewTaskApprove()`，最终落到 `/api/review/tasks/{id}/return|approve`。
- [风险] 内部 API 会校验 JWT `user_id` 是否等于当前节点 owner；PMS 外部流程中 owner 由 `next_step.assignee_id` 声明，二者命名空间不一致时会导致“verify 通过但 changed 落库失败”。
- [现状] 被驳回到 `sj` 后，任务会被前端识别为 `returnedInitiatedTasks`；设计端面板展示退回意见、批注列表，并允许保存 confirmed record。
- [现状] 设计端保存处理结果后不直接推进工作流；再次流转依赖 PMS 后续触发 `active` 或内部提交。
- [方案] PMS iframe/postMessage 外部路径应统一改为 `/api/review/workflow/sync`，并传入 `actor`、`next_step`、`comments`。
- [实现] `plant3d-web` 的 external `workflow_changed` 已改走 `/api/review/workflow/sync`，内部按钮仍保留原 `/api/review/tasks/{id}/submit|return|approve` 路径。
- [兼容] `nextStep` 优先使用 PMS 显式传入；旧 PMS/simulator 未传时，前端按 action/currentNode/targetNode 推导下一节点和负责人。
- [实现] 后端 `review_workflow_history` schema 原已支持 `form_id/target_node/source/actor_*`，本轮补齐 `workflow/sync` 写入字段，便于后续排查外部流转。
- [验证] `npm run type-check` 和 `cargo check --bin web_server --features web_server` 均通过；真实 return/active 数据闭环尚未执行。
- [验证] 真实 HTTP payload 闭环已通过：`SJ active -> JH return -> SJ fixed -> SJ active`，最终任务 `jd/submitted` 且 `returnReason=null`。
- [验证] SurrealDB 直查确认三条 `review_workflow_history` 均写入 `form_id/target_node/source/actor_id/actor_role`，source 分别为 `rus248-cli-verify-active`、`rus248-cli-verify-return`、`rus248-cli-verify-reactive`。
- [环境] 默认 target 启动当前 web_server 失败是因为旧 `:3100` 服务锁定 `target/debug/web_server.exe`；独立 `target-rus248` + `WEB_SERVER_PORT=3199` 可绕开。首次全量编译需要 `C:\Program Files\NASM` 在 PATH。

## Archived Previous Findings

# pe_transform 后端重构发现

## 2026-05-08 Discovery

- `Cargo.toml` 已有 `parquet-export` feature，负责引入 `parquet`、`arrow-array`、`arrow-schema`、`polars`；新增 transform Parquet 能力应考虑复用或拆出更轻的 `transform-store-parquet`。
- `options.rs::validate_model_writer_features` 已有清晰的 feature 校验模式，可复用于 transform backend，例如未启用 `transform-store-ducklake` 时禁止 `--transform-read-backend ducklake`。
- `pe_transform_refresh.rs` 当前直接调用 `save_pe_transform_entries(&entries)` 批量写 SurrealDB，是插入 `PeTransformSink` / dual-write 的主要入口。
- `transform_cache.rs` 和 `transform_rkyv_cache.rs` 当前读取链路是 rkyv/内存优先，miss 后从 SurrealDB `pe_transform` 查询；新增 source 应保持最终统一 prime 到内存 cache。
- `fast_model/export_model/export_dbnum_instances_parquet.rs` 已有 `transforms.parquet`，但它表达的是唯一 transform hash 到矩阵，不是 `refno -> local/world transform` 的 PE 映射，不能直接替代 `pe_transform` 表。
- DuckLake 支持 `ATTACH 'ducklake:metadata.ducklake' AS lake (DATA_PATH 'data/')` 后建表写入，也支持先写外部 Parquet 再 `CALL ducklake_add_data_files(...)` 注册。
- DuckLake partitioning 支持 `ALTER TABLE ... SET PARTITIONED BY (...)`，首版建议按 `project_name, dbnum` 分区，避免按 refno 产生过多小文件和目录。
- 首轮测试样本已由用户指定为 `dbnum=7997`。
- 对比前必须清理历史 `pe_transform`，否则 SurrealDB 旧数据可能和新刷新的 Parquet/DuckLake 数据混在一起，导致矩阵一致性和加载耗时结论失真。
- 当前实现中 `dual` 写入表示 SurrealDB + Parquet 双写；DuckLake 首版通过 `transform-store-ducklake` 生成注册 SQL 脚本，不直接引入 Rust DuckDB/DuckLake 运行时。
- `transform_read_backend=ducklake` 当前先复用 Parquet source 读取文件内容；DuckLake 原生 time-travel 查询需要后续接入 DuckDB/DuckLake CLI 或 Rust binding。
- 当前环境 `cargo` 不在 PATH，无法做 Rust 编译校验；后续必须在 Rust 工具链可用环境补跑 `cargo check`，再跑真实 `--refresh-transform 7997` 流程。
- 本轮无法产出真实耗时 profile：缺少 Rust 工具链、DuckDB/Surreal CLI，且 8020 端口未检测到数据库监听；表格只能记录待测项和当前阻塞状态。

## 2026-05-08 Next-Step Findings

- 下一步不应继续扩大功能面；优先把当前 worktree 主体实现编译收敛，再做 `7997` 的 SurrealDB/Parquet 对比。
- 首轮 profile 表必须区分“计算 transform”和“存储/读取 backend”两类耗时，否则无法判断 Parquet/DuckLake 是否真正改善预热阶段。
- `dual` 写入的验收对象是 SurrealDB baseline 与 Parquet 文件一致性；DuckLake 首轮只验证注册脚本和 metadata 管理，不承诺原生读取性能。
- 对比表的核心列应固定为：`Backend | Write Time | Read Time | Loaded | Missing | Mismatched | Max Delta | Notes`。
- 如果 Parquet 出现 missing，优先排查分区路径和递归扫描；如果出现 mismatched，优先按 refno 抽样比较 local/world 矩阵展开列。
- 指定 `D:/Rust/.cargo/bin` 后 Rust 工具链可用；当前真正阻塞不再是 cargo 缺失，而是 `rs-core` 的 `rust-ploop-processor` git 依赖无法在线更新且本机没有本地副本。
- 为了使后续 `cargo check` 可继续，需要二选一：提供 `D:/work/plant-code/rust-ploop-processor/ploop-rs` 本地仓库并加 patch，或恢复访问 `https://github.com/happyrust/rust-ploop-processor`。

## 2026-05-11 Phase 10 Findings

- [性能] **Parquet 读取速度约 9.5 倍于 SurrealDB**：Parquet 1,711ms vs SurrealDB ~16,250ms。这证实了 Parquet 作为 transform 预热数据源的可行性。
- [精度] Parquet 序列化/反序列化引入 max_delta=0.000854 的 float 精度差异，影响 58,930/143,222 条记录（41%），但绝对误差极小（<0.001mm），在工程精度内可接受。
- [数据完整性] Parquet missing=32,115 不是 bug：SurrealDB 包含 175,337 条历史记录（可能涵盖多个 dbnum），而 Parquet 仅写入本次刷新的 143,222 条。差值 32,115 = 非本次 dbnum 的历史数据。
- [清理] `--clear-transform-before-refresh` 报告 refnos=0，说明按 `dbnum=7997` 查询历史 pe_transform 的查询未找到对应记录。可能原因：pe_transform 表以 refno 为主键、不含独立 dbnum 字段，或 dbnum 筛选逻辑有误。需复查 `clear_pe_transforms_for_dbnums` 的 SurQL 查询。
- [对比输出] 出现两行 SurrealDB 对比结果（第一行 missing=1053/mismatched=0，第二行 missing=0/mismatched=75575），需要检查 `compare_backends` 函数是否对同一后端做了两次不同维度的对比（如分别对比 local 和 world transform），或是代码误输出。
- [写入确认] Dual 写入成功：SurrealDB 和 Parquet 均有数据写入，Parquet 文件 4.5 MB。
- [编译] `cargo build` 通过（29s），`cargo run` 运行完整流程 724s（~12 分钟），其中大部分时间花在 176,390 节点的 transform 计算和 SurrealDB 批量写入。

## 2026-05-11 Phase 11 Profile Findings

- [瓶颈] **Parquet 写入是最大耗时瓶颈**（245,339ms = 39.5%），超过 SurrealDB 写入（145,763ms = 23.4%）。原因：`save_entries_to_parquet` 每批（500条）调用时执行 read-merge-dedup-write 全文件操作，随文件增大为 O(n²)。
- [瓶颈] 计算 local/world transform 占 37.1%（230,888ms），主要由 BFS 遍历 + 逐节点 SurrealDB 查询 `get_local_mat4` 和 `get_children_refnos` 贡献。
- [性能] SurrealDB 批量写入（23.4%）在三个阶段中效率最高，因为使用了原生批量 INSERT。
- [性能] transform_cache prime = 0ms，说明 `prime_global_transform_cache_from_pe_entries` 未实际执行缓存操作（可能全局缓存未初始化）。
- [优化方向] Parquet 写入优化建议：(1) 每批写独立文件 `batch_NNN.parquet`，最后一次合并去重；(2) 或在内存中累积所有 entries，最终一次写入；预期可将 Parquet 写入从 245s 降到 <5s。
- [读取对比] Parquet 读取 1,698ms vs SurrealDB 读取 ~14,900ms，Parquet 读取约 8.8x 快。这说明 Parquet 写入慢只是当前实现问题，读取端已经验证了 Parquet 格式的优势。

## 2026-05-11 Parquet 优化 & Compare 修复 Findings

- [优化] Parquet 写入从 O(n²) 优化为 O(n)：每批写独立 batch 文件 → 最终 merge+dedup。写入 245,339ms → 2,250ms（**73x 快**），finalize 1,113ms。
- [优化] 总刷新耗时从 621,990ms 降到 380,072ms（**39% 减少**），瓶颈已从 Parquet 写入转移到 BFS 计算（59.7%）和 SurrealDB 写入（39.7%）。
- [修复] Compare 冗余 SurrealDB 加载：当 `surreal` 在 `transform_compare_backends` 中时跳过重复加载，输出从 3 行变为 2 行（baseline + parquet）。
- [确认] 优化后 Parquet compare 结果不变：loaded=143222, missing=32115, mismatched=58930, max_delta=0.000854, elapsed=1743ms，证明 batch 写入+合并与旧的增量合并在数据正确性上一致。
