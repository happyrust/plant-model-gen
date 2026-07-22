---
status: accepted
date: 2026-07-22
supersedes: ADR-0002
---

# 版本真相收敛为单源：Surreal MVCC + sesno 锚点，DuckLake 整体退役，解析路径零版本痕迹

我们决定撤销 ADR-0002「DuckLake 作为数据版本权威」的方向，回到并收窄 ADR-0001 的立场：**数据版本的唯一真相源是 SurrealDB(RocksDB) 实例级 MVCC + `sesno_version_anchor` 锚点；版本只由增量更新产生，全量解析路径不写任何版本痕迹**。DuckLake 权威链（staging → 指纹 seal → authority snapshot → 副本原子复制 + manifest 校验）、`version_store/{authority,parse_staging,legacy_bridge,replica}`、`generation_replica_*` 五张副本表、`bootstrap-generation-read` CLI、双后端 perf gate 与离线包 DuckDB 扩展资产全部删除。

关键取舍：

- **解析零痕迹**：全量解析只更新 `dbnum_info`/`db_meta` 的 `latest_sesno`（它是增量起点的既有回退源，与版本机制无关），不再固化 `source='full'` 锚点、不再运行 DuckLake 重链。首个增量提交之前，历史查询对该库诚实返回空；基线状态等价于「首个增量锚点减去该增量」，无真实消费方。
- **已提交水位语义不变**：优先 `sesno_version_anchor`（此后仅 `incremental`/`model_gen` 两种 source 会新增），无锚点回退 `dbnum_info_table`。增量提交的 lease / Commit Pending / 指纹幂等 / ContinuityGap 门禁（specs/022）原样保留。
- **存量切换为硬切分级拒绝**：行为键 `parse_storage_backend=ducklake`、`generation_read_backend=ducklake|compare` 在启动校验（`get_db_option_ext` 校验链）中硬错误——拒绝静默改变站点显式声明过的存储语义；惰性路径键（`ducklake_*`、`duckdb_*`）仅警告；磁盘 `.ducklake` 目录保留为只读遗物，不迁移不删除。
- **交付台账 `model_unit_commit` 搬入模型同库的 Surreal 表，零数据迁移**：该表自 ADR-0005（2026-07-22）出生即在 DuckLake metadata 库，存量数据寿命以天计；新表空表起步，旧账随遗物封存，外置 duckdb CLI 脚本作可选救济（不进主二进制、不留依赖）。ADR-0005 的最小交付单元语义原封不动。
- **放弃的能力**：独立于 RocksDB 的列式权威副本（审计对账、compare 双后端校验）。窗外/损坏兜底回到唯一原则：重新解析 PDMS 源文件（维持 ADR-0001 retention=0 默认前提）。

> 2026-07-23 修订：上文“首增量前不建立基线”及“此后仅 incremental/model_gen 新增”的判断已由 ADR-0010 修订。新站首次增量显式建立 `incremental_baseline`；legacy `full` 只读兼容，既有 legacy 历史不回填 baseline。

被否决的替代方案：① 保留 DuckLake 权威、把 bootstrap 挪到首次增量——版本仍只由增量产生，但双后端供养全保留，与简化诉求矛盾；② DuckLake 降级为纯交付台账存储——为一张表多养一套存储引擎；③ 软退役过渡期（保留配置键警告回落）——双路径再养一个发布周期，且警告在无人看日志的站点形同虚设。

模型生成读路径的对应决策见 ADR-0008；ADR-0006 的模型水位/欠账闭环不受影响（其依赖的锚点与水位语义由本 ADR 继续保证）。术语变更见根 `CONTEXT.md`（权威版本库、版本化读副本、两种 snapshot 绑定词条废除）。

初始化 staging、首次增量 `incremental_baseline`、legacy `full` 只读兼容及已有历史目录禁止原地重解析的后续边界见 ADR-0010。
