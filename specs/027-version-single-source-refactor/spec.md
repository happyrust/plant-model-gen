# Feature Specification: 版本单源收敛与解析零版本重构

**Created**: 2026-07-22
**Status**: Accepted for implementation
**Revised**: 2026-07-23（第二轮 grill-with-docs）
**Upstream**: `docs/adr/0007-version-single-source-surreal-mvcc.md`（supersede ADR-0002）、`docs/adr/0008-generation-primary-read-version-at.md`（supersede ADR-0003）、`docs/adr/0010-initialization-and-incremental-version-boundary.md`
**术语**: 初始化解析 / 会话元数据导入 / 增量起版基线 / 数据库增量接入 / 生成读取时刻 / 模型生成运行，见根 `CONTEXT.md`；定案过程见 `docs/plans/2026-07-22-parse-zero-version-refactor-plan.md`

## User Scenarios

### US1 - 初始化时读取稳定的最新态

新站点在唯一的 staging 数据目录执行初始化解析：读取初始化当时的一轮稳定源文件最新态，前后内容指纹一致才算成功；不回放历史会话、不写数据锚点。新站默认以 `versioned=true`、retention=`0` 建库，但只有锚点才构成业务可见版本。

### US2 - 完整初始化后一次启用

初始化解析、数据完整性校验、首次全量模型生成与 `model_gen` 起点锚点全部成功后，staging 目录才切为正式站点；任一步失败都不启用该目录。首次增量前允许模型基线可查询而数据历史为空。

### US3 - 首次增量建立数据前态与新版本

初始化后尚无数据锚点的 dbnum，首次增量在应用变更前，以 `dbnum_info` 水位写一次 `source='incremental_baseline'` 的完整前态锚点，再提交普通 `incremental` 锚点，因此第一次变化也能做前后 diff。已有 legacy full/incremental 历史的 dbnum 直接沿既有水位续跑，不回填 baseline。此后增量仍受 lease、Commit Pending、指纹幂等和 ContinuityGap 门禁保护。

### US4 - 会话元数据独立导入

需要设计会话号、时间等展示信息时，操作者显式运行独立的会话元数据导入命令/任务；该任务不进入初始化解析控制流，不写数据或模型版本。

### US5 - 已有历史目录拒绝原地初始化

目录存在任一业务锚点时，初始化解析立即失败并提示新目录切换。新增 dbnum 走数据库增量接入：在项目锁内导入该库当前完整态、校验并建立 `incremental_baseline`；移除整个 dbnum 不伪造 sesno，必须新目录重新初始化。

### US6 - 多库增量使用一致模型切面

同一轮增量任一 dbnum 数据提交失败时，成功数据提交保留，但整轮不生成模型。数据已提交而对应欠账写入失败时沿 ADR-0006 形成覆盖洞，也跳过整轮模型生成。全部数据提交和欠账写入成功后，模型生成以本轮最后一个数据锚点的数据库时间作为统一 `VERSION AT`，输入清单另记录各 dbnum 水位。

### US7 - 模型中性变更与受控修复

只改非模型影响属性时不重算几何，但发布 no-op `model_gen` 锚点推进覆盖水位。正式 versioned 站点拒绝任意 generate/regen；受控 repair/catch-up 必须绑定既有数据锚点，并追加不可变 `model_generation_run` 审计记录。

### US8 - 存量锚点与 DuckLake 配置升级

存量 `source='full'` 数据锚点继续只读可查，所有新写被禁止。配置了 `parse_storage_backend=ducklake` 或 `generation_read_backend=ducklake|compare` 的站点启动即硬错误；只残留 `ducklake_*`/`duckdb_*` 路径键时仅警告；磁盘 `.ducklake` 目录原样保留。

### US9 - 交付台账连续性

升级后 unit-export 照常写入模型同库的 Surreal `model_unit_commit` 表（空表起步）；确需旧 DuckLake 台账的站点用外置脚本手工导入，主二进制不含 duckdb 依赖。

## Functional Requirements

- **FR-001**: 初始化解析 MUST 读取开始时冻结的 DB 文件清单，保存逐文件 SHA-256，结束时复核；文件集合或内容变化 MUST 使本轮失败。它 MUST NOT 固化数据版本锚点或运行历史重放链；MUST 仍更新 `dbnum_info`/`db_meta` 的 `latest_sesno`。
- **FR-002**: `committed_watermark` MUST 保持「已提交数据锚点优先、无数据锚点时回退 `dbnum_info_table`」；数据锚点来源包括只读 legacy `full`、`incremental_baseline` 与 `incremental`，`model_gen` 不参与数据水位。新写只允许 baseline/incremental，legacy 锚点仍参与存量 continuity。增量提交的 lease / Commit Pending / 指纹幂等 / ContinuityGap 门禁语义 MUST 不变。
- **FR-003**: DuckLake 权威链实现 MUST 全部删除：`version_store/`（`model_unit_commit` 走 FR-010）、`generation_read` 的 ducklake/compare 适配器与 factory 分支、`bootstrap-generation-read` CLI、`generation-read-ducklake` feature 与 `dep:duckdb`、perf gate 脚本与 fixture、打包脚本中 `runtime/ducklake` 的原生 DuckLake/SQLite 扩展、`committed_watermark`/`publish_authority_after_apply` 的 feature fork。前端离线 parquet 查看器使用的 duckdb-wasm 资产不在删除范围。
- **FR-004**: 启动配置检测 MUST 分级：行为键 `parse_storage_backend=ducklake`、`generation_read_backend=ducklake|compare` 残留 → 硬错误并输出修复指引；惰性键（`ducklake_*`、`duckdb_*`）残留 → 仅警告。检测点位于 `get_db_option_ext` 校验链。
- **FR-005**: 磁盘 `.ducklake` 目录与既有导出物 MUST NOT 被自动迁移、修改或删除。
- **FR-006**: 模型生成 MUST 经领域查询 trait 直读主 PE/ATT 表；生成算法代码 MUST NOT 包含 SQL、Surreal record-id 或全局数据库调用（沿 ADR-0003 保留的边界）。
- **FR-007**: 同轮全部 dbnum 数据提交与对应欠账写入成功后，增量生成 MUST 以本轮最后一个成功数据锚点的数据库时间为唯一 as-of 时刻，运行内全部数据读取 MUST 加同一个 `VERSION AT`；初始化全量生成 MUST 活读 staging 当前态且不绑定时刻。
- **FR-008**: `generation_replica_element/hierarchy/reference/transform/db_catalog` 五表、snapshot 绑定与 manifest 双向校验 MUST 删除；输入版本清单 MUST 降级为观测记录：写入运行 summary，MUST NOT 参与失败关闭或覆盖校验。
- **FR-009**: 初始化全量生成成功收尾 MUST 发布可查询的 `model_gen` 起点锚点（sesno = `dbnum_info_table` 当前值）；生成或后处理失败 MUST NOT 发布。正式 versioned 站点的普通 generate/regen MUST 被拒绝，后续模型重做只允许 FR-019 的受控 repair/catch-up。
- **FR-010**: `model_unit_commit` 台账 MUST 迁至模型同库 Surreal 表，字段语义对齐 ADR-0005；空表起步、MUST NOT 内置存量迁移；救济脚本 MUST 外置于 `scripts/`、不进主二进制、不引入 duckdb 依赖。
- **FR-011**: spec-026 欠账追赶闭环的模型水位、五桶欠账、欠账失败不回滚数据而形成覆盖洞的语义 MUST 保留；本 spec 只增加首次起版、多库失败门禁与 no-op 覆盖推进。
- **FR-012**: 所有新建 managed 站点及新配置生成入口 MUST 显式写 `versioned_storage=true`、`version_retention="0"`；现有配置和现有数据目录 MUST NOT 因 serde 默认值变化被自动切换。非 versioned 存量站点启用版本能力必须新目录初始化。
- **FR-013**: 每次初始化 MUST 使用唯一 staging 数据目录。只有源指纹复核、数据完整性校验、初始化全量模型生成和 `model_gen` 起点锚点全部成功后，站点注册/配置才可原子切向该目录；失败目录不得原地覆盖重试。
- **FR-014**: 存在任一业务锚点的目录 MUST 拒绝初始化解析。新增 dbnum MUST 走数据库增量接入并只写该新库；移除整个 dbnum MUST 提示新目录重新初始化，不得生成合成 sesno 或未锚定删除。
- **FR-015**: 会话元数据导入 MUST 是独立 CLI/任务阶段，可在初始化之外重跑；它只维护会话号、时间等辅助事实，不得写 `sesno_version_anchor`、模型提交或模型欠账。遗留 `sync_history=true` MUST 报错并指向新入口，`false` 可在兼容期忽略并告警；`sync_versioned` 同步退役。
- **FR-016**: 初始化后尚无数据锚点的已有 dbnum，首次增量 MUST 在任何 PE/ATT 变更前、同一项目锁和 dbnum lease 内幂等创建 `source='incremental_baseline'` 锚点；对新 dbnum 接入则在完整当前态导入并校验后创建该锚点。已有 legacy full/incremental 数据锚点的 dbnum MUST 沿既有水位续跑且不得回填 baseline。请求 sesno 早于基线时历史查询 MUST 明确无历史。
- **FR-017**: 初始化、`run_increment`、欠账追赶和受控 repair 的深层入口 MUST 自持同一项目 mutation lock；调用方只能传递不可伪造的“已持锁”令牌，不能以布尔参数跳过。
- **FR-018**: 同轮任一 dbnum 数据提交失败、进入 Commit Pending，或数据已提交但对应欠账写入失败时，MUST 跳过整轮模型生成且不得发布任何新 `model_gen`；已成功的数据提交及可用欠账保留，缺失欠账按覆盖洞处理，待受控恢复后统一追赶。
- **FR-019**: 模型中性增量 MUST 发布带 no-op 原因的 `model_gen` 锚点。受控 repair/catch-up MUST 绑定既有数据锚点；同 sesno 最新成功结果仍属于原模型提交，但每次尝试 MUST 以同一 run id 向 append-only `model_generation_run` 台账追加 started 与 terminal 事件，记录输入水位、统一读取时刻、生成契约哈希、原因、操作者、旧/新锚点时间和结果；不得原地覆写历史运行记录。
- **FR-020**: 实现 MUST 删除 `VersionCommitSource::Full` 与 `write_full_version_anchors`；schema/读模型 MAY 保留 `full` 以兼容旧记录，但任何应用写入口 MUST 无法构造新的 full 数据锚点。

## Success Criteria

- **SC-001**: 新站剧本一条冒烟可断言：默认 versioned staging → 稳定初始化解析（无数据锚点）→ 全量生成（可查询 `model_gen` 基线）→ 原子启用 → 首次增量产生 `incremental_baseline` 与 `incremental` 两个数据锚点并可做前后 diff。
- **SC-002**: 带行为键的 toml 启动退出码非 0 且 stderr 含修复指引；仅惰性键的 toml 启动成功且有警告行。
- **SC-003**: 三组构建全绿：默认 build、`--features full`、`scripts/build-sync-cli.ps1`；`cargo tree` 无 duckdb 残留。
- **SC-004**: 两个 dbnum 同轮提交时，所有生成查询记录同一 `VERSION AT`；注入任一 dbnum 的数据提交或欠账写入失败后，本轮 `model_generation_run` 无成功 terminal 事件且无 `model_gen` 发布，恢复后受控 catch-up/repair 才推进。
- **SC-005**: `scripts/smoke/unified_versioning_e2e.ps1`、`anchor_continuity_audit.ps1` 回归通过（未改语义部分零波动）。
- **SC-006**: unit-export 后新 Surreal 台账可查询到 `(dbnum, unit_refno, sesno)` 入账记录。
- **SC-007**: 初始化过程中修改任一源文件，任务以明确的 fingerprint mismatch 失败，正式站点目录指针不变；重试使用全新 staging 目录。
- **SC-008**: 对已有锚点目录调用初始化解析返回明确拒绝；新 dbnum 通过 onboarding 成功建立 `incremental_baseline`，整库移除请求返回“新目录初始化”指引。
- **SC-009**: `sync_history=true` 配置不能再进入普通解析分支；独立会话元数据导入成功后 PE/ATT checksum、锚点数量与模型水位均不变。
- **SC-010**: 模型中性增量不执行几何生成但推进 no-op `model_gen`；正式站点直接 regen 被拒，受控 repair 成功后 sesno 身份不变且同一 run id 有 started/terminal 审计事件。
- **SC-011**: 存量 `full` 锚点仍可查询且可作为 continuity 起点；初始化/增量/repair 完成后全库没有新增 `source='full'` 记录。

## Non-goals

- 增量数据链本体语义修改（lease / fingerprint / 门禁 / recover-pending——原样保留，另见 `docs/plans/2026-07-20-incremental-update-hardening-dev-plan.md`）。
- 项目成员关系版本轴；整库 dbnum 移除通过新目录初始化解决，本 spec 不引入 synthetic project revision。
- 自动修改存量站点的 `versioned_storage`/retention，或自动删除失败 staging、旧数据目录、`.ducklake` 遗物。
- 活动 db 文件发现与 db_index 收敛等其它 brooks-review 事项；项目锁下沉已因 ADR-0010 升为本 spec 核心要求。
- 模型 diff API、mesh GC（沿 spec-026 non-goals）。
