# Feature Specification: ZoneStream 按 ZONE 双缓冲初始化（spec 030）

> [ADR-0016](../../docs/adr/0016-zone-stream-initialization-mode.md) 的实施规格。
> spec 028（mem 生成缓存轮转）的收编方，spec 029（ZONE 切片站点回灌）seam 的首个实现方。
> 验收数据集：AvevaMarineSample `7329 + 7997`，7997 覆盖 ZONE `=24381/144870`。

## User Need

初始化一个新站点时，不再等「全量解析完 → 全量生成完」两段串行走完才有第一个可用 dbnum。
改为以完整 ZONE 为最小流水单位、双缓冲交替推进，让「解析下一 ZONE」与「生成、回填当前 ZONE」
真实重叠，并让已完成的 dbnum 先发布、先只读可用。

Legacy 行为、入口和执行顺序必须保持不变，可随时按站点选择。

## Evidence

### 现有链路形态

- 初始化编排入口在 `src/web_server/managed_project_sites.rs`（**601 KB 单文件**）：
  `spawn_parse_process`(L9945)、`spawn_generation_process`(L10189)、
  `ensure_site_db_started`(L11840)。在这些函数内部加模式判断会把两套语义焊死，
  因此 ADR-0016 D1 要求分流只发生在编排入口。
- 站点模式相关的既有开关：`pipeline_db_mode` / `runtime_db_mode`（`ManagedSiteDbMode`，
  `models.rs` L993 附近）、`GenerationReadBackendKind`、`ModelWriterMode`。
  三者语义各自独立，ADR-0016 D1 明确不复用。
- `ManagedSiteStatus`（`models.rs` L755）为站点运行状态；`TaskType`（`models.rs` L82）为任务类型。

### seam 现状

- `GenerationOutputBackfill`、`ZoneScopeSeal`、`zone_stream` 在 `src/` 全域 **0 命中**。
  spec029 只定义 seam 未实现，本 spec 承担首次实现（ADR-0016 D7）。

### 引擎与并发约束

- 默认 feature 含 `kv-mem`、不含 `kv-rocksdb`；mem 引擎乐观 MVCC，
  高并发批量写会稳定 Transaction conflict（`scripts/debug_bran_mem.ps1` 默认 `WriteWorkers=1`）。
  → 回填一律顺序写。
- 磁盘产物由生成直接落文件系统，与后端引擎无关，不属于回填范围。

### 案例 ZONE 画像（沿用 spec029 实测）

- ZONE `=24381/144870`（`/Copy-of-1RCS-1RX-LD`）：`child_count=137` 全部 PIPE，每 PIPE 1~9 个 BRAN；
  scope 展开 2012 roots → 165 生成目标。
- spec 002 闭包（含 owner 祖先链）：DESI 白名单约 2000 元素 + 4 个 CATA 库。
- 元件源数据存在**天生零参数**（如 `/LAO.WELD.FIELD.FF20` 的 `PHEI = PARAM 3 = 0`），
  对应几何被 `validate_cate_csg_shape` 判退化跳过（E-GEO-INVALID）属**合法跳过**，
  验收基准必须按「生成目标 − 合法跳过」口径，不得用「零 E-GEO-INVALID」做基准。

## Scope

1. **配置与模式**：`InitializationPipelineMode { Legacy, ZoneStream }`（`legacy | zone-stream`）、
   `zone_stream_memory_budget_mib`（默认 `4096`）、初始化开始后禁止切换、未知值启动失败。
2. **任务与状态**：`TaskType::ZoneStreamInitialization`（Start / Stop / Resume 分别映射到
   `POST /api/admin/tasks`、`/cancel`、`/retry`）；`ManagedInitializationStatus`
   （含 `PartiallyReady`）为独立枚举。
3. **编排**：独立 ZoneStream orchestrator；旧入口内部零改动。
4. **承载体**：单个 loopback-only kv-mem WS sidecar（动态端口 + run-scoped namespace，
   含 `deps` / `slot-a` / `slot-b`）+ 两个 slot supervisor + 每 ZONE 短命生成子进程。
5. **读路由**：`SurrealVersionedReadSession` 显式 client 化 + 复合 session 精确路由；
   清除生成路径中绕过 session 的全局读取。
6. **规划**：dbnum 升序、ZONE 稳定序；per-dbnum 依赖并集 → 不可变 `deps_epoch/hash`；
   `ZoneScopeSeal`。
7. **回填**：`GenerationOutputBackfill` trait 首版实现（双 WS + SurrealQL 顺序写）、
   Zone attempt manifest、分层幂等、read-back 校验后 checkpoint。
8. **预算与反压**：总预算覆盖 sidecar + deps + 两 slot + 临时批次；超限反压；
   `deps + 单 ZONE` 超限则当前 dbnum 失败。
9. **持久状态**：`initialization_runs`（SQLite）、`initialization_zone_checkpoint`、
   `initialization_dbnum_publication`（目标 RocksDB）。
10. **发布与可见性**：六步发布顺序、Published gate、`PartiallyReady` 只读。
11. **恢复**：Stop / Resume 语义与错误分级。
12. **观测**：指标 JSON + 管理页 Start/Stop/Resume、阶段展示、耗时摘要、JSON 下载。

## Non-Goals

- 单 ZONE 切片站点（那是 spec029）、半 ZONE 生成、三阶段全并发、双 generator、PIPE 动态切分。
- 已有站点迁移、SST 直灌、嵌入式 `mem://` 承载体、`PartiallyReady` 期间增量写入。
- Legacy 性能实跑对照与固定加速门槛（首版只跑 ZoneStream 基线）。
- 不改增量链路（watch-incremental）、不改 CATA 闭包语义（spec 002）、
  不改生成算法（spec 025 五能力契约不动）。
- 不新增 cargo test（AGENTS.md：验证走 CLI + JSON / HTTP）。

## Decisions（ADR-0016）

| # | 决策 | 结论 |
|---|---|---|
| D1 | 模式开关 | 独立 `InitializationPipelineMode`；不复用 `pipeline_db_mode` / `GenerationReadBackendKind` / `ModelWriterMode`；未知值启动失败；分流只在编排入口 |
| D2 | 流水边界 | ZONE 为最小单位；同时最多一个 generator / 一个 backfill；只允许「解析下一 ZONE」与「下游当前 ZONE」重叠 |
| D3 | 承载体 | 单 loopback sidecar，`deps`/`slot-a`/`slot-b` 三逻辑库，双 supervisor，每 ZONE 短命生成子进程 |
| D4 | 读路由 | 精确路由，禁止双查取先到者；重复/缺失/同 ID 异内容硬失败 |
| D5 | 依赖 | per-dbnum 依赖并集先装载，产出不可变 `deps_epoch/hash` |
| D6 | 完整性证明 | ZONE 级发 `ZoneScopeSeal`；dbnum 级 `pe_owner/seed Ready` 只在整库审计后发布 |
| D7 | 回填 | `GenerationOutputBackfill` seam 由本 spec 首次实现；双 WS + SurrealQL 顺序写 |
| D8 | 持久状态 | 只新增三类，不造第四个事实源 |
| D9 | 发布 | 六步固定顺序；anchor 与 publication 同一 Surreal 事务；未发布点查 404 |
| D10 | contract hash | 纳入模式 / ZONE plan / 源清单 / schema 版本；**不纳入内存预算** |
| D11 | 反压 | 超限先停接下一 ZONE；`deps + 单 ZONE` 超限则当前 dbnum 失败，不拆 PIPE、不降级 |
| D12 | spec 028 | 030 是 028 的超集，028 冻结并标记 `merged-into-030` |

## Requirements

1. **Legacy 零影响**：默认配置、旧 UI / 任务、解析与生成执行顺序完全不变；
   旧 Parse/Generate 入口内部不含模式判断。ZoneStream 站点调用旧 Parse/Generate 任务返回 `409`。
2. **流水纪律可证**：同一时刻最多一个 generator、最多一个 backfill；
   指标必须能证明「下一 ZONE 解析」与「当前 ZONE 下游阶段」存在真实重叠。
3. **路由无兜底**：复合 session 不得出现跨库回退查询；重复、缺失、同 ID 异内容一律硬失败，
   错误信息带 run / slot / ZONE / 记录 ID。
4. **依赖不可变**：ZONE 流水开始后，本 dbnum 的 `deps_epoch/hash` 不得变化；
   变化即视为不可恢复错误。
5. **不伪造 Ready**：单 ZONE 完成只写 `ZoneScopeSeal`；dbnum 级 `pe_owner/seed Ready`
   必须由整库审计真实计算后发布，不加豁免开关、不改 ADR-0012 门槛。
6. **回填唯一路径**：模型产物与源数据进入目标库只经 `GenerationOutputBackfill`；
   生成期不得旁路直写目标 RocksDB。
7. **失败边界**：回填失败不留半态——整轮可见或整轮不可见；中途 kill 后重跑，
   结果与一次成功执行等价（无重复行、无残留）。
8. **幂等分层**：ZONE 独占记录按 attempt manifest 删除后重放；祖先 / 共享依赖 / 内容寻址记录
   只允许同 ID 同 fingerprint 幂等写，单个 ZONE 不得删除；fingerprint 冲突硬失败。
9. **发布原子性**：`model_gen` baseline anchor 与 create-only dbnum publication 必须同一 Surreal 事务。
   初始化不创建 data anchor；单个 ZONE 永不发布 model anchor。
10. **可见性 gate**：公共入口（`/api/databases`、db status、`/api/v1/dbnums`、
    树 / 模型 / Parquet / refno 点查、`/files/output`）未发布一律 `404`；管理 API 可见 Pending/Failed。
11. **PartiallyReady 只读**：该阶段 watch、增量、手工生成一律 `409`；
    `PartiallyReady` 不并入 `ManagedSiteStatus`，站点可同时 `Running + PartiallyReady`。
12. **导出不可跳过**：Parquet feature / 配置缺失或 exporter 返回 skipped，在 ZoneStream 中视为失败。
13. **反压与失败**：超预算停止接收下一 ZONE；`deps + 单 ZONE` 仍超限则当前 dbnum 失败，
    不拆 PIPE、不回退 Legacy。
14. **恢复语义**：Stop 在当前解析或写回批次边界生效（task `Cancelled`，run `Interrupted`，
    未完成 ZONE 不写 checkpoint）；Resume 仅在源 manifest、contract hash、ZONE plan 三者一致时
    继续同一 run，清空槽位、按 attempt manifest 清理半写独占行、跳过 Verified ZONE 与 Published dbnum。
15. **错误分级**：worker / 网络 / 导出错误只停当前 dbnum，其他 dbnum 继续；
    源漂移、同 ID 异 fingerprint、目标非空或已有业务锚点、契约不一致为不可恢复错误——
    停止全部剩余工作，已发布 dbnum 保持只读，要求显式创建 / 重置新目标后重跑。
16. **写锁**：目标写入始终持有项目 mutation lock；部分可用期间禁止其他写路径。
17. **合法跳过口径**：E-GEO-INVALID 等跳过必须可分类计数（天生零参数类 vs 其它），
    验收基准 = 生成目标 − 合法跳过。
18. **观测**：指标 JSON 带 schema version、mode、run/source/contract hash，记录
    初始化总耗时、首个 dbnum 可用时间、每 dbnum Published 时间、依赖发现/装载耗时、
    每 ZONE 解析/seal/生成/回填/验证/导出耗时、overlap 时长与比例、行数/字节数、重试次数、
    deps/slot/临时/总内存峰值。管理页摘要与下载 JSON 必须一致。

## Acceptance Criteria

验证遵守 AGENTS.md：不运行 `cargo test` 或 test target；web 启动后通过 HTTP/POST 验证，
数据库通过 aios-database CLI + JSON 查询。

- `cargo build --bin aios-database` 成功。
- 使用 AvevaMarineSample `7329 + 7997`（7997 覆盖 ZONE `=24381/144870`）：
  确认 **7329 先发布并可访问**，此时 7997 未发布、所有公共入口均隐藏（点查 `404`），
  随后两者全部可用、站点转 `Ready`。
- 验证 Legacy 默认配置、旧 UI / 任务、解析和生成顺序完全不变。
- 验证同一时刻最多一个 generator / backfill，且下一 ZONE 解析与当前下游阶段存在真实重叠
  （以指标中的 overlap 比例为证）。
- 在解析、生成、部分回填、checkpoint 前后、导出和发布**六个边界**分别终止进程，
  验证 Resume、清理和幂等性。
- 验证单 dbnum 失败继续后续 dbnum、源漂移停止全局、内存反压 / 超限两条分支、
  共享记录 fingerprint 冲突和重复运行结果 digest 稳定。
- 验证 anchor 与 publication 同事务、未发布数据和静态产物不可见、
  `PartiallyReady` 阶段所有写入口被拒（`409`）。
- 验证管理页摘要与下载 JSON 一致，关键耗时可供后续 Legacy 对比。
- 首版仅运行 ZoneStream 性能基线，不执行 Legacy 对照，也不设置固定加速门槛。

## Open Questions

1. **ZONE 稳定序的具体定义**：按层级遍历序还是按 refno 升序？两者在同一 dbnum 内都稳定，
   但影响「首个可见构件」的主观体感与 deps 装载的局部性。
2. **`initialization_zone_checkpoint` 的 payload 粒度**：只存 seal 摘要 + 逐表行数 digest，
   还是完整 attempt manifest？后者便于取证但会随 ZONE 规模线性增长。
3. **短命生成子进程的启动开销占比**：ZONE 数量大时进程启动可能吃掉重叠收益，
   需在首个基线里量化后再决定是否引入进程池（进程池会重新引入全局状态泄漏风险）。
4. **`PartiallyReady` 的前端呈现**：只读 viewer 是否要显式提示「还有 dbnum 在初始化」，
   以及是否暴露进度给公共端（当前倾向只在管理页暴露）。
5. **`deps` 库在 dbnum 之间的复用**：不同 dbnum 的依赖并集常有大量重叠，
   首版按 dbnum 重建；是否值得做跨 dbnum 的 deps 复用留待基线数据决定。
