# C — SurrealDB 连接/存储引擎初始化与内存模式可行性约束

> 任务：搞清当前 SurrealDB 初始化方式（embedded RocksDB / remote client）、引擎选择配置点、
> 切内存引擎（kv-mem / `mem://`）需要动哪里、以及与 specs/022/023 约束的冲突。
> 只读分析，未改任何生产代码。证据均带 文件:行号（rs-core / surrealdb fork 为本地 patch 检出：
> `../rs-core`、`../surrealdb`，对应 git 依赖 happyrust/rs-core dev-3.1、happyrust/surrealdb dev-3.1）。

## TL;DR

1. 全局只有一个主连接 `SUL_DB: Lazy<Surreal<Any>>`（rs-core `rs_surreal/mod.rs:99`）；
   `model_primary_db()` 与 `project_primary_db()` 都是它的别名（同文件 :111/:120）——"模型表与 PE/ATT 固定同库"在代码层就是"同一个连接对象"。
2. 引擎由配置文件 `[surrealdb] mode = "file" | "ws"` 决定（`DbConnMode` 仅两个变体）：
   file → 进程内嵌入式 `rocksdb://…`（**需要 `kv-rocksdb` feature，默认构建不含**）；
   ws → 连接外部 `surreal` server 进程（站点体系全部走这条路）。
3. **`kv-mem` 已经在默认 feature 里编译进来了**（plant-model-gen 与 rs-core 的 surrealdb 依赖都声明了 `kv-mem`），
   `SUL_DB.connect("mem://")` 今天就能跑（rs-core 测试 helper 已有先例）。缺的只是配置面：`DbConnMode` 加 `Mem` 变体 + 连接串组装。
4. 意外发现：**fork 的 mem 引擎（surrealmx）本身支持 `versioned=true&retention=…`**，
   `SELECT … VERSION` 时间旅行在内存引擎上可用（未开 versioned 时报 `UnsupportedVersionedQueries`，不会静默错数据）；
   甚至支持 `mem:///path` 落盘持久化（AOL/snapshot）。
5. 真正的硬约束不是引擎能力，而是**易失性 × specs/022 锚点体系**：
   重启即丢 `sesno_version_anchor` / `dbnum_info_table` 水位线 / pe_owner meta / PE 数据本身，
   watch 增量链路失去起点，只能整库重灌。内存模式只适合"一次性解析→生成→导出"的短生命周期场景。

---

## ① 连接初始化调用链

### 全局连接对象（rs-core）

```99:122:../rs-core/src/rs_surreal/mod.rs
pub static SUL_DB: Lazy<Surreal<Any>> = Lazy::new(Surreal::init);
pub static SECOND_SUL_DB: Lazy<Surreal<Any>> = Lazy::new(Surreal::init);
// ...
pub fn model_primary_db() -> &'static Surreal<Any> { &SUL_DB }
// ...
pub fn project_primary_db() -> &'static Surreal<Any> { &SUL_DB }
```

- `Surreal<Any>` = 运行时按连接串 scheme 选引擎（不是编译期定死）。
- 其余连接对象：`SECOND_SUL_DB`（二号机组，ws）、`SUL_MEM_DB`（`mem-kv-save` feature，**也是 ws 远程**，与嵌入式 mem 无关）、
  plant-model-gen 侧 `REVIEW_PRIMARY_DB: Surreal<Client>`（`src/web_api/review_db.rs:13`，校审数据独立 ws 连接，仅支持 ws）。

### 初始化入口（谁调用了 connect）

| 入口 | 路径 | 说明 |
|---|---|---|
| 主应用 `run_app` | `src/lib.rs:700` → `aios_core::initialize_databases()`（rs-core `runtime.rs:275`） | File 分支整个包在 `#[cfg(feature = "kv-rocksdb")]` 里，无该 feature 直接报错（`runtime.rs:339-345`）；含 RocksDB LOCK 自动清理（`runtime.rs:312-324`）。Ws 分支走 `init_surreal_with_retry` → `try_connect_database` → `init_surreal` |
| CLI 各模式 | `src/main.rs:342/457/1759/2888/3706`、`src/cli_modes.rs:3092/3154/3976/4560/5244/5342` 直接 `aios_core::init_surreal()` | rs-core `lib.rs:330`：读 `DB_OPTION_FILE`（默认 `db_options/DbOption.toml`）→ `effective_surrealdb()` → File: `SUL_DB.connect(("rocksdb://…", config))` 无 signin；Ws: connect + `signin(Root)`。之后 `use_ns_db_compat(SUL_DB, surreal_ns, project_name)`（NS=surreal_ns, DB=project_name）+ `define_common_functions` + `load_attr_cn_names` |
| 生成流水线幂等封装 | `src/fast_model/utils.rs:17` `ensure_surreal_init()` | OnceCell + `RETURN 1` 探针，就绪则跳过重复 `init_surreal`（防 WS router 并发死锁，spec 006） |
| web_server | `src/web_server/mod.rs:424` `initialize_databases` | 同主应用 |
| 站点体系（外部 server 进程） | `managed_project_sites.rs:10601-10609` spawn `surreal start … <site_rocksdb_conn_str>`；`src/bin/web_server.rs:80-86`（auto_start_surreal）；`db_startup_manager.rs:283-288`；远端部署 systemd 模板 `managed_project_sites.rs:13630/13758` | 数据在**外部 surreal 进程**里，web_server 用 ws 连过去。versioned 参数通过连接串透传给 server |

连接建立后所有读写都经 `project_primary_db()` / `model_primary_db()` / `model_query_response()`（rs-core `rs_surreal/mod.rs:125`）走同一个 `SUL_DB`。

## ② 引擎 / URL 决定点

配置 → 连接串的完整链：

1. **配置文件**：`DB_OPTION_FILE` env（默认 `db_options/DbOption`）→ `DbOption.[surrealdb]`
   = `SurrealDbConfig { mode, path, ip, port, user, password }`（rs-core `options.rs:37-56`，默认 `mode=File, port=8020`）。
   `DbConnMode` **只有 `File` 和 `Ws` 两个变体**（rs-core `options.rs:12-17`）。
2. **端口回落**：`DbOption::effective_surrealdb()`（rs-core `options.rs:710`）——子表 port 为默认值时回落顶层 `surreal_port`。
3. **数据目录**：`surrealdb_data_path()`（rs-core `options.rs:725`）默认 `db-data/{project_name}_{surreal_port}.rdb`。
4. **连接串**：`surrealdb_conn_str()`（rs-core `options.rs:735`）：File → `rocksdb://{path}`；Ws → `ws://{ip}:{port}`。
5. **versioned 参数只在 plant-model-gen 侧追加**：`crate::options::rocksdb_conn_str(data_path, versioned, retention)`
   （`src/options.rs:33-49`）→ `rocksdb://{path}?versioned=true&retention={r}`。
   versioned 开关来源（三处，语义一致）：
   - `DbOptionExt.versioned_storage` / `version_retention`（`src/options.rs:426/431`，默认 `false` / `"0"`=无限保留）；
   - 站点 `DbOption.toml` 顶层 key `versioned_storage` / `version_retention`（`managed_project_sites.rs:8925 read_versioned_params_from_path`）；
   - `current_versioned_params()`（`src/options.rs:669`，从 `DB_OPTION_FILE` 再读一遍，给拿不到 DbOptionExt 的路径用）。
6. **引擎层解析**（fork）：`ds.rs:582-593` 把连接串 `?k=v` 拆出来并映射为 `datastore_{k}`
   （`versioned` → `datastore_versioned`，`retention` → `datastore_retention`），
   RocksDB / mem / surrealkv 三个引擎的 Config 都认这两个 key（`rocksdb/cnf.rs:658`、`mem/cnf.rs:45`、`surrealkv/cnf.rs:110`）。
   嵌入式与 `surreal start` 走同一条解析路径（SDK `engine/local/native.rs:149 build_with_path(address.path)` 保留 query 参数）。

## ③ kv-mem 可行性与 feature 改动点

### 现状：引擎已编译，只缺配置面

- plant-model-gen `Cargo.toml:72-75` 与 rs-core `Cargo.toml:121-124` 的 surrealdb 依赖 **都已声明 `features = ["protocol-ws", "kv-mem"]`**；
  `kv-rocksdb` 才是可选的（feature `kv-rocksdb`，`Cargo.toml:271-274`，默认 `review` 集不含）。
  → **切内存引擎不需要改任何 Cargo feature**；默认瘦构建今天就带着 Mem 引擎。
- SDK 路由：`Surreal<Any>` 认 `mem` scheme（fork `src/opt/endpoint/mod.rs:168`，`engine/any/native.rs:47` 在 `kv-mem` 下启动本地 router）。
  连接串校验接受 `mem`/`memory`/`mem:…`（fork core `ds.rs:738-751`）。
- **仓库内已有先例**：rs-core 测试 helper `init_sul_db_with_memory()` 就是 `SUL_DB.connect("mem://")`
  （`../rs-core/src/test/test_surreal/test_helpers.rs:91-105`），连全局 SUL_DB 都验证过。

### fork mem 引擎（surrealmx）的真实能力（比预期强）

- `MemoryConfig { versioned, retention_ns, persist_path, sync_mode, aol_mode, snapshot_mode }`（`mem/cnf.rs:6-20`）。
- `mem://?versioned=true&retention=0` → 开 MVCC 版本化，`VERSION d'…'` 走 `scan_iter_at_version`（`mem/mod.rs:806-810`）。
- 未开 versioned 时任何带 VERSION 的读直接报 `Error::UnsupportedVersionedQueries`（`mem/mod.rs:52-57`）——
  **fail-fast，不是静默返回当前态**（这点比 specs/023 里 pe_owner 区间扫的 C3 静默陷阱好）。
- `mem:///abs/path?versioned=true&aol=sync&snapshot=60s` → surrealmx 落盘持久化（`ds.rs:628-634`、`mem/cnf.rs:37-44`）；
  纯内存与 RocksDB 之间存在一个中间档，评估时值得知道（本文其余部分按"纯易失 mem"给结论）。

### 需要动的地方（最小改动清单）

| # | 位置 | 改动 |
|---|---|---|
| 1 | rs-core `options.rs:12` `DbConnMode` | 加 `Mem` 变体（serde rename `mem`），`SurrealDbConfig::conn_str()`（:73）与 `DbOption::surrealdb_conn_str()`（:735）加 `mem://` 分支 |
| 2 | rs-core `lib.rs:330 init_surreal` + `runtime.rs:275 initialize_databases` | 加 `Mem` 匹配臂：等同 File（嵌入式、无 signin），但**不要**套 `#[cfg(feature="kv-rocksdb")]`、不要走 RocksDB LOCK 清理/端口释放逻辑 |
| 3 | plant-model-gen `src/options.rs:33 rocksdb_conn_str` | 该 helper 名字/scheme 是 rocksdb 专用；mem 模式要么新增 `mem_conn_str(versioned, retention)`，要么泛化为按 mode 组装（`?versioned=true&retention=` 参数拼法对 mem 完全同形） |
| 4 | 站点/外部 server 路径（可选） | `surreal start memory` / `mem://?versioned=true` 对外部 server 同样成立（server 与嵌入式共用 `ds.rs` 解析）；若只做嵌入式 mem，`managed_project_sites` 系列不用动 |
| 5 | 开关模式先例 | 参考 `ModelWriterMode` / `TransformWriteBackend`（`src/options.rs`）：env/toml 双入口 + `as_str()` + 默认值兜底，`versioned_storage` 已是现成的 toml 顶层 key 先例 |

### 硬约束（与引擎无关，架构层面）

- **嵌入式 mem = 单进程私有**：数据只在本进程，CLI 与 web_server 不能像现在（外部 surreal + ws 多客户端）那样共享；
  要共享只能 `surreal start memory` 外部进程 + ws（重启语义不变，仍易失）。
- `ensure_surreal_init` 的 `RETURN 1` 探针、`use_ns_db_compat`、`define_common_functions` 全部与引擎无关，可原样工作。
- review 库（`review_db.rs`）硬编码 `Ws` 客户端类型，嵌入式 mem 模式下 review 功能不可用（或需继续依赖外部 server）。

## ④ 与 specs/022/023 约束的冲突清单

| # | 约束（AGENTS.md / specs） | 内存模式下的冲突 | 严重度 |
|---|---|---|---|
| 1 | **模型表与 PE/ATT 固定同库**：`model_primary_db() == project_primary_db() == SUL_DB`（rs-core `rs_surreal/mod.rs:111-122`，分库机制已移除） | 切 mem 是**全库整体切换**：PE/ATT、pe_owner 边、锚点、模型表、ses/db_file_info 一起变易失。不存在"只把模型数据放内存、PE/ATT 留 RocksDB"的合法路径（那等于复活已被移除的双库分离，违反同库铁律） | 阻断性认知前提 |
| 2 | **versioned 是建库属性**：非 versioned 目录不能原地开 `versioned=true`（`src/options.rs:31-32`、`managed_project_sites.rs:5640-5659` 已初始化站点拒改） | mem 每次启动=全新库，"新建目录重灌"约束天然满足；但代价反转：**每次重启都强制 full 重灌**（`sync_pdms`），历史只从进程启动时刻开始积累 | 高 |
| 3 | **锚点是唯一业务入口**：`sesno_version_anchor` 固化后才对外暴露；历史查询 `resolve_anchor` → `SELECT … VERSION d'…'`（rs-core `version_query.rs:153/:223`） | 锚点表存在 SUL_DB 里 → 重启全丢。`/api/model-history/*` 的 404 AnchorMissing 会覆盖所有重启前的 sesno；"回溯任意历史版本"能力退化为"回溯本次运行内的版本" | 高 |
| 4 | **watch 增量起点 = Committed Watermark**（`sesno_version_anchor` 优先、回退 `dbnum_info_table`）；同 dbnum 增量串行 + Commit Pending/lease（`src/versioned_db/version_commit.rs`，全部经 `project_primary_db` 落 SUL_DB） | 重启后水位线消失：`startup_catchup` 无锚可追，`execute_incr_update` 无 from_sesno 基准；增量链路必须整库重灌后才能重新建立。lease/Commit Pending 的"防重入"保证也随内存清零（单进程 mem 下跨进程互斥本身已无意义） | 高 |
| 5 | **specs/023 pe_owner 可信分界**：`pe_owner_version_meta.maintained_since_sesno` 只由 full 重灌 / rebuild 建立，增量不写 meta | meta 易失 → 重启后树版本查询全部回退 `pe.children` VERSION 点查路径。读侧语义安全（设计如此），但 023 的图遍历版本查询收益清零，除非每次启动都跑 full 重灌把 meta 立起来 | 中 |
| 6 | **retention 默认 `0` = 无限保留、磁盘只增不减**（specs/022 ops-notes） | mem 同语义：`retention=0` + versioned=true 时**全量历史驻留 RAM**，只增不减。大站点全量 PE/ATT + 全历史版本的内存占用需要实测预算；改短 retention（如 `1h`）可缓解但窗外查询报过期 | 中（容量风险） |
| 7 | **同库数据的持久性分级**（哪些允许易失） | 见下表 | — |

### 数据持久性分级（mem 模式下）

**本来就在盘上、不受影响**：tree index rkyv（`versioned_db/tree_export`）、mesh GLB 文件、parquet 导出、
SQLite 空间索引（`spatial_index.rs`）、model_relation_store sled/SQLite（`model_relation_store*.rs`）、
`db_meta_info.json`、`DbOption-parse.toml` / manifest、站点注册 SQLite。

**SUL_DB 内、允许易失（可重算，代价=重跑）**：`aabb`/`trans`/`vec3`、`inst_relate*`/`inst_geo` 元数据、
`scene_node`、`inst_relate_bool` 等生成态（regen 流水线本来就能全量重建）。

**SUL_DB 内、必须持久（丢了破坏正确性/增量语义）**：`pe`/`att`/uda、`pe_owner` 边、
`sesno_version_anchor`、`dbnum_info_table`、version commit lease / Commit Pending、`ses` 表、
`db_file_info`、review 系列表。——这组数据决定了 mem 模式的适用边界。

### 结论：内存模式的合法使用面

- **适合**：一次性 `解析 → sync_pdms(full) → gen model → 导出(parquet/GLB/SQL 文件)` 的批处理/CI/验证场景
  （`review` feature 注释"SurrealDB 默认走 kv-mem 后端，校审验证够用"即此意，`Cargo.toml:202`）；
  以及需要 VERSION 语义的短生命周期测试（`mem://?versioned=true` 可行，且比 RocksDB 少一次建目录）。
- **不适合**：长驻站点（watch 增量、锚点历史、跨重启回溯）。这些场景的正确形态仍是 specs/022 的
  外部 `surreal start rocksdb://…?versioned=true` + ws。
- 若要"内存性能 + 有限持久"，fork 的 `mem:///path`（surrealmx AOL/snapshot）是未被现有配置面暴露的第三选项，
  但其崩溃一致性/与 UDT versioned 的组合行为未经本仓库验证，启用前需单独评估。

## 附带发现（顺手记录，不属于本任务改动范围）

1. **`file:` scheme 已被 fork 移除**（`ds.rs:650-655` 明确报错 "The `file://` scheme is no longer supported"），
   但 `src/web_server/db_startup_manager.rs:287` 非 versioned 分支仍拼 `file:{db_file_with_port}`。
   若部署的 surreal.exe 是本 fork 构建，非 versioned 站点经该路径启动会直接失败——建议统一改走 `rocksdb://`。
2. rs-core `runtime.rs` File 模式与 `init_surreal` File 模式行为不一致：前者被 `kv-rocksdb` cfg 门控并有 LOCK 清理，
   后者（`lib.rs:346-366`）无 cfg 门控——默认构建下 `init_surreal` 的 File 分支会在运行时因引擎未编译而 connect 失败，
   错误信息不如 `initialize_databases` 的编译期提示友好。加 `Mem` 变体时顺手对齐两处更稳妥。
