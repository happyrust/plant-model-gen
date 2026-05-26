-- DuckLake-side parity query for `ducklake-canonical.raw_neg_relate`.
-- Slice 4 reconcile inserts sentinel rows with target_refno='__reconcile_pending__'.
-- Parity comparison vs SurrealDB EXCEPTS those rows.

SELECT 'row_count_total' AS metric, COUNT(*) AS value FROM "ducklake-canonical"."raw_neg_relate";
SELECT 'row_count_real' AS metric, COUNT(*) AS value FROM "ducklake-canonical"."raw_neg_relate" WHERE target_refno <> '__reconcile_pending__';
SELECT 'sentinel_reconcile_rows' AS metric, COUNT(*) AS value FROM "ducklake-canonical"."raw_neg_relate" WHERE target_refno = '__reconcile_pending__';
SELECT 'distinct_carrier_real' AS metric, COUNT(DISTINCT carrier_refno) AS value FROM "ducklake-canonical"."raw_neg_relate" WHERE target_refno <> '__reconcile_pending__';

WITH ordered AS (
  SELECT carrier_refno, target_refno,
         ROW_NUMBER() OVER (ORDER BY carrier_refno, target_refno) AS rn,
         COUNT(*) OVER () AS total
  FROM "ducklake-canonical"."raw_neg_relate"
  WHERE target_refno <> '__reconcile_pending__'
)
SELECT 'sample_first' AS marker, carrier_refno, target_refno FROM ordered WHERE rn = 1
UNION ALL
SELECT 'sample_mid', carrier_refno, target_refno FROM ordered WHERE rn = GREATEST(1, total / 2)
UNION ALL
SELECT 'sample_last', carrier_refno, target_refno FROM ordered WHERE rn = total;
