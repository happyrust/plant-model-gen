# Feature Specification: ZONE 切片站点的 kv-mem 生成与产物回灌（spec 029）

> ADR-0015 的实施规格；spec 028（mem 生成缓存轮转）的先导切片。
> 案例：ZONE `=24381/144870`（AvevaMarineSample / DESI dbnum 7997）。

## User Need

对单个 refno（ZONE）子树：完全用 kv-mem 装载其全部设计数据 + CATA 依赖闭包 →
在 kv-mem 里生成完所有模型 → 回灌到一个全新 RocksDB 切片站点库 →
plant3d-web 通过常规站点路径正常加载该 ZONE。

## Evidence

### 案例画像（8042 实测 + 日志）

- ZONE `/Copy-of-1RCS-1RX-LD`：`child_count=137` 全部 PIPE，每 PIPE 1~9 个 BRAN；
  scope 展开 `Mixed roots=2012 → GenerationTargets generation=165`。
- spec 002 闭包（`--include-design-subtree`，含 owner 祖先链）：DESI 白名单约 2000 元素，
  4 个 CATA 库（seeds=2293 / visited=3138 / missing=6）。
- 2026-08-03 修复复验后的裁剪解析 + scoped 生成产物（mem 库 `pe=6606`）：
  `inst_relate=1645`、`tubi_relate=939`、`geo_relate=3437`、`inst_relate_aabb=1276`、
  `inst_info=412`、`mesh_results=259`，`model_gen` 锚点 `dbnum=7997 sesno=76` 正常发布。
  「裁剪解析 + scoped 生成」链路已通，本 spec 的工程部分以此为前提。

### 合法跳过 ≠ 漏产（验收基准的关键口径）

- `PARAM n` 表达式取值自 SCOM 的 `PARA` 数组（`fast_model/gen_model/resolve.rs`；
  `IPARAM` 默认全 0 生成物理模型）；裁剪闭包内 SCOM 的 `PARA` 完整（实测 188/188）。
- 元件源数据存在**天生零参数**：如 `/LAO.WELD.FIELD.FF20` `PARA=[20, 26.7, 0.0, 533486]`
  → `PHEI = PARAM 3 = 0` → `validate_cate_csg_shape` 判退化跳过（E-GEO-INVALID，
  日志中 `scale=Vec3(26.7, 26.7, 0.0)` 与之吻合）。此类跳过是元件库数据性质。
- 结论：验收不得用「零 E-GEO-INVALID」做基准，必须按「生成目标 − 合法跳过」口径。

### pe_owner 审计门槛机制（ADR-0012 / 实码确认）

- 生成读三个入口都过 `ensure_hierarchy_coverage`，只认
  `pe_owner_version_meta.bulk_state='ready'`（schema 断言来源 ∈ `full_reload | rebuild_cli`）。
- 裁剪解析走 `persist_partial_hierarchy`：不发 Ready、不碰 bulk_state（fail-closed 是设计）。
- `e3d_tree_api` 用 `get_maintained_since(..).unwrap_or_default()` 是软回退，前端模型树不硬卡。
- 切片库口径见 ADR-0015 D5：裁剪集即该库全量，按 full 语义真实发 ready，不改门槛。

### 库产物与磁盘产物分离

- 磁盘产物由生成直接落文件系统，与 mem/RocksDB 无关，不回灌：
  `assets/meshes`（~155MB/11166 文件）、`assets/archives`（~307MB）、
  `output/<project>/scene_tree`（~130MB）、`output/<project>/parquet`（~10.8MB）。
- 库内产物（需回灌）：
  - 源数据：`pe`、按 noun 分表的 ATT（以 mem 库 `INFO FOR DB` 实际表集合动态枚举）、
    `pe_owner`、`pe_transform`；
  - 模型：`MODEL_TABLES` 11 张（`pdms_inst.rs::ensure_model_tables_defined`：
    `inst_relate` / `inst_relate_aabb` / `inst_relate_bool` / `inst_relate_cata_bool` /
    `refno_relations` / `neg_relate` / `ngmr_relate` / `geo_relate` / `tubi_relate` /
    `tubi_info` / `inst_geo`）+ 内容寻址 `inst_info`；
  - 值对表：`aabb` / `trans` / `vec3` / `pts`；
  - 元数据（重算不照搬）：`pe_owner_version_meta`、`sesno_version_anchor`。

### 双连接与引擎约束

- `SUL_DB` 全局单例、同库铁律（`model_primary_db() == project_primary_db()`）：
  回灌必须持有 `SUL_DB` 之外的第二连接；两端都是外部 surreal 进程 + ws（ADR-0015 D4）。
- 默认 feature 含 `kv-mem`、不含 `kv-rocksdb`；RocksDB 侧用外部进程即可，零编译负担。
- mem 引擎乐观 MVCC：并发批量写会稳定 Transaction conflict（`debug_bran_mem.ps1`
  默认 `WriteWorkers=1`）；回灌一律顺序写。

### 前端加载路径

- 验收走 quick-deploy 同款 `data_source=parquet`（viewer URL 参数）。
- parquet 导出批查 `inst_relate` / `geo_relate`；只要导出源是**回灌后的切片站点库**，
  前端验收即间接覆盖回灌完整性。
- 模型树 / 属性面板运行时读站点库 `pe` / `pe_owner` / ATT（`e3d_tree_api`），
  所以源数据侧回灌不可省。

## Scope

1. **切片站点建库**：全新空数据目录 + 外部 surreal（rocksdb 引擎，ws）；
   注册进 `managed_project_sites`，`manual_refnos=[<zone_refno>]`，显式 slice 标记；
   `meshes_path` / `file_server_host` 指向共享磁盘产物目录。
2. **装载**：spec 002 原语（`collect_design_subtree_outbound(include_owner_chain=true)` +
   `CataClosureResolver` manifest 裁剪）解析进外部 kv-mem 实例。
3. **生成**：现有 scoped 生成管线在 mem 上执行（`--debug-model <zone> --regen-model` 或等价
   CLI 入口），生成算法零改动（spec 025 五能力契约不动）。
4. **pe_owner ready**：切片库按 full 语义固化并发 ready（实现手段与并行修复会话落地机制收口）。
5. **回灌**：新增 `GenerationOutputBackfill` trait + SurrealQL 顺序首版实现；
   进程内双 ws 连接；按上文清单分组搬运；每表行数 + 耗时入运行结果。
6. **导出与验收**：回灌完成后从切片站点库导 parquet、建 spatial index / scene_tree 等导出物，
   plant3d-web 以 parquet 路径加载 ZONE。
7. **scope 参数化**：编排入口的切分单位抽象为 `refno 子树 | db file` 两种，
   回灌 trait 不感知切分单位（为 spec 028 轮转复用）。

## Non-Goals

- 不推进任何生产库的锚点 / 水位 / 欠账（ADR-0015 D1）。
- 不向已有版本历史的库回灌（那是「数据库增量接入」，必须走增量提交链路）。
- 不做 SST 直灌；二进制搬运只留 trait 位，不在首版实现。
- 不改增量链路（watch-incremental）、不改 CATA 闭包语义（spec 002）。
- 不实现嵌入式 `mem://` 承载体（外部 ws 进程先行）。
- 不新增 cargo test（AGENTS.md：验证走 CLI + JSON / HTTP）。

## Decisions（ADR-0015，grill-with-docs Q1–Q8）

| # | 决策 | 结论 |
|---|---|---|
| Q1 | 功能定位 | 离线补算工具：真写回持久库，不碰锚点/水位/欠账 |
| Q2 | 回灌目标 | 全新空库，只装这一个 ZONE 的切片站点 |
| Q3 | kv-mem 职责 | spec 028 先导切片，抽象要能长到全量轮转 |
| Q4 | 拓扑 | 两端都是外部 surreal 进程 + ws |
| Q5 | pe_owner 门槛 | 切片库「裁剪集即全量」，按 full 语义真实发 ready |
| Q6 | 回灌原语 | `GenerationOutputBackfill` trait，双连接按表流式搬运 |
| Q7 | 验收口径 | `data_source=parquet`；顺序：回灌 → 切片库导 parquet → 前端加载 |
| Q8 | 优先级 | 先修漏产（2026-08-03 已修复复验），再建本 spec 工程 |

## Requirements

1. **切片站点自足**：回灌 + 导出完成后，销毁 mem 实例，plant3d-web 仅凭切片站点
   （+ 共享磁盘产物目录）完整加载 ZONE：可见构件、TUBI 直管段、模型树、属性面板。
2. **回灌唯一路径**：模型产物与源数据进入切片库只经 `GenerationOutputBackfill`；
   生成期不得旁路直写 RocksDB。
3. **失败边界**：回灌失败不得留半态——整轮可见或整轮不可见；中途 kill 后重跑，
   结果与一次成功执行等价。
4. **观测**：逐表记录回灌行数与耗时，写入运行结果（对齐 spec 028 R8）；
   回灌行数与 mem 侧逐表一致（重算类元数据除外）。
5. **元数据重算**：`pe_owner_version_meta` 在目标库按切片全量重算发 ready；
   `sesno_version_anchor` 在切片库内按「切片即全量」口径重发（沿用源观测 sesno），
   不与任何生产库锚点混同。
6. **验收顺序**：parquet 必须从回灌后的切片站点库导出；禁止从 mem 导出充当验收。
7. **合法跳过口径**：E-GEO-INVALID 等跳过必须可分类计数（天生零参数类 vs 其它），
   验收基准 = 生成目标 − 合法跳过；分类计数进错误报告或 cache_miss_report 汇总。
8. **scope 参数化**：切分单位（refno 子树 | db file）由编排层参数决定，
   闭包计算、装载、生成、回灌四阶段共用同一抽象。
9. **并发纪律**：mem 侧写并发沿用修复会话收敛后的配置；回灌顺序写，
   不引入新的并发批量写面。

## Acceptance Criteria

- `cargo build --bin aios-database` 成功（不新增/运行 cargo test）。
- 端到端一次跑通：闭包 → mem 裁剪解析 → scoped 生成 → 回灌切片库 → 切片库导 parquet →
  plant3d-web（`data_source=parquet`）打开 ZONE：
  - 前端可见构件数与「生成目标 − 合法跳过」口径一致；
  - TUBI 直管段可见；模型树可展开到 ZONE 子树；属性面板可取 ATT。
- 回灌行数报告逐表与 mem 侧一致（`pe_owner_version_meta` / `sesno_version_anchor` 按重算口径核对）。
- mem 实例销毁后（或重启机器后）切片站点冷启动，前端仍可完整加载。
- 回灌中途 kill 进程后重跑：无重复行、无半态，最终状态与一次成功等价。
- 切片库 `pe_owner_version_meta.bulk_state='ready'` 且 `verified_sesno` / 节点数 / 边数 /
  hierarchy_hash 与切片实际内容一致（用既有审计读取路径核验，不加豁免）。

## Open Questions

1. **断点续跑粒度**：整轮重跑 vs 按表断点续搬（首版倾向整轮重跑，靠幂等 + 失败边界兜底）。
2. **锚点重发的实现位置**：回灌器内重发 vs 复用生成收尾的锚点发布路径（倾向前者，
   保证「不照搬」约束集中在一处）。
3. **切片站点的进程编排**：由 `managed_project_sites` 生命周期拉起外部 surreal(rocksdb)，
   还是脚本先行（首版脚本先行，站点管理端只注册不托管亦可接受）。
4. **合法跳过分类的落点**：`MODEL_ERROR` 汇总里加分类计数，还是 `cache_miss_report`
   增加 buckets（待与修复会话的错误报告改动对齐后定）。
5. **spec 028 收编方式**：spec 028 合入主干时，本 spec 的编排入口是否直接改造为其
   轮转循环的单步（预期是，届时本 spec 状态转 merged-into-028）。
