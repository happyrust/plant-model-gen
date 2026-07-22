# Implementation Plan

> 2026-07-23 第二轮 grill-with-docs 修订。架构决策以 ADR-0007、ADR-0008、ADR-0010 为准；本文件给出实施顺序，验收口径见 `spec.md`。

## Invariants

1. 初始化解析只建立初始化时的稳定最新态，不建立数据历史。
2. 初始化全量生成可以发布 `model_gen` 基线；业务数据历史由首次增量的 `incremental_baseline` 开始。
3. legacy `full` 只读兼容，应用层永远不能再写新的 full 数据锚点。
4. 数据提交成功后同流程幂等写模型欠账；欠账失败不回滚数据但形成覆盖洞。多库模型生成只消费整轮数据和欠账均成功后的单一 MVCC 切面。
5. 初始化、增量、追赶和修复的深层 seam 自持项目 mutation lock。

## Phase 0 — 基线收敛与回滚保障

- 以 `docs/plans/2026-07-22-ducklake-retirement-deletion-manifest.md` 重新核对当前工作区，而不是照抄旧任务状态。当前已观察到：DuckLake adapters/Cargo 依赖大部已删，但 `version_store/replica.rs`、`generation_replica_*`、CLI DuckLake unit ledger 引用、打包脚本原生扩展和 full 锚点写路径仍在。
- 当前目录未检测到 git 仓库；任何源码删除或跨文件实现前先建立可恢复基线。不得把真实项目数据、模型产物或 initialization staging 纳入源码快照。
- 将删除清单逐项标成 done / remaining / keep，并记录对应任务；编译器暴露的悬挂引用只能补到清单，不临时保留旧架构分支。

## Phase 1 — InitializationWorkflow

- 建立单一初始化编排入口，内部顺序为：申请项目 mutation lock → 创建唯一 staging 数据目录 → 以 `versioned_storage=true`、retention=`0` 建库 → 冻结 DB 文件清单和逐文件 SHA-256 → 解析最新态 → 数据完整性校验 → 初始化全量模型生成 → 发布 `model_gen` 基线 → 最终复核源指纹 → 停止 staging runtime → 原子更新站点目录指针/注册信息。
- 在解析、完整性校验、模型生成前以及正式启用前复核源文件集合/路径/内容；任一变化都使本轮失败。失败 staging 标为废弃并保留诊断，重试必须创建新目录，清理由显式运维动作完成。
- 新 managed 站点和新配置生成入口显式持久化 versioned/retention 值；旧配置缺键时不改变其既有解释，避免把旧非 versioned 目录静默切换。
- 初始化入口先检查 Ready 标记和业务锚点；存在 legacy full、incremental_baseline、incremental 或 model_gen 任一种锚点即拒绝原地执行。
- 会话元数据导入拆成独立 CLI/任务及 service API，只更新 sesno 时间/描述事实；移除普通解析中的 `sync_history`/`sync_versioned` 行为分支，true 配置硬错误并指向新入口。

## Phase 2 — 数据锚点与首次起版

- 拆分“可读 anchor source”与“可写 commit source”：读侧接受 `full | incremental_baseline | incremental | model_gen`；数据写 API 只暴露 baseline/incremental，模型写 API 只暴露 model_gen。删除 `VersionCommitSource::Full` 和 `write_full_version_anchors`。
- 保留全量写 worker 的 drop/join 与 `pe_owner_version_meta(full_reload)`，必要时从 full 锚点函数中拆为非版本化完整性收尾。
- `committed_watermark` 只从 full/baseline/incremental 数据锚点取最高已提交 sesno；没有数据锚点才回退 `dbnum_info_table`，不得把初始化 `model_gen` 当成数据水位。
- 在现有 lease/continuity/pending seam 内加入 baseline handshake：无任何数据锚点的已有 dbnum，在第一条业务变更 SQL 前幂等建立 `incremental_baseline`；已有 legacy full/incremental 历史则沿既有水位续跑，不回填 baseline。
- 新 dbnum onboarding 在项目锁内导入完整当前态并校验，再发布该库 baseline；普通初始化入口不能旁路添加 dbnum。活动 dbnum 集合缩减时拒绝原地删除并指向新目录初始化。

## Phase 3 — IncrementRun 提交与生成 barrier

- 将 `ProjectMutationLock` 下沉到 `run_increment`、初始化、catch-up 和 repair；组合调用通过模块私有、不可伪造的 held-lock token 复用锁，删除公开布尔跳锁。
- 每个 dbnum 沿既有语义先完成 PE/ATT 与 incremental 锚点提交，再以 commit fingerprint 幂等写 `model_gen_debt`。欠账失败不回滚已提交数据，必须持久化/报告覆盖洞并禁止本轮生成。
- 数据阶段收集本轮全部 dbnum 的提交与欠账结果后再进入 generation barrier。任一数据失败、Commit Pending 或欠账失败时，保留其它已成功数据及欠账，但整轮 generation 标为 skipped，不调用模型写入或 model_gen 发布。
- 全部数据提交和欠账写入成功后，从数据库读取本轮最后一个数据锚点时间，构造不可变 `GenerationReadSpec { read_at, observed_watermarks }`；本轮所有 dbnum、所有模型查询共享该 `read_at`。
- 模型中性 debt 不运行几何生成，在同一 barrier 发布带原因的 no-op `model_gen`；非中性 debt 只有生成、后处理和导出全部成功后才发布。
- source fingerprint gate 继续覆盖收集、数据提交/欠账写入和模型阶段；运行 summary 记录 baseline 结果、每库提交与欠账状态、水位、统一 read_at、skip/no-op/generated 与欠账原因。

## Phase 4 — 主表读取与受控修复

- `generation_read::surreal` 通过既有领域查询 trait 批量读取主 PE/ATT、owner、reference 和 transform；初始化全量生成传 live read，增量/catch-up/repair 强制传精确 `read_at`，SQL 和 `VERSION AT` 只出现在 adapter 内。
- 删除 `version_store/replica.rs`、五张 `generation_replica_*` schema、snapshot binding、复制链和 manifest 覆盖校验；输入版本清单只保留 summary 观测字段，并清理 authoritative/replica 命名。
- 正式 Ready versioned 站点的普通 generate/regen 在 service seam 硬拒绝。受控 repair 必须解析既有 data anchor、持项目锁、使用其数据库时刻读取，并在成功后刷新同 sesno 的 model_gen 结果。
- 新建 append-only `model_generation_run` 事件台账：一次尝试先追加 started，结束追加 succeeded/failed；相同 run id 聚合为一次运行，重启后只有 started 的运行视为 abandoned。初始化、增量、no-op、catch-up、repair 使用同一 repository。

## Phase 5 — DuckLake 退役与 unit ledger

- 在主表 adapter 可用后删除 replica；在新 unit ledger 可用后删除 CLI/web 的 `DuckLakeAuthority` 调用，最终删除 `version_store/`。删除顺序必须保持每个切片可构建。
- 模型同库建立 Surreal `model_unit_commit` 表，保持 `(dbnum, unit_refno, sesno)`、impact 与 artifact pointer/hash 语义；unit-export、rollback 和 model runtime 统一依赖新 repository。
- 外置 `scripts/migrate_unit_ledger_from_ducklake.ps1` 通过独立 duckdb CLI 导出/导入旧台账；主二进制不重新引入 duckdb，旧 `.ducklake` 不自动迁移或删除。
- 清理 `bootstrap-generation-read`、perf gate、retired features/options 和 package 中 `runtime/ducklake` 的原生 DuckLake/SQLite 扩展。前端 parquet 查看器所需 duckdb-wasm 离线资产明确保留。
- 保留 `get_db_option_ext` 的 retired key 分级检测：显式 ducklake/compare 行为值硬错误，惰性路径键 warning。

## Phase 6 — 管理端、迁移与运维

- managed site 状态机增加 `Initializing → Validating → GeneratingBaseline → Ready`，失败状态保存 staging 路径、指纹和诊断；只有 Ready 可启动 watch 和对外模型服务。
- 创建 UI/API 明示默认 versioned + 无限 retention 的磁盘风险，创建前报告目标盘剩余空间和估算；不自动改成有限窗口。
- 非 versioned 存量站点继续当前态运行；启用版本能力时走新 staging 初始化。legacy full 站点直接续跑既有 continuity，但所有新运行都审计 full 行数不增长。
- 更新 specs/022/023/024/025、quickstart、ops-notes、AGENTS、CHANGELOG 和 smoke 脚本，消除“初始化解析应产生 full 锚点”以及“任意整库 regen 可修复”的旧口径。

## Verification Gates

1. 静态/构建：格式化；默认 build、`--features full`、`scripts/build-sync-cli.ps1`；`cargo tree -i duckdb` 无原生依赖。遵守仓库规则，不运行或编译 cargo test。
2. CLI/JSON：初始化 fingerprint 变化失败且不切换；首次增量双锚点与 100→105 diff；legacy full 续跑且不新增 full；onboarding；direct regen 拒绝；repair run events。
3. 故障注入：debt 写失败时数据锚点保留、覆盖洞可见且整轮无 model_gen；多库第二库提交失败同样整轮禁生成；恢复后受控 catch-up/repair 使用一个 `VERSION AT`。
4. web_server：启动真实服务后以 HTTP POST 验证新站状态机、默认配置、初始化提升、增量、会话元数据任务和 repair 权限。
5. 回归：`unified_versioning_e2e.ps1`、`anchor_continuity_audit.ps1`、`pe_owner_incr_shapes_smoke.ps1`；新增 `initialization_incremental_boundary_e2e.ps1` 覆盖 ADR-0010。

## Ordering and Dependencies

- Phase 1 与 Phase 2 可分别实现 staging 编排和 anchor 类型，但 Phase 3 barrier 完成前不得对外启用新流程。
- Phase 3 的 lock、提交/欠账结果 barrier 和 read spec 先于 Phase 4 主表 adapter 收尾，否则无法证明 `VERSION AT` 来源。
- Phase 4 adapter 先于 replica 删除；Phase 5 Surreal unit ledger 先于 DuckLake CLI/runtime 删除。
- spec-026 的 debt/catch-up 表结构、五桶语义和“欠账失败不回滚数据”保持不变；若并行改造同一生成入口，以 ADR-0010 的多库整轮生成 barrier 为新增约束。
