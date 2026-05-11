-- v3 Phase E: row count for canonical raw table `raw_tubi_relate`.
--
-- Usage: replace the four SET VARIABLE placeholders, then run:
--   duckdb -c ".read scripts/sql/model-writer-parity/raw_tubi_relate_count.sql"

SET VARIABLE LEFT_ROOT  = 'output/run-baseline/model_writer_storage/raw';
SET VARIABLE RIGHT_ROOT = 'output/run-candidate/model_writer_storage/raw';
SET VARIABLE PROJECT    = 'project-x';
SET VARIABLE DBNUM      = 1;

SELECT
    'left'  AS side,
    COUNT(*) AS rows
FROM read_json_auto(
    getvariable('LEFT_ROOT') || '/raw_tubi_relate/project_name=' || getvariable('PROJECT') || '/dbnum=' || getvariable('DBNUM') || '/batch_*.jsonl',
    auto_detect = TRUE,
    format      = 'newline_delimited'
)
UNION ALL
SELECT
    'right' AS side,
    COUNT(*) AS rows
FROM read_json_auto(
    getvariable('RIGHT_ROOT') || '/raw_tubi_relate/project_name=' || getvariable('PROJECT') || '/dbnum=' || getvariable('DBNUM') || '/batch_*.jsonl',
    auto_detect = TRUE,
    format      = 'newline_delimited'
);