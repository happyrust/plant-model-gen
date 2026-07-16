# DDL Draft: unit version by (dbnum, refno, sesno)

**Migration id (planned)**: `0007_unit_version_by_refno_sesno`  
**Schema**: `model_version`  
**Note**: DuckLake/DuckDB 侧以 `CREATE TABLE IF NOT EXISTS` + 应用层唯一键约定为主（与现有 `ducklake_store.rs` 一致）。下列 `PRIMARY KEY` 为逻辑约束；若引擎限制则改为 UNIQUE INDEX 或应用层幂等。

## New / replacement tables

```sql
-- 运维批次元数据（可选，非版本身份）
CREATE TABLE IF NOT EXISTS "model_version"."export_batches" (
    dbnum INTEGER,
    batch_sesno INTEGER,
    project_name TEXT,
    package_relpath TEXT,
    package_hash TEXT,
    generation_job_id TEXT,
    created_at TEXT,
    note TEXT
);

-- 最小交付单元版本：真相主键 (dbnum, unit_refno_u64, sesno)
CREATE TABLE IF NOT EXISTS "model_version"."unit_versions_v2" (
    dbnum INTEGER,
    unit_refno_u64 BIGINT,
    sesno INTEGER,
    project_name TEXT,
    unit_refno_str TEXT,
    unit_noun TEXT,
    unit_key TEXT,
    aggregate_hash TEXT,
    hash_version TEXT,
    rule_set_hash TEXT,
    member_count BIGINT,
    unresolved_member_count BIGINT,
    member_signature TEXT,
    package_relpath TEXT,
    status TEXT,
    label TEXT,
    -- 过渡期只读兼容；新写入应 NULL
    legacy_release_id TEXT,
    indexed_at TEXT
);

-- 组件快照：主键 (dbnum, component_refno_u64, sesno)
CREATE TABLE IF NOT EXISTS "model_version"."component_snapshots_v2" (
    dbnum INTEGER,
    component_refno_u64 BIGINT,
    sesno INTEGER,
    project_name TEXT,
    component_refno_str TEXT,
    component_key TEXT,
    noun TEXT,
    unit_refno_u64 BIGINT,
    unit_refno_str TEXT,
    owner_refno_u64 BIGINT,
    owner_refno_str TEXT,
    owner_noun TEXT,
    cata_hash TEXT,
    trans_hash TEXT,
    aabb_hash TEXT,
    spec_value BIGINT,
    has_neg BOOLEAN,
    geo_signature TEXT,
    component_hash TEXT,
    hash_version TEXT,
    member_sesno INTEGER,
    legacy_release_id TEXT,
    indexed_at TEXT
);

-- 单元成员：主键 (dbnum, unit_refno_u64, sesno, member_refno_u64)
-- unit 行的 sesno = max(member_sesno) over members
CREATE TABLE IF NOT EXISTS "model_version"."unit_memberships_v2" (
    dbnum INTEGER,
    unit_refno_u64 BIGINT,
    sesno INTEGER,
    member_refno_u64 BIGINT,
    project_name TEXT,
    unit_refno_str TEXT,
    unit_noun TEXT,
    unit_key TEXT,
    member_refno_str TEXT,
    member_noun TEXT,
    member_sesno INTEGER,
    component_hash TEXT,
    membership_kind TEXT,
    path_confidence DOUBLE,
    unresolved_reason TEXT,
    membership_hash TEXT,
    hash_version TEXT,
    legacy_release_id TEXT,
    indexed_at TEXT
);

-- 索引运行记录：按 (dbnum, sesno) 或 (dbnum, unit_refno, sesno) 聚合
CREATE TABLE IF NOT EXISTS "model_version"."unit_index_runs_v2" (
    dbnum INTEGER,
    sesno INTEGER,
    project_name TEXT,
    hash_version TEXT,
    rule_set_hash TEXT,
    unit_count BIGINT,
    member_count BIGINT,
    unresolved_member_count BIGINT,
    indexed_at TEXT
);

CREATE TABLE IF NOT EXISTS "model_version"."component_index_runs_v2" (
    dbnum INTEGER,
    sesno INTEGER,
    project_name TEXT,
    hash_version TEXT,
    component_count BIGINT,
    distinct_component_hashes BIGINT,
    indexed_at TEXT
);

-- specs/023 E2：单元状态事件（挂 (dbnum, refno, sesno)，不挂 release_id）
CREATE TABLE IF NOT EXISTS "model_version"."unit_version_status_events_v2" (
    dbnum INTEGER,
    unit_refno_u64 BIGINT,
    sesno INTEGER,
    status TEXT,
    reason TEXT,
    created_at TEXT
);
```

## Logical unique keys

```text
unit_versions_v2          UNIQUE (dbnum, unit_refno_u64, sesno)
component_snapshots_v2    UNIQUE (dbnum, component_refno_u64, sesno)
unit_memberships_v2       UNIQUE (dbnum, unit_refno_u64, sesno, member_refno_u64)
export_batches            UNIQUE (dbnum, batch_sesno, package_hash)  -- 若需要防重
```

## Unit sesno rule

```text
for each unit U at export:
  members = resolve_membership(U)
  unit_sesno = max(member.sesno for member in members if member.sesno is known)
  write unit_versions_v2(dbnum, U.refno, unit_sesno, …)
  write unit_memberships_v2(…, sesno=unit_sesno, member_sesno=…)
  write component_snapshots_v2(…, sesno=component's own sesno OR unit_sesno — 见 tasks C)
```

**推荐**：`component_snapshots_v2.sesno` = 该组件自身导出时的 sesno；  
`unit_versions_v2.sesno` = `max(member_sesno)`。  
单元与成员 sesno 不必相等；diff 单元时用 unit 行的 sesno。

## Deprecated (stop writing as identity)

- `model_releases.release_id` 作为版本 PK
- `model_release_edges.parent_release_id`
- 所有子表以 `release_id` 为唯一关联

旧表可保留只读至 Phase E；新索引路径只写 `*_v2`。

## Backfill sketch (only when sesno known)

```sql
-- 伪代码：从 history_publish metadata / extra_metadata 抽 to_sesno
-- INSERT INTO unit_versions_v2
-- SELECT dbnum, unit_refno_u64, to_sesno AS sesno, …, release_id AS legacy_release_id
-- FROM unit_versions
-- JOIN … metadata WHERE to_sesno IS NOT NULL;
```

无可解析 sesno 的旧行：**不迁**，重 export。
