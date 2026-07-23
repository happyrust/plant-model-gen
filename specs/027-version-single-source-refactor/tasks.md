# Tasks

> 2026-07-23 依据 ADR-0010 重排。`[x]` 只表示任务产物已落地；跨阶段验收仍由 P7 统一关闭。T002 已在分支 `agent/spec026-027-debt-catchup` 以基线提交 `955c7ff` 完成。遵守 AGENTS.md：不运行或编译 `cargo test`。

## P0 — 决策与安全基线

- [x] **T001 DuckLake 退役删除清单**：已有 `docs/plans/2026-07-22-ducklake-retirement-deletion-manifest.md`，覆盖 adapters、Cargo、CLI、package、tests 和运行遗物。
- [x] **T002 建立可恢复基线**：用户选择 Git 方案；分支 `agent/spec026-027-debt-catchup` 的 `955c7ff` 固化实现前工作区，可用 `git show 955c7ff:<path>` 抽样恢复。
- [x] **T003 冻结第二轮架构决策**：根 `CONTEXT.md`、ADR-0010、ADR-0007/0008 引用、`spec.md` 与 `plan.md` 已统一初始化/增量边界。
- [x] **T004 重新盘点实际残留**：更新 T001 清单的状态栏，至少核对 `write_full_version_anchors`、`VersionCommitSource::Full`、`version_store/replica.rs`、`generation_replica_*`、CLI `DuckLakeAuthority`、package 原生扩展和 history 配置分支。验收：每项有符号、处置、依赖任务和 keep/delete 理由。

## P1 — 初始化与会话元数据（依赖 T002、T004）

- [ ] **T005 新站版本化默认值**：managed site/API/模板创建的新站显式持久化 `versioned_storage=true` 与 retention `0`；旧配置缺键不改变既有解释。验收：创建 API、落盘 TOML 和实际 storage capability 一致。
- [ ] **T006 InitializationWorkflow 与深层锁**：建立唯一初始化 service，入口内部持 `ProjectMutationLock`，只接受新目标；staging 路径包含 run id，重试不复用。验收：两个并发初始化只有一个进入写阶段。
- [ ] **T007 稳定源文件门禁**：冻结 dbnum→规范路径/size/mtime/SHA-256 manifest，在解析、校验、模型生成前及启用前复核；集合、路径或内容变化均 fail closed。验收：中途替换同名同大小文件仍能检测。
- [ ] **T008 初始化验证、模型基线与原子启用**：staging 内完成解析、计数/owner/reference 完整性、全量生成和 `model_gen` 基线后，停止 runtime 并原子切换站点指针；失败不触碰 Ready 站点。验收：逐点故障注入均保持原指针。
- [ ] **T009 初始化重入守卫**：Ready 标记或任一业务锚点存在时拒绝原地初始化，返回新目录切换/onboarding 指引。验收：只有 legacy full 或只有 model_gen 的目录同样拒绝。
- [ ] **T010 独立会话元数据导入**：新增 CLI/service 只导入 sesno、发生时间和描述；不调用 PE/ATT 持久化、模型生成或 anchor 发布。验收：前后 PE/ATT checksum、anchor 数和 model watermark 不变。
- [ ] **T011 退役 history 解析配置**：移除 `sync_pdms` 的 `is_sync_history()`/`sync_versioned` 行为；true 硬错误并指向 T010，false 在兼容期 warning。验收：旧开关不能进入普通解析路径。

## P2 — 数据版本从增量起版（依赖 T002、T004）

- [ ] **T012 拆分 anchor 读写 source 类型**：读侧兼容 `full | incremental_baseline | incremental | model_gen`；数据写 API 只能构造 baseline/incremental，模型写 API 只能构造 model_gen。验收：应用代码无法类型化调用 full writer。
- [ ] **T013 删除 full 锚点写链**：删除 `write_full_version_anchors`、pending full 聚合和解析期调用；将 `pe_owner_version_meta(full_reload)` 拆到非版本化完整性收尾。验收：初始化成功且 full anchor 行数不增长。
- [ ] **T014 baseline schema 与幂等 API**：为 `incremental_baseline` 定义 schema/唯一键、确定性 fingerprint、source evidence、查询/审计支持；相同请求幂等，不同内容冲突。验收：重复首次增量不产生第二条 baseline。
- [ ] **T015 首次增量 pre-apply handshake**：无数据锚点的 dbnum 在 lease/continuity/pending 检查后、第一条业务变更 SQL 前创建 baseline；sesno 等于当时 `dbnum_info` 水位。验收：100→105 可分别 `VERSION AT` 查询前后状态。
- [ ] **T016 committed_watermark 与 legacy continuity**：full/baseline/incremental 参与数据水位，model_gen 排除，无数据锚点才回退 dbnum_info；legacy full/incremental 沿既有水位续跑且不回填 baseline。验收：baseline-only、full-only、incremental 和 model_gen-only 四类 fixture。
- [ ] **T017 新 dbnum onboarding**：已有项目内通过受锁增量流程导入新库完整当前态，校验后建立该库 baseline，再参加普通增量。验收：普通初始化入口无法旁路添加 dbnum。
- [ ] **T018 整库移除策略**：检测活动 dbnum 集合缩减并拒绝原地删除，返回新 staging 初始化/切换步骤。验收：旧目录数据与 anchors 不被删除。

## P3 — 多库增量与生成 barrier（依赖 T012–T016、spec-026 debt repository）

- [ ] **T019 mutation lock 下沉**：`run_increment`、初始化、catch-up、repair 自持同一锁；用模块私有 held-lock token 支持组合调用，删除公开 skip-lock bool。验收：CLI/watch/HTTP 交叉调用不能并行写。
- [ ] **T020 欠账写入与覆盖洞门禁**：每库数据提交后以 commit fingerprint 幂等写 `model_gen_debt`；失败不回滚数据，记录覆盖洞并阻断本轮生成。验收：故障注入后 incremental anchor 保留、debt 缺失可观测、无 model_gen。
- [ ] **T021 多库 generation barrier**：先收集本轮全部 dbnum 的数据提交与欠账结果；任一数据失败、Commit Pending 或欠账失败时 generation skipped，已成功数据及欠账保留。验收：第二库失败后无模型文件改写、无成功 run terminal、无 model_gen。
- [ ] **T022 统一读取时刻**：全部数据提交和欠账写入成功后读取本轮最后一个 data anchor 时间，形成不可变 `GenerationReadSpec`；所有 dbnum/session 共用。验收：SQL trace 中同轮 `VERSION AT` 字面值完全一致。
- [ ] **T023 增量运行 summary**：输出 dbnum 数据/欠账结果、baseline 状态、统一 read_at、observed watermarks、generation skipped/no-op/generated 和覆盖洞原因。验收：失败恢复无需从日志猜测已提交边界。
- [ ] **T024 模型中性 no-op 提交**：确认无几何/层级/transform/材质效果时不跑几何，但发布绑定同一数据水位的 no-op `model_gen`。验收：model watermark 前进，模型文件 checksum 不变。
- [ ] **T025 失败恢复与 catch-up**：欠账完整时，后续受锁 catch-up 使用原 data anchor/read_at 消费保留欠账；存在覆盖洞时只允许受控 repair，不重放数据提交。验收：两种恢复路径均不重新推进 data watermark。

## P4 — 主表读取与模型运行审计（依赖 T019–T022）

- [ ] **T026 主表直读 adapter**：`generation_read::surreal` 通过既有 trait 批量读取主 PE/ATT、owner、reference、transform；初始化 staging 活读，增量/repair 强制 `VERSION AT`。验收：同一 fixture 与旧 replica 输出语义一致。
- [ ] **T027 删除 generation replica**：删除 `version_store/replica.rs`、五张 `generation_replica_*` schema、binding/authoritative snapshot、复制与覆盖校验；manifest 仅保留观测字段。验收：非历史源码无 `generation_replica`。`version_store` 与五表已删；manifest 的 `authoritative_snapshot_id` 观测字段改名仍待完成。
- [ ] **T028 append-only `model_generation_run`**：定义 run event schema/repository，覆盖 initialization、incremental、no-op、catch-up、repair 的 started/terminal；记录 actor/reason/input/read_at/contract/error/旧新 anchor。验收：历史 event 不可 update/delete，started-only 可识别为 abandoned。
- [ ] **T029 正式站点生成权限**：普通 generate/regen 对 Ready versioned 站点硬拒绝；只允许 initialization staging 或绑定现存 data anchor 的 repair/catch-up。验收：HTTP、CLI、内部调用三层都不能绕过 service guard。
- [ ] **T030 受控 repair**：解析目标 data/model anchor，持锁，以原 read_at 重做后处理并刷新同 sesno model_gen；sesno 身份不变。验收：两次 repair 产生两个 run id 和各自事件，但模型提交身份不增加。

## P5 — DuckLake 退役与 unit ledger（依赖 T026、T028）

- [x] **T031 retired 配置分级检测**：`get_db_option_ext` raw TOML 已对 ducklake/compare 行为值硬错误、惰性键 warning；仍需纳入 T038 手测。
- [x] **T032 Surreal `model_unit_commit` repository**：定义 `(dbnum, unit_refno, sesno)` 唯一提交、impact、artifact pointer/hash 与 latest/list API。验收：幂等写、冲突检测和有序查询。
- [x] **T033 改造 unit-export/rollback/runtime 调用方**：移除 CLI/web 对 `DuckLakeAuthority` 的引用，统一依赖 T032；rollback 继续只允许从 latest 派生。验收：unit-export 后 Surreal 可查且 runtime 可消费。
- [ ] **T034 可选旧台账救济工具**：外置 PowerShell 调 duckdb CLI 导 JSON，再调用新 CLI dry-run/import；主二进制不重新引入 duckdb。验收：dry-run 数量正确、重复导入幂等、坏 hash 拒绝。
- [x] **T035 删除原生 DuckLake 残留**：依 T004 清单删 adapters/CLI/features/perf gate/原生扩展参数和复制；保留前端 viewer 的 duckdb-wasm/parquet 离线资产并在清单说明。验收：`cargo tree -i duckdb` 空，源码命中仅错误/迁移/历史说明。

## P6 — 管理端与文档

- [ ] **T036 managed site 初始化状态机**：实现 Initializing/Validating/GeneratingBaseline/Ready/Failed，展示 staging、fingerprint、错误、磁盘估算；只有 Ready 开 watch/服务。验收：服务重启可恢复状态，不自动启用半成品。
- [ ] **T037 文档与旧 spec 收尾**：更新 specs/022/023/024/025、quickstart、ops-notes、AGENTS、CHANGELOG；明确 legacy full、数据/模型初始非对称、无限 retention 风险、onboarding/移除/repair 操作。

## P7 — 验证门

- [ ] **T038 构建与配置门**：format；默认 build、`--features full`、`scripts/build-sync-cli.ps1`；默认配置无 warning，retired 行为值退出非 0，惰性键启动成功有 warning。
- [ ] **T039 初始化边界 E2E**：新增 `initialization_incremental_boundary_e2e.ps1`，覆盖 staging、fingerprint 失败、不切换、model_gen baseline、首次双数据锚点、legacy full 续跑、onboarding 和移除拒绝。
- [ ] **T040 多库/debt/repair 故障注入**：覆盖 debt 失败保留数据并形成洞、跨入口锁、第二库失败整轮跳过、统一 VERSION AT、no-op、catch-up、直接 regen 拒绝和 repair run events。
- [ ] **T041 既有冒烟回归**：运行 `unified_versioning_e2e.ps1`、`anchor_continuity_audit.ps1`、`pe_owner_incr_shapes_smoke.ps1` 和真实 web_server HTTP POST；保存命令、退出码与关键 JSON 证据。
- [ ] **T042 最终删除审计**：关闭 T004 manifest 全部项目；`full` 新写、replica、DuckLake native、history parse 均无可达路径；SC-001～SC-011 逐项链接验证证据。
