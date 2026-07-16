# Tasks: DuckLake 交付单元版本键 (dbnum, refno, sesno)

**Input**: `adr.md`, `ddl-draft.sql`  
**验证规则**: 不写 cargo test；CLI `--json` + DuckLake SQL 冒烟。

## Phase A — ADR 已落地 / 盘点

- [x] A1 写 ADR（`adr.md`）
- [x] A2 写 DDL 草案（`ddl-draft.sql`）
- [x] A3 盘点 `release_id` 读写点清单（`inventory.md`：命中表 + legacy/v2 API 职责）
- [x] A4 可映射 sesno 分类与弃旧重灌策略写进 `inventory.md`（路径分类；无固定生产样例库）

## Phase B — Schema + 写路径（第一刀）

- [x] B1 `ducklake_store.rs` 增加 migration `0007_unit_version_by_refno_sesno`：创建 `*_v2` / `export_batches`（按 `ddl-draft.sql`）
- [x] B2 写入 API：`upsert_unit_version_v2(dbnum, unit_refno, sesno, …)` 幂等（同 hash no-op / 异 hash 拒绝）
- [x] B3 索引路径（最小闭环）：`write_unit_version_with_members_v2` 算 `unit_sesno = max(member_sesno)` 后写 unit + memberships（尚未接到旧 `index_release_units`）
- [x] B4 停止向旧路径**宣传为推荐写路径**：`index_release_units` / CLI `index-units` 标注 DEPRECATED（specs/023）；完整 cutover 仍待后续 PR
- [x] B5 冒烟：`model-version unit-v2-smoke` + `scripts/smoke/unit_version_v2_smoke.ps1`

## Phase C — 读路径 / Diff

- [x] C1 `get_unit_version_v2` / `list_unit_versions_v2` + CLI `unit-v2-get` / `unit-v2-list`
- [x] C2 `diff_unit_versions_v2(dbnum, from_sesno, to_sesno, refno?)` + CLI `unit-v2-diff`；smoke 覆盖 changed=1
- [x] C3 过渡 dual-read：`get`/`list`/`diff` 扫描在 v2 无命中时，从可解析的遗留 `unit_versions.release_id`（`db{N}-s{M}`）只读回退；smoke 覆盖

## Phase D — 调用方收敛

- [x] D1 部分：`ModelReleaseRegisterRequest.export_sesno`；CLI `register` 可省略 `--release-id`（需 `--sesno` → `db{N}-s{sesno}`）；HTTP register 支持 `sesno`
- [x] D2 部分：`register` / `publish-history` 在 `index_units` 后调用 `sync_release_units_into_v2(release_id, sesno)`；完整去掉 release_id 拼装仍待后续
- [x] D3 `cli.rs`：`unit-diff` 首选 `--dbnum --from-sesno --to-sesno [--refno]`；旧 `--from/to-release-id` 告警，可解析 `db{N}-s{M}` 时映射到 v2；`unit-get`/`unit-list` 别名；`index-units` 告警

## Phase E — 包路径与清理（可后置）

- [x] E1 物化目录：`unit_version_package_relpath` / `materialize_unit_version_package_dir`；upsert/sync 写入 `package_relpath`；smoke 覆盖
- [x] E2：`unit_version_status_events_v2` + `set/list_unit_version_status*`；状态机标明为 export-batch；CLI `unit-v2-set-status` / `unit-v2-events`；reconcile/events 可用 `--dbnum --sesno`
- [x] E3 文档：`coexistence-022.md`；022 spec 决策表/FR-010 交叉引用；planning `GOAL.md` 标注 release_id 图过时
- [ ] E4 删除 dual-read 与 legacy 列（单独迭代）

## Checkpoint 验收

1. 新写入主键可解析为 `(dbnum, refno, sesno)`，无新建 release_id 依赖。
2. 成员 max sesno N→M 时出现新 unit 行，旧行仍可查。
3. diff 按 sesno 区间工作。
4. 022 PE/ATT 路径无回归。

## Suggested first coding PR

**B1 + B2 + B3 + B5 only**（schema + 写 + 单元 sesno 规则 + 冒烟），不碰 CLI 大改。
