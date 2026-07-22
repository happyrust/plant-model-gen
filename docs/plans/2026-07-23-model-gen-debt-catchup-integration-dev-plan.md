# 开发计划：增量模型欠账闭环与统一版本边界集成

> 日期：2026-07-23  
> 状态：Draft for Plannotator review  
> 权威依据：`specs/026-incremental-model-gen-debt-catchup/`、`specs/027-version-single-source-refactor/`、ADR-0006、ADR-0010  
> 实施原则：保留 spec-026 的五桶欠账与“数据提交不因模型失败回滚”；冲突处以 spec-027 / ADR-0010 的深层锁、多库整轮 barrier、统一读取时刻和受控 repair 为准。

## 1. 目标

把当前“能够写欠账、逐库尝试追赶”的实现收敛为可上线闭环：

1. 同轮所有 dbnum 的数据提交与欠账写入完成后再决定是否生成模型。
2. 任一数据提交、Commit Pending 或欠账写入失败时，整轮不改模型、不发 `model_gen`。
3. 同轮模型生成统一绑定最后一个成功数据锚点的数据库时刻。
4. 连续欠账、模型中性欠账、delete-only 欠账和历史覆盖洞都有明确且可恢复的行为。
5. 追赶与 repair 由深层 seam 自持项目锁，并写 append-only 运行台账。
6. 用 CLI+JSON、真实 Web HTTP 和 smoke 脚本验收；不新增、不运行、不编译 `cargo test`。

## 2. 当前事实与阻断项

已落地：

- `model_gen_debt` schema、五桶持久化和幂等冲突检测。
- 数据水位、模型水位与连续欠账前缀分析。
- `incremental-sesno`、watch、Web 复用 `run_increment`。
- 默认生成、`--no-generate-model`、`--no-model-impact-filter`。
- 属性三级判定与 `model_neutral_changes`。
- 连续追赶成功后的 `model_gen` + debt consumed 事务。
- `cargo check --lib/--bin aios-database --features "gen_model,sqlite-index,web_server"` 已通过。

必须先解决：

1. **delete-only cleanup 失败**：删除 PE 已不在当前 hierarchy，`pre_cleanup_for_regen_versioned` 把删除 refno 当当前 root 时返回 `MissingRequiredData`。
2. **full fallback 不是真正 regen**：`--allow-full-regen` 只切 Full scope，没有整库模型清理、强制 mesh/boolean 或 repair 审计。
3. **多库撕裂**：某库提交失败后，当前代码仍会为其它成功库逐库生成和发锚点。
4. **读取时刻不统一**：每库各开 generation session；没有显式 `GenerationReadSpec/read_at`。
5. **锁层级错误**：锁在部分 CLI/watch adapter，`run_increment`、catch-up 和 Web 深层入口自身不持锁。
6. **观测不足**：遇到第一个 debt gap 后隐藏后续欠账；dry-run 无显式五桶数量；`--dbnum` 不能省略。
7. **运行不可审计**：没有 `model_generation_run`；no-op、catch-up、repair 无统一 started/terminal 事件。

## 3. 不变量

1. 数据提交成功后，模型失败不得回滚数据锚点。
2. 欠账记录以 `(dbnum, to_sesno)` 为不可变身份；相同 payload 幂等，不同 payload 冲突。
3. 数据/欠账阶段未全部成功前，不得启动任何模型 cleanup、mesh、boolean、导出或 `model_gen` 发布。
4. 连续追赶只消费完整覆盖 `(model_watermark, data_watermark]` 的欠账。
5. 覆盖洞不得由 watch 自动整库重建；只允许显式受控 repair。
6. `model_gen` 发布与本次消费 debt 标记同事务完成。
7. dry-run、drain-only、非 Surreal writer 不发锚点、不消费 debt。
8. 正式 versioned 站点不得通过普通 generate/regen 绕过受控入口。

## 4. 目标架构

### 4.1 深层锁

- 公共 `run_increment`、catch-up、repair 入口内部申请 `ProjectMutationLock`。
- 模块内组合调用使用私有 `HeldProjectMutationLock` 令牌，避免重复加锁。
- 不提供公开 `skip_lock: bool`。
- CLI、watch、Web 只传业务参数，不再决定锁是否存在。

### 4.2 三阶段 IncrementRun

1. **Data stage**：按 dbnum 提交 PE/ATT，收集 anchor、fingerprint、Pending/失败。
2. **Debt stage**：只为已提交数据的 dbnum 幂等写 debt，收集覆盖洞与失败。
3. **Generation barrier**：
   - 任一 data/debt 失败：整轮 `skipped`，保留已成功数据和 debt。
   - 全部成功：构造一次 `GenerationReadSpec`，再统一执行模型阶段。

### 4.3 两个读取切面

```text
cleanup_read_at    = 当前 model_gen 水位锚点的 anchored_at
generation_read_at = 本轮最后一个 data anchor 的 anchored_at
```

- cleanup 以“当前已发布模型所对应的旧层级”展开删除/覆盖闭包。
- generation 以目标数据水位的统一 `VERSION AT` 读取当前 PE/ATT。
- 同轮所有 dbnum 共享 `generation_read_at`；输入清单记录各库目标 sesno。
- 新增后又在同一欠账区间删除、且从未进入已发布模型的节点，不要求 cleanup。

### 4.4 追赶类型

- **No-op**：五桶合并后为空；不跑几何，发布带 `reason=model_neutral` 的锚点。
- **Incremental**：完整连续 debt；旧切面算 cleanup，目标切面生成，成功后发布一个目标水位锚点。
- **Delete-only**：从 `cleanup_read_at` 的旧 hierarchy 展开被删模型闭包；目标切面不要求删除 PE 仍存在。
- **Repair**：存在覆盖洞且显式授权；按 dbnum 清理全部旧模型关系，强制完整 mesh/boolean/后处理，在目标 data anchor 切面重建。

## 5. 分阶段任务

### P0 — 可恢复基线与契约冻结

依赖：无。源码改动前硬门。

- [x] P0.1 已在分支 `agent/spec026-027-debt-catchup` 创建实现前基线提交 `955c7ff`。
- [x] P0.2 已执行并记录两项成功检查：`cargo check --lib --features "gen_model,sqlite-index,web_server"` 与 `cargo check --bin aios-database --features "gen_model,sqlite-index,web_server"`。
- [ ] P0.3 固化现有 `model_gen_debt` 表样例与 `catch-up --dry-run --json` 输出，作为兼容 fixture。
- [x] P0.4 区间沿 Version Commit 既有编码统一为 `[from_sesno, to_sesno]`；`from_sesno` 是本批首个实际 sesno，连续条件为 `next.from <= cursor + 1`，规格与 JSON 同步。

验收：可恢复一个抽样文件；区间语义在 spec、schema、JSON 三处一致。

### P1 — Debt repository 与完整观测

依赖：P0。

主要文件：

- `src/versioned_db/model_gen_debt.rs`
- `src/version_management/cli.rs`
- `src/version_management/increment_run.rs`
- `scripts/smoke/model_gen_debt_smoke.ps1`

任务：

- [ ] P1.1 将分析结果拆成“全部存活 debt ranges”和“可连续消费前缀”，不得在首个 gap 后丢失后续记录。
- [ ] P1.2 输出每段与合并后的五桶数量、gap 边界、目标 data/model 水位和 `needs_full_regen`。
- [ ] P1.3 为 consumed 但水位已覆盖、或外部 model_gen 已追平的遗留 debt 提供幂等整理规则，避免永久活行。
- [ ] P1.4 `model-version catch-up --dbnum` 改为可选；省略时从数据锚点/debt 并集解析候选 dbnum。
- [ ] P1.5 dry-run 输出稳定 JSON schema；不得写模型、锚点、debt 状态。

验收：首段即 gap、连续后再 gap、重叠 debt、空 debt、已追平五类 fixture 均能完整解释。

### P2 — 深层锁与多库 generation barrier

依赖：P0、P1。

主要文件：

- `src/version_management/project_mutation_lock.rs`
- `src/version_management/increment_run.rs`
- `src/version_management/watch_incremental.rs`
- `src/web_server/incremental_update_handlers.rs`
- `src/main.rs`

任务：

- [ ] P2.1 增加私有 held-lock 令牌与深层入口；移除 adapter 重复持锁。
- [ ] P2.2 `run_increment` 先完成所有 dbnum 的 data/debt 结果收集，再进入 generation barrier。
- [ ] P2.3 任一 commit failure、Pending 或 debt failure 时生成状态为 `skipped_due_to_data_barrier`；成功库 debt 保留。
- [ ] P2.4 Web 同步、CLI、watch 通过同一深层 seam；并发调用只能有一个进入写阶段。
- [ ] P2.5 source hash 复核覆盖模型阶段结束前，变化时不得发布 `model_gen`。

验收：两库中第二库故障时，第一库数据可提交，但两库均无本轮模型改写和新 `model_gen`。

### P3 — 显式 GenerationReadSpec 与主表读取

依赖：P2；对应 `specs/027` T022、T026、T027。

主要文件：

- `src/generation_read/`
- `src/fast_model/gen_model/orchestrator.rs`
- `src/fast_model/gen_model/context.rs`
- `src/version_management/model_gen_catchup.rs`

任务：

- [ ] P3.1 定义不可变 `GenerationReadSpec { read_at, observed_watermarks, mode }`；增量/追赶/repair 必填，初始化全量为 live。
- [ ] P3.2 从本轮最后一个 data anchor 解析唯一 `generation_read_at`。
- [ ] P3.3 从当前 `model_gen` 锚点解析 `cleanup_read_at`；无模型锚点且有连续历史时明确转 repair，不猜测当前态。
- [ ] P3.4 Surreal adapter 通过领域 trait 对主 PE/ATT、owner、reference、transform 批量查询，并集中附加 `VERSION AT`。
- [ ] P3.5 同轮复用一个 read session；删除 `generation_replica_*`/manifest binding 依赖前，先完成语义对照 smoke。
- [ ] P3.6 trace/summary 输出实际 `read_at`，所有生成 SQL 必须完全一致。

验收：SQL trace 可机械断言同轮只有一个 generation `VERSION AT`；禁止空后缀退回 latest。

### P4 — 统一 catch-up executor、delete-only 与 repair

依赖：P1–P3。

主要文件：

- `src/version_management/model_gen_catchup.rs`
- `src/versioned_db/model_gen_debt.rs`
- `src/fast_model/gen_model/write_pipeline.rs`
- `src/fast_model/gen_model/pdms_inst.rs`
- `src/fast_model/gen_model/orchestrator.rs`

任务：

- [ ] P4.1 catch-up 改为先规划多库 `CatchUpPlan`，再一次执行，而非循环中每库独立打开 session。
- [ ] P4.2 incremental cleanup 使用 `cleanup_read_at` 的旧 hierarchy；删除根不要求存在于目标切面。
- [ ] P4.3 delete-only 走同一 writer lifecycle，完成旧模型关系、AABB、transform、boolean/negative 关联清理后发布锚点。
- [ ] P4.4 no-op 跳过生成但仍经统一 finalize，锚点和运行台账记录明确原因。
- [ ] P4.5 gap repair 不复用普通 Full scope：新增受控 dbnum sweep + 强制 mesh/boolean/后处理 + 目标锚点绑定。
- [ ] P4.6 生成、cleanup、后处理、导出任一步失败都不 finalize；debt 留存重试。
- [ ] P4.7 finalize 事务只消费本次 plan 明确覆盖的 debt ids，不使用宽泛 `to_sesno <= target` 误消费旁支记录。

验收：metadata-only、delete-only、连续多段 debt、故障重试、gap repair 五条路径均只发布预期锚点。

### P5 — 运行台账与入口权限

依赖：P2–P4；对应 `specs/027` T028–T030。

主要文件：

- 新模块：`src/version_management/model_generation_run.rs`
- `src/version_management/model_gen_catchup.rs`
- `src/version_management/cli.rs`
- `src/cli_modes.rs`
- Web 生成/repair handlers

任务：

- [ ] P5.1 建立 append-only `model_generation_run` event repository；事件至少含 run id、kind、actor、reason、dbnums、水位、两个 read_at、contract hash、结果和错误。
- [ ] P5.2 initialization、incremental、no-op、catch-up、repair 统一写 started 与 terminal。
- [ ] P5.3 Ready versioned 站点普通 generate/regen 在 service seam 硬拒绝；CLI/HTTP 不能绕过。
- [ ] P5.4 repair 必须绑定既有 data anchor；同 sesno 重做只更新该模型提交的当前结果，运行历史只追加不覆盖。
- [ ] P5.5 started 无 terminal 的运行可识别为 abandoned，后续恢复不误判成功。

验收：两次同 sesno repair 只有一个模型版本身份、两个 run id、四条 started/terminal 事件。

### P6 — 验证与上线门

依赖：P1–P5。

新增/更新脚本：

- `scripts/smoke/model_gen_debt_smoke.ps1`
- `scripts/smoke/model_gen_failure_recovery_smoke.ps1`
- `scripts/smoke/model_gen_neutral_smoke.ps1`
- `scripts/smoke/model_gen_delete_only_smoke.ps1`
- `scripts/smoke/model_gen_gap_repair_smoke.ps1`
- `scripts/smoke/model_gen_multidb_barrier_smoke.ps1`
- `scripts/smoke/model_gen_lock_and_read_at_smoke.ps1`

验证：

- [ ] P6.1 失败自愈：注入 generation failure，数据水位前进，下一轮追平。
- [ ] P6.2 metadata-only：五桶为空、模型文件 checksum 不变、no-op 锚点前进。
- [ ] P6.3 delete-only：latest 模型不可见、旧锚点 `VERSION AT` 可见、目标锚点前进。
- [ ] P6.4 gap：watch 只报告；无授权不写；显式 repair 清理旧模型并追平。
- [ ] P6.5 多库 barrier：第二库 commit/debt 故障时整轮无模型写和锚点。
- [ ] P6.6 锁：CLI/watch/Web 交叉触发只有一个写者，其余返回结构化 contention。
- [ ] P6.7 read_at：同轮所有生成查询的 `VERSION AT` 完全一致。
- [ ] P6.8 构建：默认、目标 feature 组合、`scripts/build-sync-cli.ps1`；不运行或编译 tests。
- [ ] P6.9 启动真实 web_server，以 HTTP POST 验证 sync、状态查询、失败恢复和 repair 权限。

上线门：P6 全部有命令、退出码、关键 JSON 与数据库断言证据；`specs/026/tasks.md` 和 `specs/027/tasks.md` 只在证据齐全后勾选。

## 6. 推荐提交切片

1. `debt-report-contract`：区间语义、全量 ranges、五桶计数、CLI all-db dry-run。
2. `deep-mutation-lock`：深层锁令牌与 adapter 收敛。
3. `increment-generation-barrier`：多库 data/debt barrier 与 summary。
4. `generation-read-spec`：统一 read_at 类型、anchor 解析和 trace。
5. `main-table-read-adapter`：主 PE/ATT 领域查询适配器。
6. `delete-only-cleanup`：旧模型切面 cleanup + delete-only。
7. `controlled-gap-repair`：整库 sweep、强制重建和锚点绑定。
8. `model-generation-run-ledger`：append-only 运行事件与 direct regen guard。
9. `debt-catchup-smoke-suite`：七类 CLI/HTTP smoke 与文档证据。

每个切片必须保持可构建；不得先删 replica/旧调用方再补替代实现。

## 7. 回滚与故障策略

- 源码回滚依赖 P0 基线；不以删除真实数据目录作为回滚。
- 数据提交已成功但 barrier 后失败：保留数据和 debt，下轮受控追赶。
- debt 写失败：保留数据，记录 gap；禁止自动 full，等待 repair。
- 模型写中断：无 terminal success、无新锚点、无 debt 消费；重试先 cleanup 再生成。
- repair 中断：旧 `model_gen` 仍是最后可信入口；started-only run 标 abandoned。
- 新 JSON 字段只追加；旧字段至少保留一个兼容窗口，smoke 同时断言新旧消费者。

## 8. 完成定义

- spec-026 FR-001～FR-011 与 SC-001～SC-006 均有可复现证据。
- spec-027 FR-007、FR-011、FR-017～FR-019 在本范围内全部满足。
- delete-only 不依赖目标切面存在已删除 PE。
- 多库失败不会产生部分模型提交。
- catch-up/repair 不读 latest，不存在无 `VERSION AT` 的增量生成查询。
- 正式 versioned 站点无法直接 generate/regen。
- 所有模型尝试可由 `model_generation_run` 重建因果链。
- 目标 feature 的 lib 与 CLI 编译通过；真实 Web HTTP smoke 通过；未使用 cargo test。
