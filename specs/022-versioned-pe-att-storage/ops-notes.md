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
| 5 | 与交付单元版本关系 | 正交：022 管 PE/ATT **源行**历史（细粒度、retention）；导出交付单元版本不在本 feature 范围 |
| 6 | 存量迁移 | 新建 versioned 数据目录 + `sync_pdms` 全量重灌 + 写首条锚点；`DbOption.versioned_storage` 默认 **false**（建库属性），新建站点经管理端/配置显式开启（T022） |
| 7 | 查询接口 | 仅 CLI：`model-version history {snapshot,timeline,diff}`；封装在 rs-core `version_query`；暂不开 HTTP API |

## 二、versioned 实例运维约束

1. **versioned 是建库属性**：非 versioned 数据目录不能原地以 `versioned=true` 打开（UDT comparator 不匹配，启动即失败）。切换 = 新建目录 + 重灌。已初始化站点（Parsed/Failed）经管理端改 `versioned_storage` / `version_retention` 会被拒绝，不静默改参。
2. **同一 dbnum 增量必须串行**：watch-incremental 单队列语义不变；锚点一致性依赖此约束。
3. **Version Commit 强制串行**：增量入口还会获取数据库内的 per-dbnum lease，避免另一个 CLI / watch 进程绕过单队列并发写入。lease 带 owner 与过期时间；进程崩溃后可接管。
4. **锚点是唯一业务可见入口且不可变**：新提交以 fingerprint create-once 发布锚点；同 fingerprint 重跑读取原 anchored_at，不同 fingerprint 明确冲突，禁止刷新旧 sesno 的时间含义。既有无 fingerprint 锚点视为 Legacy Anchor，只读保留。
5. **Commit Pending 必须先恢复**：批次写入、计数核对或锚点发布失败会留下 pending/preparing 状态；同 dbnum 更高 sesno 被阻断。确认源观测与操作 fingerprint 未变后，用原 incremental-sesno 参数加 `--recover-pending` 幂等重放。普通运行不会静默越过 pending。
6. **retention 语义**：默认 **`0`（无限保留，全量历史）**；可按站点改为 `90d`/`30d` 等。有限窗口时 GC 以约 60s 粒度推进 `full_history_ts_low`；读时间戳低于水位线报 InvalidArgument（封装层 → `HistoryExpired`）。**仅改 retention** 影响 GC，不需重建库——但管理端对已初始化站点改 `version_retention` 仍拒绝（避免与数据目录语义漂移）；运维可手工改 toml 后重启。`retention=0` 时无 GC 过期路径，磁盘只增不减。
7. **HLC 时间戳仅进程内单调**：锚点记录数据库侧 `time::now()`，不受客户端时钟影响；服务重启后时间戳仍基于墙钟前进。
8. **模型表版本化**（更新：SurrealKV/MODEL_KV 分离机制已移除）：所有站点模型表与 PE/ATT 同库、一并版本化，磁盘增长由 retention 兜底；versioned 站点必须评估磁盘余量。
9. **retention 窗口外兜底**：PDMS 源 db 文件长期保留；CLI 过期错误提示改用源文件重扫。

## 三、磁盘水位与 retention 调整（T025）

- **观察建议**：周期性看站点 `db_data_path`（versioned RocksDB 目录）体积与增长斜率；粗估 ≈「日增量写入量 × retention 窗口」。设计库日常修改通常远小于全量，但模型表（与 PE/ATT 同库）会显著放大体积。
- **水位告警经验**：目录体积相对「非 versioned 基线」持续翻倍且接近盘余量时，将 `version_retention` 从 `0` 改为有限窗口如 `90d`（需业务确认可丢弃的历史窗口）。
- **调整方式（运维手工）**：改站点 `DbOption.toml` 的 `version_retention`（如 `"0"` / `"60d"` / `"90d"`）后**重启**站点 Surreal 进程即可，**无需重建库**；调小加快 GC，调大延后回收。默认 `0` 须确认磁盘预算。
- **不要**对已有非 versioned 目录改 `versioned_storage=true` 后直接重启——会启动失败；按 `quickstart.md` 新建目录重灌。
- **模型表历史警示**：`inst_relate` / mesh 等高频表与 PE/ATT 同库、一并进历史，磁盘代价明显高于仅 PE/ATT versioned；切换 versioned 前必须评估磁盘预算（SurrealKV/MODEL_KV 分离机制已移除，不再是可选缓解手段；如未来需要分离，走 ModelWriter parquet 后端）。

## 四、增量/数据版本架构收敛（improve-architecture 结论，2026-07-19）

一次架构评审把"增量更新 + 数据版本"的写侧从三条并行实现收敛为单一 Version Commit seam。四项深化落地：

1. **写路径唯一 seam**：`AiosDBManager::execute_incr_update` / `async_watch`（`src/data_interface/increment_manager.rs`）不再走 pdms-io 空壳 `update_elements_to_database` + 裸写 `db_file_info`；改与 CLI 共用 `collect_pdms_increment_for_file_with_operations` → `persist_collected_pdms_increment_files` → `commit_version()`。auto-watch 增量因此**首次真正落库并固化 Version Anchor**（此前是 no-op + 记账表，history 查不到）。
2. **Committed Watermark 是增量唯一合法起点**：`versioned_db::version_commit::committed_watermark(dbnum)` = `sesno_version_anchor` max sesno 优先，无锚点回退 `dbnum_info_table`，两者皆无返回 0（从未全量解析，不做增量）。**有意不以 `dbnum_info_table` 为准**——Commit Pending 时它可能领先锚点，以它为准会静默跳过半写区间；以锚点为准则重试同一区间并被 PendingCommit 拒绝，直到人工 `--recover-pending`。见 CONTEXT.md「Committed Watermark」。
3. **watch 失败语义**：提交失败的 dbnum 不推进内存 header、不发 MQTT/同步通知，下次文件事件自然重试；watch 循环内**绝不**自动 `--recover-pending`（恢复保持人工 CLI）；`LeaseBusy` 视为正常竞争跳过本周期。watch 为 persist-only，不触发模型生成（生成留给 CLI / IncrementRun）。
4. **IncrementRun 深模块**：`incremental-sesno` 编排从 `main.rs`（~450 行）移入 `version_management::increment_run::run_increment`，CLI 退化为薄 adapter（经 `ensure_model_store` 闭包注入连接策略）；HTTP 侧可复用同一入口。
5. **element_changes 幽灵 seam 移除**：该表从无写入方，`get_changes_at_sesno` / `gen_all_geos_data(target_sesno)` 路径永远命中空集——整体删除；`PdmsSesnoElementChange` 成为唯一 Element Change 类型（见 CONTEXT.md）。web handler 收到 `target_sesno` 时快速失败并指引改用 `incremental-sesno`。
6. **web 增量面只读化**：`/api/incremental/status`、`/site/{dbnum}` 是 `sesno_version_anchor` + `dbnum_info_table` + `version_commit_state` 之上的只读 adapter（水位、锚点时间线、Commit Pending）；从未实现的动作/配置端点返回 501 指引走 CLI。前端 `incremental_update.js` 同步改造为水位展示。

**硬约束**（已入根 `AGENTS.md`）：watch 增量必须走同一 Version Commit seam，禁止旁路直写；增量起点取 Committed Watermark。

对应提交：`14d95f34`（写路径/IncrementRun/幽灵 seam）、`5165df2f`（前端水位页）、`b48ae0c2`（mqtt/sync-cli 编译门；该 commit 整文件携带了相邻的 DuckLake 移除等 022 WIP）。
