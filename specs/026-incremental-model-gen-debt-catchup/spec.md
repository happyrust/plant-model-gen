# Feature Specification: 增量模型生成欠账追赶闭环

**Created**: 2026-07-22
**Status**: Accepted for implementation
**Upstream**: `docs/adr/0006-model-generation-watermark-debt-catchup.md`（依赖 ADR-0002 / ADR-0005）
**Amended by**: `docs/adr/0010-initialization-and-incremental-version-boundary.md` / specs/027（FR-005 的任意 `allow-full-regen` 已收敛为绑定既有数据锚点、持项目锁且写运行审计的受控 catch-up/repair；欠账失败不回滚数据的语义不变）
**术语**: 模型生成水位 / 模型生成欠账，见根 `CONTEXT.md`

## User Scenarios

### US1 - 增量更新默认产出模型

操作者以默认配置运行 `watch-incremental`。源文件 sesno 前进后，一轮内完成数据版本提交与增量模型生成，`model_gen` 锚点追平数据水位；`e3d_tree_api` 按新 sesno 查询立即可见新模型。

### US2 - 生成失败自愈

某轮模型生成失败（或进程中断），数据水位照常推进、不被阻断。下一轮 watch 检测到该 dbnum 模型生成水位落后，消费留存的欠账行补生成并一次追平。

### US3 - 存量断更站点首次追平

升级后站点存在历史断更区间且无欠账行覆盖（洞）。watch 只告警、不自动整库重建；运维发起绑定既有数据锚点、持项目锁并写运行审计的受控 catch-up/repair，追平后回到欠账行常态。

### US4 - 元数据修改零生成

设计侧批量修改 NAME/DESC 等纯业务属性。采集正常提交数据版本；元素不进生成桶，模型生成为空操作但锚点照发（水位对齐）；summary 的 `model_neutral_changes` 可核查全部被过滤元素。

### US5 - 纯删除推进水位

增量区间只含删除。模型产物清理照常执行、锚点照发；`VERSION AT` 旧锚点仍能查到删除前的模型（MVCC 墓碑保留历史）。

### US6 - 欠账可观测

`model-version catch-up --dry-run --json` 输出每 dbnum 的数据水位、模型生成水位、欠账区间与五桶规模、覆盖性（完整/有洞/需整库兜底）；watch 每轮 summary 携带同款字段。

## Functional Requirements

- **FR-001**: 数据版本提交成功 MUST 同流程幂等写入该 dbnum 的欠账行（五桶 refno + `(from_sesno, to_sesno]` 区间）；欠账行写入失败 MUST 告警并按洞语义处理，不回滚已生效的数据提交。
- **FR-002**: 模型生成水位 MUST 定义为该 dbnum `model_gen` 锚点的最高 sesno（无锚点视为 0）。
- **FR-003**: watch 每轮 MUST 对每个候选 dbnum 比对模型生成水位与已提交水位；落后且欠账行完整覆盖区间时 MUST 合并五桶并集执行一次 Incremental scope 生成。
- **FR-004**: 追平成功 MUST 只发布一个 `model_gen` 锚点（sesno = 数据水位）并标记所消费欠账行；生成或后处理失败 MUST NOT 发锚点、MUST NOT 消费欠账行。
- **FR-005**: 欠账区间存在洞 MUST NOT 自动整库重建；MUST 告警并在 dry-run/summary 标注 `needs_full_regen`；整库兜底 MUST 仅由绑定既有数据锚点、持项目 mutation lock 且追加 `model_generation_run` 审计的受控 catch-up/repair 触发，单独的 `--allow-full-regen` 标志不得绕过该门禁。
- **FR-006**: 纯删除与空操作运行 MUST 照常推进模型生成水位（发锚点）。
- **FR-007**: `watch-incremental` / `incremental-sesno` 及 web 增量入口的 `generate_model` 默认 MUST 为开；`--no-generate-model` MUST 可显式关闭，关闭时不消费欠账、不发锚点。
- **FR-008**: 采集分类 MUST 应用属性级影响过滤：仅 Modified 且全部变更属性不影响生成输入的元素不进生成桶；Added/Deleted/OWNER 变化/noun 变化 MUST 无条件进桶；生成链路上未知属性 MUST 默认触发；被过滤元素 MUST 记入 `model_neutral_changes`；`--no-model-impact-filter` MUST 恢复全触发。
- **FR-009**: 追赶失败 MUST per-dbnum 隔离：单库失败不阻断其它 dbnum 与后续数据提交，欠账留存下轮重试。
- **FR-010**: dry-run / 非 Surreal writer 模式 MUST NOT 发锚点、MUST NOT 消费欠账行。
- **FR-011**: 追赶生成 MUST 复用既有 Incremental scope 管线（含 `pre_cleanup_for_regen_versioned` 与 delete 桶清理），不引入第二套生成路径。

## Success Criteria

- **SC-001**: 注入生成失败后，下一轮 watch 自动补生成并追平（锚点 = 数据水位），全程数据提交未被阻断。
- **SC-002**: 断更站点（模型水位落后且区间有洞）watch 只告警；绑定目标数据锚点的受控 catch-up/repair 一次追平并发锚点，运行台账可查。
- **SC-003**: 纯元数据批量修改的增量：生成桶为空、锚点照发、`model_neutral_changes` 列出全部被过滤元素。
- **SC-004**: delete-only 增量：模型产物 latest 不可见、`VERSION AT` 旧锚点可见、锚点照常推进。
- **SC-005**: `catch-up --dry-run --json` 与 watch summary 的水位/欠账字段可供 smoke 脚本直接断言。
- **SC-006**: `--no-generate-model` 下行为与升级前默认（纯数据同步）完全一致。

## Non-goals

- 模型 diff API、按 sesno 模型实例集 API（等出现真实消费方再立项）。
- 自动触发最小交付单元 unit-export（交付历史仍为显式操作，见 ADR-0005）。
- mesh 文件 GC 机制（仅记录"锚点可达 geo_hash 为存活根"约束）。
- 增量数据链本体的修改（lease / fingerprint / 连续性门禁等，见 `docs/plans/2026-07-20-incremental-update-hardening-dev-plan.md`）。
