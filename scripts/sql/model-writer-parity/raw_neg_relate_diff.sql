-- v3 Phase E: key-set diff for canonical raw table `raw_neg_relate`.
--
-- Key columns: carrier_refno, target_refno, geom_refno
-- Reports rows present only in LEFT (missing from RIGHT) and only in RIGHT
-- (extra in RIGHT). Empty result set => parity holds on the key surface.
--
-- Usage: replace the four SET VARIABLE placeholders, then run:
--   duckdb -c ".read scripts/sql/model-writer-parity/raw_neg_relate_diff.sql"

SET VARIABLE LEFT_ROOT  = 'output/run-baseline/model_writer_storage/raw';
SET VARIABLE RIGHT_ROOT = 'output/run-candidate/model_writer_storage/raw';
SET VARIABLE PROJECT    = 'project-x';
SET VARIABLE DBNUM      = 1;

WITH
    L AS (
        SELECT carrier_refno, target_refno, geom_refno
        FROM read_json_auto(
            getvariable('LEFT_ROOT') || '/raw_neg_relate/project_name=' || getvariable('PROJECT') || '/dbnum=' || getvariable('DBNUM') || '/batch_*.jsonl',
            auto_detect = TRUE,
            format      = 'newline_delimited'
        )
    ),
    R AS (
        SELECT carrier_refno, target_refno, geom_refno
        FROM read_json_auto(
            getvariable('RIGHT_ROOT') || '/raw_neg_relate/project_name=' || getvariable('PROJECT') || '/dbnum=' || getvariable('DBNUM') || '/batch_*.jsonl',
            auto_detect = TRUE,
            format      = 'newline_delimited'
        )
    ),
    only_left AS (
        SELECT carrier_refno, target_refno, geom_refno FROM L
        EXCEPT
        SELECT carrier_refno, target_refno, geom_refno FROM R
    ),
    only_right AS (
        SELECT carrier_refno, target_refno, geom_refno FROM R
        EXCEPT
        SELECT carrier_refno, target_refno, geom_refno FROM L
    )
SELECT 'only_left' AS side, carrier_refno, target_refno, geom_refno FROM only_left
UNION ALL
SELECT 'only_right' AS side, carrier_refno, target_refno, geom_refno FROM only_right
ORDER BY side, carrier_refno, target_refno, geom_refno;