-- DuckLake-side parity query for `ducklake-canonical.raw_inst_info`.
-- Run with: duckdb metadata.ducklake -c "INSTALL ducklake; LOAD ducklake; ATTACH ...; <this SQL>"
-- See goals/ducklake-model-writer/sql/README.md for the runner.

-- (1) row count
SELECT 'row_count' AS metric, COUNT(*) AS value FROM "ducklake-canonical"."raw_inst_info";

-- (2) distinct primary key count (inst_id is the canonical id derived from RefnoEnum.to_string())
SELECT 'pk_distinct' AS metric, COUNT(DISTINCT inst_id) AS value FROM "ducklake-canonical"."raw_inst_info";

-- (3) 3 sample inst_id at first / midpoint / last when ordered by inst_id
WITH ordered AS (
  SELECT inst_id, ROW_NUMBER() OVER (ORDER BY inst_id) AS rn,
         COUNT(*) OVER () AS total
  FROM "ducklake-canonical"."raw_inst_info"
)
SELECT 'sample_first' AS marker, inst_id FROM ordered WHERE rn = 1
UNION ALL
SELECT 'sample_mid', inst_id FROM ordered WHERE rn = GREATEST(1, total / 2)
UNION ALL
SELECT 'sample_last', inst_id FROM ordered WHERE rn = total;
