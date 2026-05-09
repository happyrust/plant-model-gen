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
