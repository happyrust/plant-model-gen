# Quickstart: 按版本实时查询模型树 — 验证指引

**状态**: 能力验证（FR-011）已完成 2026-07-19；端到端验收待 M3/M4 落地后执行。

## 前置

- fork surreal 二进制（dev-3.1，≥3.2.0-nightly）在 PATH
- versioned 站点或 fixture 实例（`db-data/run_surrealkv_versioned.ps1`，port 8030）
- specs/022 锚点体系可用（`sesno_version_anchor` + `fn::sesno_version`）

## Scenario 0: 能力验证（已完成，可重跑）

```powershell
powershell -File db-data/run_surrealkv_versioned.ps1   # 如 8030 未起
powershell -File scripts/smoke/pe_owner_version_capability_smoke.ps1
```

**Expected**：C0/C1/C2/C4/C5 语义正确；C3（id 区间扫 + VERSION）返回当前态——**这是禁用写法的证据**；C6 撞 id 报错。原始输出 `db-data/smoke_023_result.json`，结论见 `research.md`。

## Scenario 1: 增量维护 pe_owner（M1 交付后）

1. 对 fixture 站点跑一次含层级变更的增量：`incremental-sesno --file <...> --json`
2. 核对提交摘要含 `pe_owner_rows` 计数、锚点正常固化
3. 一致性：

```surql
-- 当前态双源一致
SELECT VALUE in FROM pe:<X><-pe_owner ORDER BY record::id(id)[1];
SELECT VALUE children FROM pe:<X>;
-- 历史回溯（增量前锚点）
LET $t = fn::sesno_version(<DBNUM>, <增量前 sesno>);
SELECT VALUE in FROM pe:<X><-pe_owner VERSION $t;
```

**Expected**：当前态两源一致；历史查询返回旧子列表；`pe_owner_version_meta:<dbnum>` 已写入。

## Scenario 2: 树接口版本查询（M3 交付后）

```powershell
# 版本列表（022 既有）
Invoke-RestMethod "$base/api/model-history/anchors?dbnum=$d"
# 两个锚点分别查 children
Invoke-RestMethod "$base/api/e3d/children/$refno?sesno=$oldSesno"
Invoke-RestMethod "$base/api/e3d/children/$refno?sesno=$newSesno"
# 不传 sesno（现状路径，零回归）
Invoke-RestMethod "$base/api/e3d/children/$refno"
```

**Expected**：两锚点子集/顺序/名称各自正确；响应带 `version` 信封；无锚点 sesno 返回 404 AnchorMissing；`site-nodes`/`visible-insts` 带 sesno 返回 400 VersionUnsupported。

## Scenario 3: 端到端验收（M4）

```powershell
powershell -File scripts/smoke/tree_version_smoke.ps1
```

**Expected**：SC-001~SC-005 全过（增/删/移/改名四类变更两锚点比对；性能抽样 P95 ≤ 1s；不传 sesno 对照无回归；fallback source 标注正确）。

## Scenario 4: 存量站点接入（M5）

1. 升级二进制（先确认无 `commit_pending` 残留）
2. `model-version rebuild-pe-owner --dbnum <D> --json`
3. 抽样核对边与 children 一致率 100%；`pe_owner_version_meta` 更新
4. 老锚点（重建前）查询自动走 `pe_children_fallback` 且结果正确

## 已知边界

- 版本粒度 per-dbnum；多 dbnum 世界树由前端按 dbnum 组合
- retention 窗外 410；兜底仍是 PDMS 源 db 重扫（022 既定）
- 几何/instances 不按版本（后续独立 feature）
