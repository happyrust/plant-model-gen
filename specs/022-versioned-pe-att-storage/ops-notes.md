# 运维说明与决策沉淀: PE/ATT versioned 存储 (specs/022)

> **2026-07-20 更新**：版本化架构已由 `specs/024-unified-rocksdb-versioning` 统一收敛（模型锚点、DuckLake 交付链退役、retention=0 前提），本文与 023 的共存表述以 024 与 `docs/adr/0001` 为准。

**Date**: 2026-07-16 | 对应 tasks.md T024 / T025（已关闭）

> 七项决策与运维约束已沉淀于此；**运维约束精简版**已搬入根目录 `AGENTS.md`（T024，`alwaysApply`）。切换手册见 `quickstart.md`。

## 一、七项 grill 决策（2026-07-13）

| # | 决策点 | 结论 |
|---|--------|------|
| 1 | 启用范围 | SUL_DB（项目主库，RocksDB）实例级开 `versioned=true`；~~模型高频写数据靠 MODEL_KV 分离隔离~~（更新：SurrealKV/MODEL_KV 分离机制已整体移除，模型表与 PE/ATT 同库并一并版本化，磁盘靠 retention 兜底） |
| 2 | retention | 默认 **`0`（无限保留）**，DbOption 可配置，透传到 `surreal start` / `rocksdb://…?versioned=&retention=`；可按站点改为 `90d`/`30d` |
| 3 | 版本锚点 | `sesno_version_anchor:[dbnum,sesno,source]`；数据源为 `full/incremental`，模型生成完成源为 `model_gen` |
| 4 | 删除语义 | 保持硬 DELETE，删除前状态由 versioned 存储层通过 `VERSION $t` 回答 |
| 5 | 交付 | 023 release/DuckLake 链已退役；交付由 `model-version export --dbnum --sesno` 从 `model_gen` 锚点即时导出 |
| 6 | 存量迁移 | 新建 versioned 数据目录 + `sync_pdms` 全量重灌 + 写首条锚点；`DbOption.versioned_storage` 默认 **false**（建库属性），新建站点经管理端/配置显式开启（T022） |
| 7 | 查询接口 | 仅 CLI：数据 `snapshot/timeline/diff`，模型 `model-snapshot/model-diff`，封装在 rs-core `version_query`；不新增 HTTP 历史 API |

## 二、versioned 实例运维约束

1. **versioned 是建库属性**：非 versioned 数据目录不能原地以 `versioned=true` 打开（UDT comparator 不匹配，启动即失败）。切换 = 新建目录 + 重灌。已初始化站点（Parsed/Failed）经管理端改 `versioned_storage` / `version_retention` 会被拒绝，不静默改参。
2. **同一项目写入必须串行**：`watch-incremental` 是唯一常驻增量实现，并全生命周期持有 `output/<project>/incremental.lock`；incremental/regen/full generation 争用同一把跨进程 OS advisory lock。
3. **Version Commit 双层串行**：项目文件锁之外，数据 commit 仍获取数据库内 per-dbnum lease；lease 带 owner 与过期时间，进程崩溃后可接管。
4. **锚点是唯一业务可见入口**：数据锚点（`full/incremental`）以 fingerprint create-once；模型锚点（`model_gen`）只在全部模型写入与后处理成功后 UPSERT，同 sesno 成功重跑刷新时间。数据查询不得解析 model_gen，模型查询/导出不得解析数据锚点。
5. **Commit Pending 必须先恢复**：批次写入、计数核对或锚点发布失败会留下 pending/preparing 状态；同 dbnum 更高 sesno 被阻断。确认源观测与操作 fingerprint 未变后，用原 incremental-sesno 参数加 `--recover-pending` 幂等重放。普通运行不会静默越过 pending。
6. **retention 语义**：默认 **`0`（无限保留，全量历史）**；可按站点改为 `90d`/`30d` 等。有限窗口时 GC 以约 60s 粒度推进 `full_history_ts_low`；读时间戳低于水位线报 InvalidArgument（封装层 → `HistoryExpired`）。**仅改 retention** 影响 GC，不需重建库——但管理端对已初始化站点改 `version_retention` 仍拒绝（避免与数据目录语义漂移）；运维可手工改 toml 后重启。`retention=0` 时无 GC 过期路径，磁盘只增不减。
7. **HLC 时间戳仅进程内单调**：锚点记录数据库侧 `time::now()`，不受客户端时钟影响；服务重启后时间戳仍基于墙钟前进。
8. **模型表版本化**（更新：SurrealKV/MODEL_KV 分离机制已移除）：所有站点模型表与 PE/ATT 同库、一并版本化；模型 record ID 为纯 refno，历史代由 MVCC + `model_gen` 锚点回答。
9. **retention 窗口外兜底**：PDMS 源 db 文件长期保留；CLI 过期错误提示改用源文件重扫。

## 三、磁盘水位与 retention 调整（T025）

- **观察建议**：周期性看站点 `db_data_path`（versioned RocksDB 目录）体积与增长斜率；粗估 ≈「日增量写入量 × retention 窗口」。设计库日常修改通常远小于全量，但模型表（与 PE/ATT 同库）会显著放大体积。
- **水位告警经验**：目录体积相对「非 versioned 基线」持续翻倍且接近盘余量时，将 `version_retention` 从 `0` 改为有限窗口如 `90d`（需业务确认可丢弃的历史窗口）。
- **调整方式（运维手工）**：改站点 `DbOption.toml` 的 `version_retention`（如 `"0"` / `"60d"` / `"90d"`）后**重启**站点 Surreal 进程即可，**无需重建库**；调小加快 GC，调大延后回收。默认 `0` 须确认磁盘预算。
- **不要**对已有非 versioned 目录改 `versioned_storage=true` 后直接重启——会启动失败；按 `quickstart.md` 新建目录重灌。
- **模型表历史警示**：`inst_relate` / mesh 等高频表与 PE/ATT 同库、一并进历史，磁盘代价明显高于仅 PE/ATT versioned；切换 versioned 前必须评估磁盘预算。ModelWriter DuckLake/Parquet 发布后端已退役，不是容量回避手段；容量不足只能经业务确认采用有限 retention 或扩容。

## 四、Spec 024 单一增量与模型历史架构（2026-07-20）

1. **唯一增量 runner**：CLI、`sync_live` 与 remote runtime 都调用
   `version_management::watch_incremental::run_watch_incremental`；旧 notify watcher、
   `increment_manager.rs` 与 MySQL `INCREMENT_DATA` 已删除。MQTT 只保留在独立
   `mqtt_file_sync.rs` 做源文件分发，不参与 commit。
2. **Committed Watermark 是唯一合法起点**：
   `committed_watermark(dbnum)` 只统计 `full/incremental` 数据锚点，优先锚点、
   无锚点才回退 `dbnum_info_table`。Commit Pending 时不会静默越过半写区间。
3. **IncrementRun 是唯一写 seam**：采集、源文件写前/写后 hash 门禁、PE/ATT
   commit、可选模型生成和 `model_gen` 锚点收尾在同一编排内。manifest 归档、
   publication handoff 与 release-id 参数均已删除。
4. **失败语义**：失败 dbnum 不推进水位、不发布 MQTT；watch 不自动
   `--recover-pending`。模型生成任一下游阶段失败都不写 model_gen，历史查询自然
   回退上一模型锚点。
5. **幽灵 seam 已删除**：数据库 `element_changes` 读取、`DbOption.target_sesno`
   与 `gen_all_geos_data(target_sesno)` 参数链不存在；IncrementRun 内存中的
   `element_changes` 仅是本批报告。
6. **历史与交付**：rs-core 对同一 anchored_at 查询关系及引用的
   `inst_info/inst_geo`；过期统一映射 `HistoryExpired`。v3-json 锚点导出缺锚点、
   过期或引用记录不完整时 fail closed，绝不回退当前态。

**硬约束**（已入根 `AGENTS.md`）：watch 增量必须走同一 Version Commit seam，禁止旁路直写；增量起点取 Committed Watermark。

对应提交：`14d95f34`（写路径/IncrementRun/幽灵 seam）、`5165df2f`（前端水位页）、`b48ae0c2`（mqtt/sync-cli 编译门；该 commit 整文件携带了相邻的 DuckLake 移除等 022 WIP）。

### 锚点链审计与历史断链修复口径（2026-07-20 增量加固批次一）

- **审计方法**：`powershell -File scripts/smoke/anchor_continuity_audit.ps1 -Ns <surreal_ns> -Db <project>`（连接默认 `127.0.0.1:8030` root/root，可参数/环境变量覆盖）执行 `db-data/audit_anchor_continuity.surql`：第一段输出按 `(dbnum, sesno)` 升序的全量锚点链，第二段列出断链可疑项——`from_sesno` 存在、`source != 'full'` 且 `from_sesno != 前一锚点 sesno + 1`（无 `from_sesno` 的 Legacy Anchor 与 full 基线重置锚点不参与判定）。结果落 `db-data/audit_anchor_continuity.out.md`，存在可疑项时退出码 1。
- **写侧门禁（增量防新洞）**：`commit_version` 对 `source=incremental` 且非 recover 的提交检查区间衔接——`from_sesno > committed_watermark + 1` 返回 `ContinuityGap` 显式失败，不再静默锚定带洞的链；full（基线重置）豁免，recover 路径由 pending fingerprint 匹配把关。
- **历史断链修复口径**：审计发现的历史洞**只能对该 dbnum 全量重灌**（重建锚点链基线），**不做"补洞"式回填**——原因：锚点是 create-once 不可变发布记录，补写过去区间必须伪造/改写既有锚点链的时间与指纹语义，会摧毁"锚点 = 已验证 Version Commit 的唯一可信入口"这一审计基础。
