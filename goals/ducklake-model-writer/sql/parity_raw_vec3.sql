-- DuckLake-side parity query for `ducklake-canonical.raw_vec3`.
-- Only mesh-derived pts payloads written here (Q1=C scope); tubi pts are Known Gap.

SELECT 'row_count' AS metric, COUNT(*) AS value FROM "ducklake-canonical"."raw_vec3";
SELECT 'pk_distinct_vec3_id' AS metric, COUNT(DISTINCT vec3_id) AS value FROM "ducklake-canonical"."raw_vec3";

WITH ordered AS (
  SELECT vec3_id, ROW_NUMBER() OVER (ORDER BY vec3_id) AS rn,
         COUNT(*) OVER () AS total
  FROM "ducklake-canonical"."raw_vec3"
)
SELECT 'sample_first' AS marker, vec3_id FROM ordered WHERE rn = 1
UNION ALL
SELECT 'sample_mid', vec3_id FROM ordered WHERE rn = GREATEST(1, total / 2)
UNION ALL
SELECT 'sample_last', vec3_id FROM ordered WHERE rn = total;
