# Feature Specification: 版本单源收敛与解析零版本重构

**Created**: 2026-07-22
**Status**: Accepted for implementation
**Upstream**: `docs/adr/0007-version-single-source-surreal-mvcc.md`（supersede ADR-0002）、`docs/adr/0008-generation-primary-read-version-at.md`（supersede ADR-0003）
**术语**: 生成读取时刻 / 输入版本清单（观测记录），见根 `CONTEXT.md`；定案过程见 `docs/plans/2026-07-22-parse-zero-version-refactor-plan.md`

## User Scenarios

### US1 - 全量解析零版本痕迹

新站点全量解析完成后，仓内不存在任何版本痕迹：无 `source='full'` 锚点、无 DuckLake 产物；`dbnum_info`/`db_meta` 的 `latest_sesno` 照常更新。此时对该库的历史查询诚实返回空。

### US2 - 首次增量建立版本

解析后的首个增量以 `dbnum_info` 基线为起点衔接（ContinuityGap 门禁 watermark 回退语义放行），提交后产生该库第一个 `incremental` 锚点；历史查询自此可用。

### US3 - 全量生成起点握手

全量生成成功收尾发布 `model_gen` 锚点（sesno = 解析基线）。升级/新建站点首轮 watch 看到模型水位 = 数据水位，欠账闭环不误报 `needs_full_regen`；全量生成漏跑或中途崩溃时水位诚实为 0，watch 告警等待人工 `catch-up --allow-full-regen`。

### US4 - 增量生成读到一致切面

web 触发的增量生成与 watch 数据提交并发时，生成运行的全部读取钉在本次数据提交锚点的 `anchored_at` 时刻（`VERSION AT`），不出现跨 dbnum 撕裂读；全量生成活读当前态。

### US5 - 存量 DuckLake 配置站点升级

配置了 `parse_storage_backend=ducklake` 或 `generation_read_backend=ducklake|compare` 的站点启动即硬错误，错误信息给出删改指引；只残留 `ducklake_*`/`duckdb_*` 路径键的站点启动仅警告并正常运行；磁盘 `.ducklake` 目录原样保留。

### US6 - 交付台账连续性

升级后 unit-export 照常入账——写入模型同库的 Surreal `model_unit_commit` 表（空表起步）；确需旧 DuckLake 台账的站点用外置脚本手工导入，主二进制不含 duckdb 依赖。

## Functional Requirements

- **FR-001**: 全量解析路径 MUST NOT 写入任何版本痕迹（不固化 `full` 锚点、不运行 DuckLake staging/seal/snapshot/复制链）；MUST 仍更新 `dbnum_info`/`db_meta` 的 `latest_sesno`。
- **FR-002**: `committed_watermark` MUST 保持「`sesno_version_anchor` 优先、回退 `dbnum_info_table`」；增量提交的 lease / Commit Pending / 指纹幂等 / ContinuityGap 门禁语义 MUST 不变。
- **FR-003**: DuckLake 相关实现 MUST 全部删除：`version_store/`（`model_unit_commit` 走 FR-010）、`generation_read` 的 ducklake/compare 适配器与 factory 分支、`bootstrap-generation-read` CLI、`generation-read-ducklake` feature 与 `dep:duckdb`、perf gate 脚本与 fixture、打包脚本中的 DuckDB 扩展资产、`committed_watermark`/`publish_authority_after_apply` 的 feature fork。
- **FR-004**: 启动配置检测 MUST 分级：行为键 `parse_storage_backend=ducklake`、`generation_read_backend=ducklake|compare` 残留 → 硬错误并输出修复指引；惰性键（`ducklake_*`、`duckdb_*`）残留 → 仅警告。检测点位于 `get_db_option_ext` 校验链。
- **FR-005**: 磁盘 `.ducklake` 目录与既有导出物 MUST NOT 被自动迁移、修改或删除。
- **FR-006**: 模型生成 MUST 经领域查询 trait 直读主 PE/ATT 表；生成算法代码 MUST NOT 包含 SQL、Surreal record-id 或全局数据库调用（沿 ADR-0003 保留的边界）。
- **FR-007**: 增量生成 MUST 以本次数据提交锚点的 `anchored_at` 为唯一 as-of 时刻，运行内全部数据读取 MUST 加 `VERSION AT`；全量生成 MUST 活读且不绑定时刻。
- **FR-008**: `generation_replica_element/hierarchy/reference/transform/db_catalog` 五表、snapshot 绑定与 manifest 双向校验 MUST 删除；输入版本清单 MUST 降级为观测记录：写入运行 summary，MUST NOT 参与失败关闭或覆盖校验。
- **FR-009**: 全量/手动生成成功收尾 MUST 发布 `model_gen` 锚点（sesno = `dbnum_info_table` 当前值）；生成或后处理失败 MUST NOT 发布（维持 `publish_model_gen_anchors_after_generation` 现语义，增量路径仍按已提交 data anchor 的实际结束 sesno 发布）。
- **FR-010**: `model_unit_commit` 台账 MUST 迁至模型同库 Surreal 表，字段语义对齐 ADR-0005；空表起步、MUST NOT 内置存量迁移；救济脚本 MUST 外置于 `scripts/`、不进主二进制、不引入 duckdb 依赖。
- **FR-011**: spec-026 欠账追赶闭环行为 MUST 不受影响（模型水位定义、锚点语义、五桶欠账消费均原样）。

## Success Criteria

- **SC-001**: 新站剧本一条冒烟可断言：全量解析（查无 full 锚点/无 DuckLake 产物）→ 全量生成（`model_gen` 锚点 = 基线）→ 首次增量（watermark 从基线衔接、闭环不误报）。
- **SC-002**: 带行为键的 toml 启动退出码非 0 且 stderr 含修复指引；仅惰性键的 toml 启动成功且有警告行。
- **SC-003**: 三组构建全绿：默认 build、`--features full`、`scripts/build-sync-cli.ps1`；`cargo tree` 无 duckdb 残留。
- **SC-004**: watch 数据提交进行中触发 HTTP 增量生成，产物读取一致（同一 `VERSION AT` 切面），无撕裂告警。
- **SC-005**: `scripts/smoke/unified_versioning_e2e.ps1`、`anchor_continuity_audit.ps1` 回归通过（未改语义部分零波动）。
- **SC-006**: unit-export 后新 Surreal 台账可查询到 `(dbnum, unit_refno, sesno)` 入账记录。

## Non-goals

- 增量数据链本体语义修改（lease / fingerprint / 门禁 / recover-pending——原样保留，另见 `docs/plans/2026-07-20-incremental-update-hardening-dev-plan.md`）。
- brooks-review 伴生加固（锁进 `run_increment` seam、活动 db 文件解析收敛等）——独立轨道，见 `docs/plans/2026-07-22-parse-zero-version-refactor-plan.md` M6，可与本 spec 并行但不进本 spec 验收门。
- 模型 diff API、mesh GC（沿 spec-026 non-goals）。
