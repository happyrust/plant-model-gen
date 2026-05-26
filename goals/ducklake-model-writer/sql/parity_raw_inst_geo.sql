-- DuckLake-side parity query for `ducklake-canonical.raw_inst_geo`.
-- raw_inst_geo gets mesh columns filled by Slice 3 persist_mesh_results
-- (meshed/bad/mesh_aabb_id/mesh_pts_hashes_json). Slice 6 parity records
-- both the total row count and the meshed-row count to track Slice 2/3 split.

SELECT 'row_count' AS metric, COUNT(*) AS value FROM "ducklake-canonical"."raw_inst_geo";
SELECT 'pk_distinct_geo_hash' AS metric, COUNT(DISTINCT geo_hash) AS value FROM "ducklake-canonical"."raw_inst_geo";
SELECT 'meshed_rows' AS metric, COUNT(*) AS value FROM "ducklake-canonical"."raw_inst_geo" WHERE meshed IS TRUE;
SELECT 'bad_rows' AS metric, COUNT(*) AS value FROM "ducklake-canonical"."raw_inst_geo" WHERE bad IS TRUE;

WITH ordered AS (
  SELECT geo_hash, ROW_NUMBER() OVER (ORDER BY geo_hash) AS rn,
         COUNT(*) OVER () AS total
  FROM "ducklake-canonical"."raw_inst_geo"
)
SELECT 'sample_first' AS marker, geo_hash FROM ordered WHERE rn = 1
UNION ALL
SELECT 'sample_mid', geo_hash FROM ordered WHERE rn = GREATEST(1, total / 2)
UNION ALL
SELECT 'sample_last', geo_hash FROM ordered WHERE rn = total;
