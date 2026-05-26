-- DuckLake-side parity query for `ducklake-canonical.raw_inst_relate`.

SELECT 'row_count' AS metric, COUNT(*) AS value FROM "ducklake-canonical"."raw_inst_relate";
SELECT 'pk_distinct_refno' AS metric, COUNT(DISTINCT refno) AS value FROM "ducklake-canonical"."raw_inst_relate";
SELECT 'pk_distinct_inst_id' AS metric, COUNT(DISTINCT inst_id) AS value FROM "ducklake-canonical"."raw_inst_relate";

WITH ordered AS (
  SELECT refno, inst_id, ROW_NUMBER() OVER (ORDER BY refno, inst_id) AS rn,
         COUNT(*) OVER () AS total
  FROM "ducklake-canonical"."raw_inst_relate"
)
SELECT 'sample_first' AS marker, refno, inst_id FROM ordered WHERE rn = 1
UNION ALL
SELECT 'sample_mid', refno, inst_id FROM ordered WHERE rn = GREATEST(1, total / 2)
UNION ALL
SELECT 'sample_last', refno, inst_id FROM ordered WHERE rn = total;
