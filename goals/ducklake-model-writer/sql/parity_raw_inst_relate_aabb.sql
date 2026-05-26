-- DuckLake-side parity query for `ducklake-canonical.raw_inst_relate_aabb`.
-- Slice 3 emits per-mesh-aabb edges (no union AABB per refno);
-- parity vs SurrealDB may differ on union semantics. Source column captures
-- the origin of the edge ('mesh' currently; future slices may add 'union').

SELECT 'row_count' AS metric, COUNT(*) AS value FROM "ducklake-canonical"."raw_inst_relate_aabb";
SELECT 'distinct_refno' AS metric, COUNT(DISTINCT refno) AS value FROM "ducklake-canonical"."raw_inst_relate_aabb";
SELECT 'distinct_aabb_id' AS metric, COUNT(DISTINCT aabb_id) AS value FROM "ducklake-canonical"."raw_inst_relate_aabb";
SELECT 'distinct_source' AS metric, COUNT(DISTINCT source) AS value FROM "ducklake-canonical"."raw_inst_relate_aabb";

WITH ordered AS (
  SELECT refno, aabb_id, source,
         ROW_NUMBER() OVER (ORDER BY refno, aabb_id) AS rn,
         COUNT(*) OVER () AS total
  FROM "ducklake-canonical"."raw_inst_relate_aabb"
)
SELECT 'sample_first' AS marker, refno, aabb_id, source FROM ordered WHERE rn = 1
UNION ALL
SELECT 'sample_mid', refno, aabb_id, source FROM ordered WHERE rn = GREATEST(1, total / 2)
UNION ALL
SELECT 'sample_last', refno, aabb_id, source FROM ordered WHERE rn = total;
