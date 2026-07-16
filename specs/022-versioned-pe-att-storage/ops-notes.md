# 运维说明与决策沉淀: PE/ATT versioned 存储 (specs/022)

**Date**: 2026-07-16 | 对应 tasks.md T024 / T025

> 本页先把七项决策与 versioned 实例约束沉淀在 spec 目录内。**待 022 落定（M1+M2 进分支）后，再把「运维约束」小节精简搬入 `AGENTS.md`（T024）**——`AGENTS.md` 是 `alwaysApply` 会注入每个会话，落定前搬入会造成 always-on 上下文与未落定代码不一致，故此处先行、彼时同步。

## 一、七项 grill 决策（2026-07-13）

| # | 决策点 | 结论 |
|---|--------|------|
| 1 | 启用范围 | SUL_DB（项目主库，RocksDB）实例级开 `versioned=true`；模型高频写数据靠 MODEL_KV 分离隔离 |
| 2 | retention | 默认 90d，DbOption 可配置，透传到 `surreal start` 启动参数 |
| 3 | 版本锚点 | 新建 `sesno_version_anchor` 表，每次增量落库完成后固化 `dbnum + sesno → 时间戳`；历史查询按 sesno 入参、内部换算时间戳 |
| 4 | 删除语义 | 保持硬 DELETE，删除前状态由 versioned 存储层通过 `VERSION $t` 回答 |
| 5 | 与 DuckLake 关系 | 共存（specs/023）：022 管 PE/ATT **源行**历史（细粒度、retention）；023 管**导出交付单元**版本，主键 `(dbnum, refno, sesno)`，不以 `release_id` 为版本真相 |
| 6 | 存量迁移 | 重新解析建库：新建 versioned 数据目录 + `sync_pdms` 全量重灌 + 写首条锚点；新站点默认开启 |
| 7 | 查询接口 | 仅新增 CLI 子命令（`model-version history *`），暂不开 HTTP API；封装层放 rs-core `version_query` 模块 |

## 二、versioned 实例运维约束（落定后搬入 AGENTS.md）

1. **versioned 是建库属性**：非 versioned 数据目录不能原地以 `versioned=true` 打开（UDT comparator 不匹配，启动即失败）。切换 = 新建目录 + 重灌，绝不改开关原地打开。
2. **同一 dbnum 增量必须串行**：watch-incremental 单队列语义不变；锚点一致性依赖此约束。
3. **锚点是唯一业务可见入口**：增量半途失败可能在存储里留下无锚点的中间时间戳，但对外只暴露有锚点的 sesno；重跑增量覆盖后写锚点。
4. **retention 语义**：默认 90d；`retention=0` 无限保留（磁盘风险）。GC 以 60s 粒度推进 `full_history_ts_low` 水位线；读时间戳低于水位线报 InvalidArgument（封装层翻译为 `HistoryExpired`）。改 retention 只影响 GC，不需重建库。
5. **HLC 时间戳仅进程内单调**：锚点记录数据库侧 `time::now()`，不受客户端时钟影响；服务重启后时间戳仍基于墙钟前进。
6. **未开 MODEL_KV 分离的站点**：模型表也会被版本化，磁盘增长由 retention 兜底；建议评估磁盘余量或启用 KV 分离。
7. **retention 窗口外兜底**：PDMS 源 db 文件长期保留，是过期历史的最终兜底；CLI 过期错误提示改用 DuckLake 存档（023）或源文件重扫。

## 三、磁盘水位与 retention 调整

- **观察建议**：versioned 目录体积随「增量修改率 × retention 窗口」增长；PDMS 设计库日修改量通常远小于全量，但需周期性观察数据目录大小。
- **调整方式**：改站点 `DbOption.toml` 的 `version_retention` 后重启站点即可，**无需重建库**；调小只加快 GC 推进、调大只延后。
- **未开 KV 分离警示**：此类站点模型表（inst_relate/mesh 等）会被一并版本化，磁盘代价明显更高，切换前必须评估。
