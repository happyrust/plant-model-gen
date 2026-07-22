# 解析直写 DuckLake 多库设计

## 目标与边界

PDMS 解析结果先写入按 `run_id/dbnum` 隔离的本地 DuckDB 暂存库；单库解析、层级校验和 transform 补齐全部完成后，才作为一个 DuckLake 权威 snapshot 原子发布。SurrealDB 是该 snapshot 的版本化读取副本，不是并行权威写入路径。

配置必须选择 `parse_storage_backend = "ducklake" | "surreal_legacy"`。`ducklake` 不允许自动降级或独立双写；`surreal_legacy` 只保留现有解析兼容路径。`save_db = false` 时，两种后端均不发布数据，只保留 tree/meta 生成行为。

## 暂存状态机

每个暂存库由 `(run_id, dbnum)` 唯一定位：

1. `created`：创建 schema，写入 run、dbnum 与输入版本元数据。
2. `parsing`：按 chunk 幂等写入 element、attribute、reference、hierarchy 和 catalog 事实。
3. `facts_sealed`：核对 chunk 计数及滚动哈希，禁止继续写解析事实。
4. `transforms_finalized`：从暂存事实计算全部有效元素的 world transform，并完成覆盖、端点与无环校验。
5. `sealed`：生成不可变 stage fingerprint，可供 authority 读取。
6. `authority_committed`：fingerprint 已唯一绑定 DuckLake snapshot。
7. `replica_applied`：Surreal binding 与权威 manifest 双向校验通过。

任何状态只能单向推进。未达到 `sealed` 的暂存库不得进入生成读取接口；`authority_committed` 之前失败不会产生可见 snapshot。

## Chunk 幂等与事实编码

- element 和 attribute 的业务键均为 `(dbnum, refno)`；跨库相同低位 refno 必须物理隔离。
- 相同键、相同 payload/hash 的 chunk 重放成功；相同键、不同内容立即报冲突。
- ATT 仅通过 `AttributeSet::from_named_attr_map` 编码，生成读取与 replica 禁止再解释解析器内部 JSON。
- reference、hierarchy 端点必须能在同一 stage 中解析；父子边不得跨 dbnum。
- 滚动哈希只用于检测暂存重放与损坏，权威 snapshot 仍使用既有 canonical payload hash 和 fingerprint。

## Transform 完成语义

`StagingTransformFactSource` 从暂存库批量提供 ATT、owner、children、ancestor 与 PLINE 事实。根节点没有 local transform 时使用 identity；子节点没有 local transform 时继承父 world transform。显式 local transform 按父 world 组合。

finalize 必须满足：

- 每个有效 element 恰有一个 world transform；
- 层级无环且所有父节点存在；
- 策略查询失败、事实覆盖不足或 transform 非有限值均失败封闭；
- transform 完成前不得 seal stage。

## 单库提交和多库 manifest

`commit_staged_db` 以只读方式 attach 一个 sealed stage，并在单个 DuckDB 事务内：

1. 校验 stage fingerprint、dbnum、sesno 和状态；
2. 替换目标 dbnum 的 element、attribute/reference、hierarchy、transform 与 catalog 分区；
3. 保留 manifest 中其他 dbnum 的已提交版本；
4. 更新目标 `data_version_state` 与全局 manifest；
5. 写入唯一 commit metadata 并提交事务。

提交后必须按 fingerprint 唯一解析 snapshot，从 pinned snapshot 回读 manifest、计数和 payload hash。sesno 回退、历史缺洞、损坏 payload、跨库边或同 fingerprint 多 snapshot 均被拒绝。

## 副本与恢复

权威提交成功后，由 pinned snapshot 构造并 seal `ReplicaApplyBatch`。Surreal apply 完成后，必须验证：

- binding 的 `authoritative_snapshot_id` 与 DuckLake snapshot 一致；
- `manifest_hash` 与权威 manifest 完全一致；
- replica version time 能回读同一计数和 payload hash。

若副本失败，保留 stage、fingerprint 与 snapshot id，状态停在 `authority_committed`。恢复只允许按 fingerprint 重试同一 replica batch；不得重新发布权威 snapshot。

## 隔离、运维与验收

默认暂存根目录为 `runtime/ducklake/staging`，实际路径为 `{root}/{run_id}/{dbnum}.duckdb`。测试和站点运行必须覆盖该目录以及 DuckLake metadata/data/temp、Surreal、mesh/cache 和输出目录，避免污染现有数据。

发布按单库灰度、多库 compare、站点切换三个阶段进行。门禁包括双库隔离、故障恢复、DuckLake/Surreal DTO 与 missing 集对拍、transform 覆盖、`GenerationArtifacts`/最终模型语义哈希，以及 AvevaMarineSample 7997 的真实按需解析和模型导出。
