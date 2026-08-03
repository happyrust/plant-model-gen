---
status: merged-into-030
---

# Feature Specification: 按 db file 轮转的内存生成缓存库（spec 028）

> **2026-08-03 收编说明（[ADR-0016](../../docs/adr/0016-zone-stream-initialization-mode.md) D12）**
>
> 本 spec 由 [spec 030（ZoneStream 按 ZONE 双缓冲初始化）](../030-zone-stream-initialization/spec.md)
> 收编，工作树 `plant-model-gen-mem-gen-cache`（`feat/028-mem-generation-cache`）冻结。
> 此处的两份文档从该工作树原样搬入主线保存，**不再单独实施**。
>
> 收编对应关系：
>
> | spec 028 | spec 030 | 说明 |
> |---|---|---|
> | 轮转单位 = db file（1 个 DESI + 依赖层） | 流水单位 = ZONE | 030 把粒度切得更细，才有双缓冲重叠的空间 |
> | 公共依赖层灌一次常驻（Q4「分层常驻」） | 共享生成依赖库 `deps`，per-dbnum 装载并冻结为不可变 `deps_epoch/hash` | 030 用 epoch 把「允许滞后」收紧成「流水期间不得变动」 |
> | 轮转层 = 当前 DESI 的 PE/ATT + 模型产物 | ZONE 工作区 `slot-a` / `slot-b` 双缓冲 | 028 是单缓冲，无重叠 |
> | 每轮结束按 dbnum range 清理轮转层 | 每 ZONE 用短命生成子进程，退出即清全局缓存影响 | 进程边界比 range 删除更难泄漏 |
> | **Q3 回灌原语形态 = 未定（阻塞项）** | `GenerationOutputBackfill` trait，首版双 WS + SurrealQL 顺序写 | Q3 已由 ADR-0015 D6 解封、ADR-0016 D7 落实首次实现 |
> | 断点续跑以 dbnum 为单位 | ZONE 检查点 + attempt manifest + dbnum 发布注册表 | 030 的恢复粒度细一级，且区分可恢复/不可恢复错误 |
> | 验收要求与 RocksDB 直跑逐表比对 | 首版只跑 ZoneStream 基线，不做 Legacy 对照 | 030 保留全部分项耗时供后续补测 |
>
> spec 028 中仍然有效、已被 030 继承的事实证据：spec 004 的写侧冲突实测、
> 027/ADR-0008 之后「生成算法不再打 DB」的读路径现状、同库铁律
> （`model_primary_db() == project_primary_db() == SUL_DB`）、以及「单个 DESI 库无法自给自足」。

## User Need

初始化解析与首次全量模型生成整体太慢。希望改成**按 db file 逐个推进的流水线**：一个 db file 解析进 `kv-mem` 内存实例 → 在内存里完成该库的模型生成 → 把该轮产物回灌 RocksDB → 进入下一个 db file。内存峰值由此收敛到「公共依赖层 + 单个设计库」，而不是整个工程。

## Evidence

### 已有实测证据（spec 004，`ams7997_0001` release）

- 总耗时 127min；`base_write_ms` 到过 33s/批，`inst_aabb_ms` 到过 35s/批。
- `pending_mesh_outputs` 长期贴 `batch_channel_capacity` 上限（108），背压已传导到生产端。
- 失败形态是 `inst_relate_aabb:... already exists`（唯一索引脏冲突）、`Cannot COMMIT`、`写入失败超出重试限制`。
- `specs/004/architecture-and-principles.md:14,42` 明确：**不是 CPU、内存或磁盘打满**，而是下游 SurrealDB 写入慢；当时机器可用内存约 21GB。

> 这组证据的关键含义：模型写侧的痛点是**并发竞争**（冲突、重试、背压），不是原始写吞吐不足。

### 读路径现状（决定了收益不在读侧）

spec 027 / ADR-0008 之后，生成算法**不再打 DB**：

- `src/fast_model/gen_model/context.rs:39-85` — `GenerationReadContext::from_hierarchy` 在运行开始前把 hierarchy、`attributes`、`catalog_nodes`、`transforms` 全部批量预载为 `Arc<BTreeMap<..>>`。
- `src/fast_model/gen_model/session_query.rs:8-17` — 算法调用的 `get_named_attmap` 就是一次 `read.attributes.get(&refno)`。
- `src/generation_read/mod.rs:38-77` — 边界测试硬性禁止 8 个生成算法文件出现 `project_primary_db`、`aios_core::get_named_attmap(`、裸 `"SELECT ` 等。

因此**把源数据放进内存引擎对生成算法的查询没有收益**——预载后的原生 Rust map 已经比任何 SurrealDB 后端快。内存引擎的收益面是：解析写、预载批量读、模型写、以及写后回读的全表扫描。

> `docs/analysis/surreal-mem/B-read-path.md` 描述的是 027 重构前的形态，在这一点上已过时。

### 生成期仍然直连 SUL_DB 的写与回读

`pdms_inst.rs`(25) / `mesh_generate.rs`(20) / `manifold_bool.rs`(8) / `boolean_backfill.rs`(4) / `model_writer.rs`(1)。其中**写后回读**是本方案必须用「库」而不是写缓冲区的原因：

- 布尔回填 `SELECT VALUE in.id FROM inst_relate WHERE has_cata_neg = true`（全表谓词扫描）
- `fetch_cata_bool_tasks_from_db` 的 `inst_relate` + `geo_relate` 图查询
- mesh 阶段 `inst_geo` existence
- 收尾 `reconcile_missing_neg_relations` / `filter_missing_inst_aabb`
- 导出阶段 `inst_relate` / `geo_relate` 批查

现有的 `ModelWriterMode::DrainOnly` 与 `sql_file_writer.rs` 都是「写了就不管」，支撑不了这些回读。

### 引擎侧可行性（`docs/analysis/surreal-mem/C-engine-init.md`）

- `kv-mem` 已在默认 feature（`Cargo.toml:73-76`），rs-core 测试 helper 已有 `SUL_DB.connect("mem://")` 先例。缺的只是配置面：`DbConnMode` 只有 `File` / `Ws` 两个变体（rs-core `options.rs:12`）。
- fork 的 mem 引擎支持 `?versioned=true&retention=`，未开 versioned 时带 VERSION 的读 fail-fast 报 `UnsupportedVersionedQueries`，不会静默返回当前态。
- 铁律约束：`model_primary_db() == project_primary_db() == SUL_DB`（rs-core `rs_surreal/mod.rs:111-122`），分库机制已移除。**不存在「只把 PE/ATT 放 mem、模型表留 RocksDB」的中间态**——解析进 mem 必然等于该轮整个 SUL_DB 都是 mem。

### 按 db file 切分的现成零件

- 解析侧已支持文件粒度选择：`manual_db_files` / `selected_db_file_names`。
- 生成侧已支持按 dbnum 限定：`gen_pipeline.rs:654 get_filtered_dbnums`，`manual_db_nums` 一路透到底。
- 模型提交身份本来就是 `(dbnum, unit_refno, sesno)`，按 dbnum 切片与领域模型对齐。
- 依赖范围计算已有：`db_index.sqlite` 的 dbnum→dbnum 精确依赖边（`db_index.rs:870 record_dependencies`）+ `specs/002-on-demand-cata-closure` / `data_interface/cata_closure.rs`。

### 单个 DESI 库无法自给自足

一轮生成必须同时读到：`SYSTEM_SYNC_DB_TYPES = ["DICT","SYST","GLB","GLOB"]`（`database.rs:357`，属性字典与 UDA 名称）、CATA 元件库（`managed_project_sites.rs:86 RELATED_DEPENDENCY_DB_TYPES`）、以及可能的模板 DESI 库。所以轮转单位是「1 个 DESI + 它的依赖层」，不是字面意义的单个文件。

## Architecture

```
                     ┌─────────────────── kv-mem 实例（进程内，常驻）───────────────────┐
  db files ──解析──► │  公共依赖层（灌一次即常驻）：SYST / DICT / GLOB / GLB + CATA 闭包  │
                     │  ───────────────────────────────────────────────────────────── │
  DESI dbnum N ────► │  轮转层：当前 DESI 的 PE/ATT  +  本轮生成的模型产物              │
                     └────────────────────────────┬───────────────────────────────────┘
                                                  │ 生成产物回灌（顺序、无并发竞争）
                                                  ▼
                                        RocksDB（versioned=true，唯一真相源）
```

每轮：解析 DESI dbnum N 进 mem → 生成（读预载 + 写 mem + 回读 mem）→ 回灌 RocksDB → range 清理 dbnum N 的轮转层数据 → N+1。

## Scope

- 给 rs-core `DbConnMode` 增加 `Mem` 变体，打通 `init_surreal` / `initialize_databases` / 连接串组装三处（doc C §③ 最小改动清单）。
- 新增 bootstrap 编排器：按 db file 轮转的「解析 → 生成 → 回灌 → 清理」循环，公共依赖层常驻。
- 新增**生成产物回灌**阶段，作为该轮数据进入持久库的唯一路径；回灌形态抽象成 trait，首版实现走 SurrealQL 顺序写。
- 公共依赖层的预灌范围由 `db_index.sqlite` 依赖边 + CATA 闭包清单计算。
- 记录每轮的内存峰值、解析耗时、生成耗时、回灌耗时，用于验证收益。

## Non-Goals

- 不改增量链路（watch-incremental）。锚点、已提交水位、commit lease、Commit Pending 仍全部在 RocksDB 上，本期不碰。
- 不改 CATA 按需解析闭包语义（沿用 spec 002）。
- 不做 RocksDB SST 直灌。目标库是 `versioned=true`，MVCC 版本键编码必须由引擎自己生成；自己拼 SST 会破坏 `VERSION AT` 与 `sesno_version_anchor` 锚点读。
- 不复活已移除的模型表/PE 分库机制。mem 实例内部仍是同一个库。
- 不把常驻生成缓存库（增量场景）纳入本期实现，只要求抽象能容纳它。

## Decisions（grill-with-docs）

| # | 决策 | 结论 |
|---|---|---|
| Q1 | 生命周期边界 | **两段式**：本次把 bootstrap 与常驻态设计成同一套抽象，先落地 bootstrap，常驻态留后续阶段 |
| Q2 | 缓存库承载范围 | **解析写 + 模型写都进 mem**。受同库铁律约束，等价于「该轮 SUL_DB 就是 mem」 |
| Q3 | 回灌原语形态 | **未定**（见 Open Questions） |
| Q4 | 共享依赖层安置 | **分层常驻**：mem 实例不销毁，公共层灌一次常驻；DESI 逐个换入，回灌后只清该 dbnum 的轮转层数据 |

## Requirements

1. 一轮 bootstrap 结束后，RocksDB 中该 dbnum 的 PE/ATT 与模型产物必须与「直接在 RocksDB 上跑同一轮」的结果等价。
2. 回灌必须是该轮数据进入持久库的唯一路径；生成期不得有任何旁路直写 RocksDB。
3. 回灌失败的 dbnum 不得推进模型生成水位，也不得留下半态：要么整轮可见，要么整轮不可见。
4. 内存峰值必须可观测并有上限告警；超过预算时给出明确错误，不得靠 OOM 暴露。
5. 公共依赖层的预灌范围必须来自 `db_index.sqlite` 依赖边与 CATA 闭包清单，允许滞后，漂移由生成期兜底链自愈（沿用依赖闭包清单的既有语义）。
6. 轮转层清理必须按 dbnum range 删除，不得误删公共依赖层或其它已完成 dbnum 的数据。
7. `versioned` 语义：mem 侧是否开 `versioned=true` 必须显式配置，不得依赖默认值；未开时任何带 VERSION 的读必须 fail-fast（引擎已有此行为，不得绕过）。
8. 必须记录每轮的：解析耗时、生成耗时、回灌耗时、mem 峰值占用、回灌行数，写入运行结果供对比。

## Acceptance Criteria

- `cargo build --release --bin aios-database` 成功。
- 同一工程分别用「RocksDB 直跑」与「mem 轮转」跑一次 bootstrap，模型产物逐表比对一致。
- mem 轮转的总耗时相对 RocksDB 直跑基线有可量化的下降，且 `base_write_ms` / `inst_aabb_ms` 的 p95 明显下降、无 `already exists` / `Cannot COMMIT` / 重试耗尽。
- 内存峰值不超过配置预算，且在超预算时给出明确错误而非 OOM。
- 中途 kill 进程后重跑，已回灌的 dbnum 不重复生成，未回灌的 dbnum 从头重跑。

## Open Questions

### Q3：回灌原语形态（阻塞项，需要决策）

三个候选，代价与收益差别很大：

1. **SurrealQL 顺序回灌**（零 fork 改动）：收尾单线程、大批 INSERT、无 DELETE 交错、无 mesh/CPU 争抢。
   *账要算清*：本方案下 PE/ATT 并不会被写两遍 RocksDB——它是「写一遍 mem（廉价）+ 写一遍 RocksDB」，相对现状只多了那次 mem 写。所以 SurrealQL 回灌在 PE/ATT 上大致中性，在模型表上是净赚（消除 spec 004 实测的冲突/重试/背压）。
2. **二进制搬运**：在 fork 里加 mem `Datastore` → RocksDB `Datastore` 的扫描/`set` 原语。`Datastore::transaction(TransactionType, LockType) -> Transaction` 在 fork 里已是 public（`core/src/kvs/ds.rs:3353`），`Transactable` 有 set/scan。障碍：plant-model-gen 现在只依赖 SDK 层 `surrealdb`（`Cargo.toml:73`），拿不到 `Datastore`；需要加 `surrealdb-core` 依赖或在 fork SDK 层加搬运 API。索引需要一并搬或重建。
3. **SST 直灌**：已在 Non-Goals 中排除。

**倾向**：先实现 (1) 并把回灌抽象成 trait，(2) 作为第二实现预留——与 Q1 选定的「同一套抽象、分阶段落地」一致。用 (1) 的实测数据判断是否值得做 (2)。

### 其它待定

- **中途失败的断点续跑粒度**：以 dbnum 为单位重跑，还是要更细？
- **公共依赖层自身何时回灌**：第一轮结束时一次性回灌，还是等全部轮次结束？
- **`基线 sesno` 这个术语现在无主**：它当前定义为「生成缓存库中一个 db 副本灌入时对应的已提交水位」，前提是缓存库里有源数据副本。本方案下缓存库里确实有 PE/ATT，但它是**该轮新解析的**而不是从持久库灌入的副本，术语需要重新表述或废除。
- **工程实际规模未知**：`db-data` 已被清空，拿不到真实体量。公共依赖层占比决定了「分层常驻」相对「每轮全新实例」的优劣，需要真实数字复核 Q4。
