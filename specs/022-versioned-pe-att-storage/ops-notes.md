# 运维说明与决策沉淀: PE/ATT versioned 存储 (specs/022)

**Date**: 2026-07-16 | 对应 tasks.md T024 / T025（已关闭）

> 七项决策与运维约束已沉淀于此；**运维约束精简版**已搬入根目录 `AGENTS.md`（T024，`alwaysApply`）。切换手册见 `quickstart.md`。

## 一、七项 grill 决策（2026-07-13）

| # | 决策点 | 结论 |
|---|--------|------|
| 1 | 启用范围 | SUL_DB（项目主库，RocksDB）实例级开 `versioned=true`；~~模型高频写数据靠 MODEL_KV 分离隔离~~（更新：SurrealKV/MODEL_KV 分离机制已整体移除，模型表与 PE/ATT 同库并一并版本化，磁盘靠 retention 兜底） |
| 2 | retention | 默认 **`0`（无限保留）**，DbOption 可配置，透传到 `surreal start` / `rocksdb://…?versioned=&retention=`；可按站点改为 `90d`/`30d` |
| 3 | 版本锚点 | `sesno_version_anchor` 表，增量/全量落库成功后固化 `dbnum + sesno → 时间戳`；历史查询按 sesno 入参、内部换算时间戳 |
| 4 | 删除语义 | 保持硬 DELETE，删除前状态由 versioned 存储层通过 `VERSION $t` 回答 |
| 5 | 与 DuckLake 关系 | 共存（specs/023）：022 管 PE/ATT **源行**历史（细粒度、retention）；023 管**导出交付单元**版本，主键 `(dbnum, refno, sesno)`，不以 `release_id` 为版本真相 |
| 6 | 存量迁移 | 新建 versioned 数据目录 + `sync_pdms` 全量重灌 + 写首条锚点；`DbOption.versioned_storage` 默认 **false**（建库属性），新建站点经管理端/配置显式开启（T022） |
| 7 | 查询接口 | 仅 CLI：`model-version history {snapshot,timeline,diff}`；封装在 rs-core `version_query`；暂不开 HTTP API |

## 二、versioned 实例运维约束

1. **versioned 是建库属性**：非 versioned 数据目录不能原地以 `versioned=true` 打开（UDT comparator 不匹配，启动即失败）。切换 = 新建目录 + 重灌。已初始化站点（Parsed/Failed）经管理端改 `versioned_storage` / `version_retention` 会被拒绝，不静默改参。
2. **同一 dbnum 增量必须串行**：watch-incremental 单队列语义不变；锚点一致性依赖此约束。
3. **锚点是唯一业务可见入口**：增量半途失败可能在存储里留下无锚点的中间时间戳，但对外只暴露有锚点的 sesno；重跑增量覆盖后写锚点。
4. **retention 语义**：默认 **`0`（无限保留，全量历史）**；可按站点改为 `90d`/`30d` 等。有限窗口时 GC 以约 60s 粒度推进 `full_history_ts_low`；读时间戳低于水位线报 InvalidArgument（封装层 → `HistoryExpired`）。**仅改 retention** 影响 GC，不需重建库——但管理端对已初始化站点改 `version_retention` 仍拒绝（避免与数据目录语义漂移）；运维可手工改 toml 后重启。`retention=0` 时无 GC 过期路径，磁盘只增不减。
5. **HLC 时间戳仅进程内单调**：锚点记录数据库侧 `time::now()`，不受客户端时钟影响；服务重启后时间戳仍基于墙钟前进。
6. **模型表版本化**（更新：SurrealKV/MODEL_KV 分离机制已移除）：所有站点模型表与 PE/ATT 同库、一并版本化，磁盘增长由 retention 兜底；versioned 站点必须评估磁盘余量。
7. **retention 窗口外兜底**：PDMS 源 db 文件长期保留；CLI 过期错误提示改用 DuckLake 存档（023）或源文件重扫。

## 三、磁盘水位与 retention 调整（T025）

- **观察建议**：周期性看站点 `db_data_path`（versioned RocksDB 目录）体积与增长斜率；粗估 ≈「日增量写入量 × retention 窗口」。设计库日常修改通常远小于全量，但模型表（与 PE/ATT 同库）会显著放大体积。
- **水位告警经验**：目录体积相对「非 versioned 基线」持续翻倍且接近盘余量时，将 `version_retention` 从 `0` 改为有限窗口如 `90d`（需业务确认可丢弃的历史窗口）。
- **调整方式（运维手工）**：改站点 `DbOption.toml` 的 `version_retention`（如 `"0"` / `"60d"` / `"90d"`）后**重启**站点 Surreal 进程即可，**无需重建库**；调小加快 GC，调大延后回收。默认 `0` 须确认磁盘预算。
- **不要**对已有非 versioned 目录改 `versioned_storage=true` 后直接重启——会启动失败；按 `quickstart.md` 新建目录重灌。
- **模型表历史警示**：`inst_relate` / mesh 等高频表与 PE/ATT 同库、一并进历史，磁盘代价明显高于仅 PE/ATT versioned；切换 versioned 前必须评估磁盘预算（SurrealKV/MODEL_KV 分离机制已移除，不再是可选缓解手段；如未来需要分离，走 ModelWriter parquet/ducklake 后端）。
