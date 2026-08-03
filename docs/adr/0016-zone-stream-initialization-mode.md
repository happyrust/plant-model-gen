---
status: proposed
---

# ZoneStream：按 ZONE 双缓冲的初始化流水模式

## Context

现行初始化是「整 dbnum 解析完 → 整 dbnum 生成」的两段式串行（Legacy）。两个后果：
首个可用 dbnum 的等待时间等于全量解析 + 全量生成之和；解析与生成之间没有任何重叠，
机器在解析期不跑生成、在生成期不跑解析。

把粒度切细到 ZONE 之后，「解析下一 ZONE」与「生成、回填当前 ZONE」可以重叠：

```text
slot-a：解析 ZONE 0 → 生成子进程 → 顺序回填 → 校验/checkpoint
slot-b：             解析 ZONE 1 ──────────────→ 生成 → 回填
slot-a：                                      解析 ZONE 2 ...
```

这条路上已有三份相邻决策，但没有一份覆盖「初始化全量、按 ZONE 流水、双缓冲」：

- **ADR-0012** 把 kv-mem 限定在初始化全量生成，并立下 `pe_owner` 完整性审计门槛
  （`bulk_state='ready'`，来源限 `full_reload | rebuild_cli`）。
- **ADR-0013** 给增量 scope 做内存生成读后端，承载体是纯 Rust 快照，模型仍直写 SurrealDB。
- **ADR-0015 / spec029** 定义了 ZONE 切片站点与 `GenerationOutputBackfill` seam，
  但落点是**离线补算工具**（单 ZONE 切片站点），不是初始化主链路。

### 事实基线（2026-08-03 决策时查证）

- `GenerationOutputBackfill`、`ZoneScopeSeal`、`zone_stream` 在 `src/` 全域 **0 命中**：
  spec029 只定义了 seam，**没有任何实现可以复用**。本 ADR 因此把 seam 的首次实现纳入自身范围。
- spec 028（mem 生成缓存轮转）位于工作树 `plant-model-gen-mem-gen-cache`
  （分支 `feat/028-mem-generation-cache`），`specs/028-mem-generation-cache/` 至今**未提交**。
- `src/web_server/managed_project_sites.rs` 为 **601 KB 单文件**，
  `spawn_parse_process` / `spawn_generation_process` / `ensure_site_db_started` 均在其中；
  任何「在旧入口里加模式判断」的做法都会把 Legacy 与 ZoneStream 焊死在同一段巨型函数里。
- 默认 feature 含 `kv-mem`、不含 `kv-rocksdb`；mem 引擎为乐观 MVCC，
  高并发批量写会稳定 Transaction conflict。
- 磁盘产物（`assets/meshes`、`assets/archives`、`scene_tree`、`parquet`）由生成直接落文件系统，
  与后端引擎无关。

## Decision

### D1 独立模式，不复用任何旧开关

新增 `InitializationPipelineMode { Legacy, ZoneStream }`，TOML / 站点配置值为
`legacy | zone-stream`。

- 缺省与旧配置一律 `legacy`；**未知值启动失败，不静默回退**。
- 初始化开始后禁止切换模式；需要换模式就新建目标目录 / 站点。
- **不复用** `pipeline_db_mode`、`GenerationReadBackendKind`、`ModelWriterMode`。
  这三个开关各自承载了 file/ws、读后端、写后端的既有语义，复用会让「模式」一词多义。
- 新增 `zone_stream_memory_budget_mib`，默认 `4096`。
- 模式分流**只发生在 managed-site 编排入口**：按模式跳转到独立的 ZoneStream orchestrator，
  旧解析、旧生成入口内部不增加任何模式判断。

### D2 ZONE 是最小流水边界

同一 ZONE 必须解析并封存后才能生成；同一时间只有一个生成 / 回填下游通道。
允许「解析下一 ZONE」与「生成、回填当前 ZONE」重叠。

**不做**半 ZONE 生成，**不让**两个 generator 并行，**不做**三阶段全并发。
理由：generator 与回填都是重内存 / 重写入的阶段，并行两个会同时放大内存峰值与
mem 引擎的写冲突面，而收益只是把已经重叠掉的那一段再压一次。

### D3 承载体：单 sidecar、三逻辑库、双 slot supervisor

每次运行由系统启动一个 **loopback-only** 的 Surreal kv-mem WS sidecar，
使用动态端口和 run-scoped namespace，内含 `deps`、`slot-a`、`slot-b` 三个逻辑数据库。

- 两个常驻 slot supervisor 分别负责 A / B，维护 slot generation 与防 ABA lease。
- 每个 ZONE 使用**新的短命生成子进程**，退出即清除全局 transform、CATA、DbMeta、
  报告等缓存影响。这是用进程边界换取「全局状态不跨 ZONE 泄漏」，比逐个清理全局单例可靠。
- kv-mem 崩溃后**不恢复内存内容**，依据源清单和 RocksDB checkpoint 重建。

### D4 精确路由，禁止兜底查询

调整 `SurrealVersionedReadSession` 使其持有显式 client；在其上新增复合 session：

- 设计 PE / ATT / owner / transform 精确路由到**当前 slot**；
- 共享依赖精确路由到 **`deps`**；
- **不允许**「两库都查、取先返回者」；重复、缺失或同 ID 不同内容均**硬失败**。

snapshot 身份包含 run、slot generation、防 ABA lease、deps epoch、源哈希和精确 route map。

同时清除生成路径中绕过 session 的全局读取，包括 owner、LOOP 高度、CATA ref、
arrive/leave 和 transform fallback。Legacy adapter 仍绑定原主库，结果不变。

> 兜底查询在双缓冲下是**正确性问题**而非健壮性优化：slot-a 与 slot-b 同时存在不同 ZONE 的
> 同名记录，任何「查不到就换一个库」的回退都会静默读到另一个 ZONE 的数据。

### D5 deps epoch 先于 ZONE 流水

dbnum 按现有升序执行，ZONE 按稳定的层级 / refno 顺序执行。每个 dbnum **先**计算全部 ZONE 的
依赖并集，装载 SYSTEM、CATA 及闭包所需 DICT 元数据，生成**不可变** `deps_epoch/hash`，
之后才开始 ZONE 流水。共享依赖在一个 dbnum 内只装一次，且在流水期间不再变动。

### D6 ZONE 级只发 Seal，dbnum 级 Ready 留给整库审计

ZONE 使用新的 `ZoneScopeSeal` 证明子树、祖先链、CATA 闭包和 transform 完整。

**不修改也不伪造**现有 dbnum 级 `pe_owner Ready`。ADR-0012 的审计门槛保持原样：
只有在 dbnum 的全部 ZONE 都 Verified、并对目标 RocksDB 做过整库层级 / reference /
模型关系审计之后，才发布真实的 dbnum 级 `pe_owner/seed Ready`。

> 与 ADR-0015 D5 的差别：切片站点里「裁剪集即全量」所以可以直接按 full 语义发 ready；
> ZoneStream 的目标库最终要装完整 dbnum，单个 ZONE 完成时它**确实**不完整，因此不能发。

### D7 回填是产物进入目标库的唯一路径，seam 由本 spec 首次实现

复用 ADR-0015 / spec029 定义的 `GenerationOutputBackfill` seam。鉴于该 seam 目前**只有定义、
没有实现**（见事实基线），首次实现落在本 spec，spec029 回标为「seam 由 spec030 实现」。

首版采用双 WS、SurrealQL 顺序回填：

- 生成子进程的全局模型写库绑定当前 slot；磁盘产物写入
  `<runtime_dir>/zone-stream/<run_id>/` 私有目录；生成过程**不直接写目标 RocksDB**。
- 生成 barrier 后**先固化完整 Zone attempt manifest**，再开始目标写入。
- 幂等语义分层：ZONE 独占的设计 / 关系记录在重试时按 manifest 删除后重放；
  祖先、共享依赖及内容寻址记录**只允许同 ID、同 fingerprint 的幂等写**，
  不由单个 ZONE 删除。
- 从目标库 read-back 校验行数和 digest 后，才创建 Verified checkpoint。

### D8 只新增三类持久状态

避免重复事实源：

- 管理 SQLite `initialization_runs`：run/site、状态、源/契约/ZONE 规划哈希、目标 dbnums、
  两个 slot 状态、当前 attempt manifest、错误和完整指标 JSON。
- 目标 RocksDB `initialization_zone_checkpoint`：仅保存 create-only 的 Verified 证明；
  相同 payload 幂等，不同 payload 冲突。
- 目标 RocksDB `initialization_dbnum_publication`：dbnum 的唯一 Published 注册表和
  公共 API allowlist。

### D9 发布顺序固定，未发布即不可见

dbnum 发布顺序：

1. 所有 ZONE Verified。
2. 对目标 RocksDB 做整库层级、reference、模型关系审计，发布真实的 dbnum 级 `pe_owner/seed Ready`。
3. 从目标 RocksDB 导出 Parquet，并构建 spatial index；两者先写私有 staging，
   再校验 SHA、schema 和 row count，最后提升到最终路径。
4. 复核完整源文件集合、canonical path、header60、sesno 和逐文件 SHA-256。
5. **在同一个 Surreal 事务中**写一次 `model_gen` baseline anchor 和 create-only dbnum publication。
6. 首个 Published 后启动只读 web/viewer；所有 dbnum 完成后释放写锁并启用 watcher / 增量能力。

初始化不创建 data anchor；单个 ZONE 永不发布 model anchor。Parquet feature / 配置缺失
或 exporter 返回 skipped，在 ZoneStream 中**视为失败**。

可见性：公共 `/api/databases`、db status、`/api/v1/dbnums`、树 / 模型 / Parquet / refno 点查
和 `/files/output` 全部经过 Published gate，未发布点查返回 `404`（不是空结果）；
管理 API 仍可查看 Pending/Failed。`PartiallyReady` 阶段完全只读，
watch、增量、手工生成返回 `409`。

`PartiallyReady` 是独立的 `ManagedInitializationStatus`，**不加入**现有 `ManagedSiteStatus`；
站点可以同时为 `Running + PartiallyReady`。

### D10 contract hash 口径：预算不进 hash

Resume 是否继续同一个 run，由 contract hash 判等决定。纳入：

- 初始化模式（`zone-stream`）；
- ZONE plan 哈希；
- 源文件清单（canonical path + header60 + sesno + 逐文件 SHA-256）；
- 契约 schema 版本。

**不纳入** `zone_stream_memory_budget_mib`，预算只记录在 `initialization_runs` 行内。
理由：预算进 hash 会让「失败后调大预算再 Resume」直接变成「必须从头跑」，
与 D11 的反压设计自相矛盾。预算是执行资源参数，不是数据契约的一部分。

### D11 内存反压：先减速，再失败，不降级

总预算覆盖 mem sidecar、deps、两个 slot 和临时批次。

- 超限时**停止接收下一 ZONE** 形成反压（不打断当前 ZONE）。
- `deps + 单个 ZONE` 仍超限，则**当前 dbnum 失败**；不拆 PIPE、不回退 Legacy。

> 不拆 PIPE 是因为 PIPE 内的 BRAN 共享 arrive/leave 拓扑，拆开就破坏了 D2 的
> 「ZONE 封存后才生成」前提；不回退 Legacy 是因为静默降级会让性能基线失去意义（D1 同源理由）。

### D12 与 spec 028 的关系

spec 030 是 spec 028 的超集：028 的切分单位是「按 db file 轮转的 bootstrap 全量」，
030 的切分单位是 ZONE，且额外给出双缓冲、发布 gate 与恢复语义。
`feat/028-mem-generation-cache` 工作树冻结，spec 028 标记 `merged-into-030`。

## Consequences

- Legacy 路径逐位不变：默认值、旧入口、旧 UI / 任务、解析与生成执行顺序均不受影响，
  代价是编排层出现两套并存的 orchestrator。
- ADR-0015 / spec029 的离线切片工具与本模式共用 `GenerationOutputBackfill` seam，
  但目标库语义不同（切片库「裁剪集即全量」vs 初始化库「ZONE 只是局部」），
  因此 `pe_owner` 的发布时机必须分别处理（D6）。
- 首个 dbnum 可用时间成为一等指标：`PartiallyReady` 让站点在全部 dbnum 完成前就能只读服务。
- 残留风险：
  1. 双 slot 路由错误会静默串 ZONE 数据 —— 由 D4 的硬失败与 route map 快照身份兜底；
  2. 发布跨越 RocksDB registry 与 Surreal anchor 两种存储 —— 以 Surreal 事务为原子边界，
     RocksDB publication 做 create-only 幂等注册，冲突即失败不覆盖；
  3. 首版无 Legacy 性能对照，加速比无法量化 —— 指标 JSON 保留全部分项耗时供后续补测。

## Non-Goals（首版明确不纳入）

单 ZONE 切片站点、半 ZONE 生成、三阶段全并发、双 generator、PIPE 动态切分、
已有站点迁移、SST 直灌、嵌入式 mem 承载体、`PartiallyReady` 期间增量写入、
Legacy 性能实跑。

## References

- specs/030-zone-stream-initialization/spec.md（本 ADR 的实施规格）
- [ADR-0015](0015-zone-slice-site-kvmem-generation-backfill.md) 与
  [spec029](../../specs/029-zone-slice-kvmem-backfill/spec.md)（`GenerationOutputBackfill` seam 的定义方）
- ADR-0012（kv-mem 边界与 `pe_owner` 审计门槛）、ADR-0013（增量内存生成读后端）
- specs/002-on-demand-cata-closure（闭包原语）、specs/017-sidecar-process-reaper（sidecar 生命周期）
- specs/025-versioned-generation-read-session（生成读会话五能力契约）
- spec 028（工作树 `plant-model-gen-mem-gen-cache`，按 D12 冻结）
