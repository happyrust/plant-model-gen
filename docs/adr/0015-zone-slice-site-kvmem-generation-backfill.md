---
status: proposed
---

# ZONE 切片站点：kv-mem 装载生成 + 产物回灌全新 RocksDB（spec 028 先导切片）

## Context

用户诉求：对单个 refno（案例 ZONE `=24381/144870`）**完全使用 kv-mem** 装载其子树全部数据
与 CATA 依赖，在 kv-mem 里生成完所有模型，再写回 RocksDB；验收目标是「模型都能正常生成，
plant3d-web 正常加载」。

该诉求落在三份既有决策的夹缝里：

- **ADR-0012**：kv-mem 严格限定初始化全量生成、投影只有层级；并立下 `pe_owner` 全量完整性
  审计门槛（`bulk_state='ready'`，来源限 `full_reload | rebuild_cli`；裁剪解析永不发 Ready，
  生成读 fail-closed）。
- **ADR-0013**（proposed）：增量 scope 的内存生成读后端，但承载体是**纯 Rust 快照**而非
  kv-mem，且模型仍直写 SurrealDB，没有「先在内存生成、再回灌」阶段。
- **spec 028**（未提交，`plant-model-gen-mem-gen-cache` 工作树）：解析写 + 模型写都进 mem、
  再整体回灌 RocksDB——形态正确，但切分单位是**按 db file 轮转的 bootstrap 全量**，
  且核心 Q3（回灌原语形态）标为阻塞未决。

本次要的本质是「**spec 028 的形态 + ADR-0013 的 scope**」。本 ADR 通过 2026-08-02/03 的
grill-with-docs 会话敲定八个决策（Q1–Q8），并为 spec 028 解封 Q3。

### 事实基线（决策时查证）

- 本地没有任何已解析的持久 RocksDB 站点库（`deployment_sites.sqlite` 17 个站点数据目录全空，
  `db-data` 基线已删）；数据来源只能是源 db 文件（`D:/AVEVA/Projects/E3D2.1/AvevaMarineSample`）。
- 同库铁律：`model_primary_db() == project_primary_db() == SUL_DB`（全局单例），分库机制已移除，
  不存在「PE/ATT 在 mem、模型表在 RocksDB」的中间态；回灌是唯一同时接触两侧的环节，
  必须持有 `SUL_DB` 之外的独立连接。
- 默认 feature 含 `kv-mem`、不含 `kv-rocksdb`（嵌入式 RocksDB 需额外 C++ 编译）；
  mem 引擎为乐观 MVCC，高并发批量写会稳定 Transaction conflict（`debug_bran_mem.ps1`
  默认 WriteWorkers=1 即为此坑）。
- 磁盘产物（`assets/meshes` / `assets/archives` / `scene_tree` / `parquet`）由生成直接落文件系统，
  与后端引擎无关，不属于回灌范围。
- 案例 ZONE `=24381/144870`（`/Copy-of-1RCS-1RX-LD`）：child_count=137 全部 PIPE，每 PIPE 1~9 个
  BRAN；scope 展开 2012 roots → 165 生成目标；spec 002 闭包（含 owner 祖先链）DESI 白名单
  约 2000 元素 + 4 个 CATA 库。
- **2026-08-03 状态更新**：此前「裁剪解析 + scoped 生成」被 `pe_owner` 审计门槛挡死的问题，
  已在并行修复会话中解决并复验（fresh mem 库 `pe=6606` 裁剪量级下，产出
  `inst_relate=1645 / tubi_relate=939 / geo_relate=3437 / inst_relate_aabb=1276`，
  ZONE 量级，`model_gen` 锚点正常发布）。本 ADR 的工程部分（切片站点 + 回灌）以该修复为前提。
- 元件源数据本身存在**天生零参数**（如 `/LAO.WELD.FIELD.FF20` 的 `PARA=[20, 26.7, 0.0, 533486]`，
  `PHEI = PARAM 3 = 0`），对应几何被 `validate_cate_csg_shape` 判退化跳过（E-GEO-INVALID）
  是元件库数据性质，不是漏产；验收基准必须区分「合法跳过」与「真实漏产」。

## Decision

### D1（Q1）功能定位：离线补算工具

真写回持久站点库，但**不碰生产库的锚点 / 水位 / 欠账**。合规依据：CONTEXT.md「初始化解析」
定义——项目首次写入**全新数据目录**不回放历史会话、不形成数据版本；本工具语义 =
最小范围的初始化解析 + 初始化生成，天然无需豁免任何版本语义。

### D2（Q2）回灌目标：全新空库的切片站点

只装这一个 ZONE 的**切片站点（slice site）**。禁止向已有版本历史的库旁路直写
（那属于「数据库增量接入」领域，必须走 `persist_collected_pdms_increment_files` →
`commit_version()`，见 AGENTS.md）。站点注册复用 `managed_project_sites.manual_refnos`
现成钩子（spec 013 预留、至今未用）。切片站点配置必须显式标记 slice 身份，
防止被误当作完整 dbnum 站点使用。

### D3（Q3）kv-mem 职责：spec 028 的先导切片

ZONE 是 spec 028 缺失的最小可验证单元。抽象必须参数化切分单位——
**refno 子树 | db file** 两种 scope 共用同一套「解析进 mem → 生成 → 回灌」编排与回灌原语，
使其能长到 spec 028 的全量轮转。

### D4（Q4）拓扑：两端都是外部 surreal 进程 + ws

mem 在一个端口（现行 8042），RocksDB 站点库在另一个端口；回灌进程内持双 ws 连接。
理由：零 rs-core 改动（`SUL_DB` 单例不动）、不背 `kv-rocksdb` 嵌入式编译、
与站点部署的标准拓扑一致。嵌入式 `mem://` 承载体留作后续优化，不在本期。

### D5（Q5）pe_owner 门槛：切片库里「裁剪集即全量」

在切片站点库中，dbnum 的全量**就是** ZONE 子树 + owner 祖先链那个裁剪集，因此按 full 语义
固化 `pe_owner` 并**真实发布** `bulk_state='ready'` 是一句真话——不改门槛、不加豁免开关。
闭包已含 owner 祖先链（`collect_design_subtree_outbound(include_owner_chain=true)`），
审计所需节点数 / 边数 / hierarchy_hash 都能真实计算。实现手段二选一：
切片解析路径按 full 语义直发，或解析完成后在切片库执行既有 rebuild CLI；
以并行修复会话已落地的机制为准收口。

### D6（Q6）回灌原语：`GenerationOutputBackfill` trait

进程内双连接、**按表流式搬运**，解封 spec 028 Q3。要点：

- 首版实现走 SurrealQL 顺序写（无并发竞争面）；二进制搬运（fork `Datastore` 级）
  作为第二实现预留在 trait 后面，用首版实测数据决定是否值得做。
- 必须记录每表行数与耗时（对齐 spec 028 R8），失败边界为「整轮可见或整轮不可见」
  （对齐 spec 028 R3）。
- 搬运清单分三组：
  1. 源数据侧：`pe`、按 noun 分表的 ATT（表集合以 mem 库实际存在为准动态枚举）、
     `pe_owner`、`pe_transform`；
  2. 模型产物侧：`MODEL_TABLES` 11 张（`inst_relate` / `inst_relate_aabb` / `inst_relate_bool` /
     `inst_relate_cata_bool` / `refno_relations` / `neg_relate` / `ngmr_relate` / `geo_relate` /
     `tubi_relate` / `tubi_info` / `inst_geo`）+ 内容寻址的 `inst_info`；
  3. 值对表：`aabb` / `trans` / `vec3` / `pts`。
- **不照搬**：`pe_owner_version_meta` 必须在目标库按 D5 口径重算发布；
  `sesno_version_anchor` 在切片库内按「切片即全量」口径重发（沿用源观测 sesno），
  不得与任何生产库锚点混同。

### D7（Q7）验收口径：`data_source=parquet`，顺序定死

与现有 quick-deploy 站点一致走 parquet 加载，但顺序必须是：
**回灌 → 从切片站点库导出 parquet → plant3d-web 加载**。
parquet 导出本身批查 `inst_relate` / `geo_relate`，因此前端验收间接覆盖回灌完整性；
禁止从 mem 直接导 parquet 充当验收（那只证明导出脚本能跑，不证明回灌对了）。
磁盘产物不回灌，切片站点 `meshes_path` / `file_server_host` 指向共享产物目录。

### D8（Q8）优先级：先修漏产，再建工程

生成正确性先于回灌工程。该项已于 2026-08-03 在并行会话完成修复与 replay 复验
（见事实基线），本 ADR 的工程部分在其收口后实施。

## Consequences

- spec 028 的 Q3 阻塞解除；ZONE 切片成为 spec 028 抽象的第一个真实消费者，
  db file 轮转复用同一回灌原语。
- 与 ADR-0013 并存不冲突：承载体不同（kv-mem SurrealDB vs 纯 Rust 快照）、
  场景不同（离线切片补算 vs 增量读加速）；scope 参数化抽象可共享。
- 新术语提案（待并入 CONTEXT.md，避免与在途改动冲突暂记于此）：
  **切片站点（Slice Site）**——只装某 refno 子树闭包的全新站点库，库内「全量」即该裁剪集；
  **生成产物回灌（Generation Output Backfill）**——内存工作库产物进入持久站点库的唯一路径。
- 残留风险：
  1. 切片库被误用作完整 dbnum 站点（缓解：站点配置显式 slice 标记 + 注册表 `manual_refnos`）；
  2. 元件天生零参数导致的合法跳过与真实漏产混淆（缓解：验收按
     「生成目标 − 合法跳过」口径，错误报告区分分类计数）；
  3. mem 乐观 MVCC 的并发写冲突（缓解：回灌顺序写；生成期写并发沿用修复会话收敛后的配置）。

## References

- specs/029-zone-slice-kvmem-backfill/spec.md（本 ADR 的实施规格）
- specs/028-mem-generation-cache（工作树 `plant-model-gen-mem-gen-cache`，形态母体）
- specs/002-on-demand-cata-closure（闭包与裁剪解析原语）
- specs/013-bran-scoped-generation（scoped 生成 + `manual_refnos` 钩子）
- ADR-0012 / ADR-0013（kv-mem 边界与内存生成读）
- 决策过程：2026-08-02/03 grill-with-docs 会话（Q1–Q8），案例 ZONE `=24381/144870`
