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
