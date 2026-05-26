-- DuckLake-side parity query for `ducklake-canonical.raw_geo_relate`.

SELECT 'row_count' AS metric, COUNT(*) AS value FROM "ducklake-canonical"."raw_geo_relate";
SELECT 'pk_distinct_inst_id_geo_hash' AS metric, COUNT(DISTINCT (inst_id, geo_hash)) AS value FROM "ducklake-canonical"."raw_geo_relate";
SELECT 'distinct_inst_id' AS metric, COUNT(DISTINCT inst_id) AS value FROM "ducklake-canonical"."raw_geo_relate";
SELECT 'distinct_geo_hash' AS metric, COUNT(DISTINCT geo_hash) AS value FROM "ducklake-canonical"."raw_geo_relate";
SELECT 'tubi_rows' AS metric, COUNT(*) AS value FROM "ducklake-canonical"."raw_geo_relate" WHERE is_tubi IS TRUE;

WITH ordered AS (
  SELECT inst_id, geo_hash, geom_refno, idx,
         ROW_NUMBER() OVER (ORDER BY inst_id, geo_hash, idx) AS rn,
         COUNT(*) OVER () AS total
  FROM "ducklake-canonical"."raw_geo_relate"
)
SELECT 'sample_first' AS marker, inst_id, geo_hash, idx FROM ordered WHERE rn = 1
UNION ALL
SELECT 'sample_mid', inst_id, geo_hash, idx FROM ordered WHERE rn = GREATEST(1, total / 2)
UNION ALL
SELECT 'sample_last', inst_id, geo_hash, idx FROM ordered WHERE rn = total;
