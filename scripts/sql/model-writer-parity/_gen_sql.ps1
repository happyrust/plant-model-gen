# Generator for v3 Phase E SQL parity scripts.
# Run once to (re-)materialize 26 SQL files (13 tables * 2 each).
# All generated files are checked into git; this script is for maintenance.

$root = $PSScriptRoot

$tables = @(
    @{ name = 'raw_inst_info';          keys = @('refno') },
    @{ name = 'raw_inst_relate';        keys = @('relation_id') },
    @{ name = 'raw_inst_geo';           keys = @('owner_refno', 'geo_hash') },
    @{ name = 'raw_geo_relate';         keys = @('relation_id') },
    @{ name = 'raw_tubi_info';          keys = @('tubi_id') },
    @{ name = 'raw_tubi_relate';        keys = @('refno', 'branch_id') },
    @{ name = 'raw_neg_relate';         keys = @('carrier_refno', 'target_refno', 'geom_refno') },
    @{ name = 'raw_ngmr_relate';        keys = @('carrier_refno', 'target_refno', 'geom_refno') },
    @{ name = 'raw_aabb';               keys = @('aabb_id') },
    @{ name = 'raw_trans';              keys = @('trans_id') },
    @{ name = 'raw_vec3';               keys = @('vec3_id') },
    @{ name = 'raw_inst_relate_aabb';   keys = @('relation_id') },
    @{ name = 'raw_refno_assoc_index';  keys = @('refno') }
)

foreach ($t in $tables) {
    $name = $t.name
    $keys = $t.keys
    $key_csv = $keys -join ', '

    $count_path = Join-Path $root "${name}_count.sql"
    $count_sql = @"
-- v3 Phase E: row count for canonical raw table ``$name``.
--
-- Usage: replace the four SET VARIABLE placeholders, then run:
--   duckdb -c ".read scripts/sql/model-writer-parity/${name}_count.sql"

SET VARIABLE LEFT_ROOT  = 'output/run-baseline/model_writer_storage/raw';
SET VARIABLE RIGHT_ROOT = 'output/run-candidate/model_writer_storage/raw';
SET VARIABLE PROJECT    = 'project-x';
SET VARIABLE DBNUM      = 1;

SELECT
    'left'  AS side,
    COUNT(*) AS rows
FROM read_json_auto(
    getvariable('LEFT_ROOT') || '/${name}/project_name=' || getvariable('PROJECT') || '/dbnum=' || getvariable('DBNUM') || '/batch_*.jsonl',
    auto_detect = TRUE,
    format      = 'newline_delimited'
)
UNION ALL
SELECT
    'right' AS side,
    COUNT(*) AS rows
FROM read_json_auto(
    getvariable('RIGHT_ROOT') || '/${name}/project_name=' || getvariable('PROJECT') || '/dbnum=' || getvariable('DBNUM') || '/batch_*.jsonl',
    auto_detect = TRUE,
    format      = 'newline_delimited'
);
"@
    Set-Content -Path $count_path -Value $count_sql -Encoding UTF8 -NoNewline

    $diff_path = Join-Path $root "${name}_diff.sql"
    $diff_sql = @"
-- v3 Phase E: key-set diff for canonical raw table ``$name``.
--
-- Key columns: $key_csv
-- Reports rows present only in LEFT (missing from RIGHT) and only in RIGHT
-- (extra in RIGHT). Empty result set => parity holds on the key surface.
--
-- Usage: replace the four SET VARIABLE placeholders, then run:
--   duckdb -c ".read scripts/sql/model-writer-parity/${name}_diff.sql"

SET VARIABLE LEFT_ROOT  = 'output/run-baseline/model_writer_storage/raw';
SET VARIABLE RIGHT_ROOT = 'output/run-candidate/model_writer_storage/raw';
SET VARIABLE PROJECT    = 'project-x';
SET VARIABLE DBNUM      = 1;

WITH
    L AS (
        SELECT $key_csv
        FROM read_json_auto(
            getvariable('LEFT_ROOT') || '/${name}/project_name=' || getvariable('PROJECT') || '/dbnum=' || getvariable('DBNUM') || '/batch_*.jsonl',
            auto_detect = TRUE,
            format      = 'newline_delimited'
        )
    ),
    R AS (
        SELECT $key_csv
        FROM read_json_auto(
            getvariable('RIGHT_ROOT') || '/${name}/project_name=' || getvariable('PROJECT') || '/dbnum=' || getvariable('DBNUM') || '/batch_*.jsonl',
            auto_detect = TRUE,
            format      = 'newline_delimited'
        )
    ),
    only_left AS (
        SELECT $key_csv FROM L
        EXCEPT
        SELECT $key_csv FROM R
    ),
    only_right AS (
        SELECT $key_csv FROM R
        EXCEPT
        SELECT $key_csv FROM L
    )
SELECT 'only_left' AS side, $key_csv FROM only_left
UNION ALL
SELECT 'only_right' AS side, $key_csv FROM only_right
ORDER BY side, $key_csv;
"@
    Set-Content -Path $diff_path -Value $diff_sql -Encoding UTF8 -NoNewline

    Write-Host "wrote $count_path"
    Write-Host "wrote $diff_path"
}
