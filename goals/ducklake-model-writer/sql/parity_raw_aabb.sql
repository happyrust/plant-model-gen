-- DuckLake-side parity query for `ducklake-canonical.raw_aabb`.
-- Only mesh-derived AABBs are written here (Q1=C scope); tubi AABBs are Known Gap.

SELECT 'row_count' AS metric, COUNT(*) AS value FROM "ducklake-canonical"."raw_aabb";
SELECT 'pk_distinct_aabb_id' AS metric, COUNT(DISTINCT aabb_id) AS value FROM "ducklake-canonical"."raw_aabb";

-- Float precision check: total min/max coords are useful but we report bounds extent.
SELECT 'min_x_min' AS metric, MIN(min_x) AS value FROM "ducklake-canonical"."raw_aabb";
SELECT 'max_x_max' AS metric, MAX(max_x) AS value FROM "ducklake-canonical"."raw_aabb";

WITH ordered AS (
  SELECT aabb_id, ROW_NUMBER() OVER (ORDER BY aabb_id) AS rn,
         COUNT(*) OVER () AS total
  FROM "ducklake-canonical"."raw_aabb"
)
SELECT 'sample_first' AS marker, aabb_id FROM ordered WHERE rn = 1
UNION ALL
SELECT 'sample_mid', aabb_id FROM ordered WHERE rn = GREATEST(1, total / 2)
UNION ALL
SELECT 'sample_last', aabb_id FROM ordered WHERE rn = total;
