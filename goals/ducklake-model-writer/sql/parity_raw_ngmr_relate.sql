-- DuckLake-side parity query for `ducklake-canonical.raw_ngmr_relate`.

SELECT 'row_count' AS metric, COUNT(*) AS value FROM "ducklake-canonical"."raw_ngmr_relate";
SELECT 'distinct_carrier' AS metric, COUNT(DISTINCT carrier_refno) AS value FROM "ducklake-canonical"."raw_ngmr_relate";
SELECT 'distinct_target' AS metric, COUNT(DISTINCT target_refno) AS value FROM "ducklake-canonical"."raw_ngmr_relate";
SELECT 'distinct_ngmr' AS metric, COUNT(DISTINCT ngmr_refno) AS value FROM "ducklake-canonical"."raw_ngmr_relate";

WITH ordered AS (
  SELECT carrier_refno, target_refno, ngmr_refno,
         ROW_NUMBER() OVER (ORDER BY carrier_refno, target_refno, ngmr_refno) AS rn,
         COUNT(*) OVER () AS total
  FROM "ducklake-canonical"."raw_ngmr_relate"
)
SELECT 'sample_first' AS marker, carrier_refno, target_refno, ngmr_refno FROM ordered WHERE rn = 1
UNION ALL
SELECT 'sample_mid', carrier_refno, target_refno, ngmr_refno FROM ordered WHERE rn = GREATEST(1, total / 2)
UNION ALL
SELECT 'sample_last', carrier_refno, target_refno, ngmr_refno FROM ordered WHERE rn = total;
