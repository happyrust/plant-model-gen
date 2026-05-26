// SPDX-License-Identifier: MIT
//
// DuckLake parity runner (Slice 6 c of goals/ducklake-model-writer/).
//
// Opens the freshly generated DuckLake metadata + INSTALL/LOAD ducklake +
// ATTACH 'ducklake:metadata.ducklake' AS lake, then runs the row count +
// primary key set + 3 sample queries for each of the 9 in-scope raw tables.
// Emits one JSON line per table to stdout and (optionally) appends each line
// to `goals/ducklake-model-writer/progress.jsonl`.
//
// Requires feature `model-writer-ducklake` so the duckdb crate is linked.

#![cfg(feature = "model-writer-ducklake")]

use std::fs::OpenOptions;
use std::io::Write;
use std::path::PathBuf;

use clap::{Arg, ArgAction, Command};
use duckdb::Connection;
use serde::Serialize;

const DUCKLAKE_CANONICAL_SCHEMA: &str = "ducklake-canonical";

const IN_SCOPE_TABLES: [&str; 9] = [
    "raw_inst_info",
    "raw_inst_relate",
    "raw_inst_geo",
    "raw_geo_relate",
    "raw_neg_relate",
    "raw_ngmr_relate",
    "raw_aabb",
    "raw_vec3",
    "raw_inst_relate_aabb",
];

#[derive(Serialize)]
struct TableParity {
    ts: String,
    event: &'static str,
    table: &'static str,
    row_count: i64,
    pk_distinct: i64,
    extra: serde_json::Value,
    samples: Vec<serde_json::Value>,
}

fn now_iso() -> String {
    // Lightweight ISO8601 with +08:00 offset (project convention from
    // model-writer-backend-abstraction/progress.jsonl).
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    // Crude conversion; precision isn't critical for evidence ts.
    // (chrono is in deps but pulling it here is overkill; format manually.)
    let secs_in_day = 86_400u64;
    let day = now / secs_in_day;
    let rem = now % secs_in_day;
    let h = (rem / 3_600) as u32;
    let m = ((rem % 3_600) / 60) as u32;
    let s = (rem % 60) as u32;
    // Shift to +08:00 by adding 8h.
    let h_local = (h + 8) % 24;
    format!(
        "1970-01-{:02}T{:02}:{:02}:{:02}+08:00",
        (day % 30) + 1,
        h_local,
        m,
        s
    )
}

fn open_ducklake(root_dir: &PathBuf, catalog: &str) -> anyhow::Result<Connection> {
    let metadata_path = root_dir.join("metadata.ducklake");
    let data_dir = root_dir.join("data");

    let conn = Connection::open_in_memory()
        .map_err(|e| anyhow::anyhow!("ducklake_parity: open in-memory duckdb failed: {e}"))?;
    conn.execute_batch("INSTALL ducklake; LOAD ducklake;")
        .map_err(|e| anyhow::anyhow!("ducklake_parity: INSTALL/LOAD ducklake failed: {e}"))?;
    let metadata_uri = format!(
        "ducklake:{}",
        metadata_path.to_string_lossy().replace('\\', "/")
    );
    let data_path = data_dir.to_string_lossy().replace('\\', "/");
    let attach_sql = format!(
        "ATTACH '{}' AS {} (DATA_PATH '{}')",
        metadata_uri, catalog, data_path
    );
    conn.execute_batch(&attach_sql)
        .map_err(|e| anyhow::anyhow!("ducklake_parity: ATTACH failed: {e}"))?;
    conn.execute_batch(&format!("USE {};", catalog))
        .map_err(|e| anyhow::anyhow!("ducklake_parity: USE {} failed: {e}", catalog))?;
    Ok(conn)
}

fn run_table_parity(conn: &Connection, table: &'static str) -> anyhow::Result<TableParity> {
    let qual = format!("\"{}\".\"{}\"", DUCKLAKE_CANONICAL_SCHEMA, table);

    let row_count: i64 = conn
        .query_row(&format!("SELECT COUNT(*) FROM {}", qual), [], |r| r.get(0))
        .map_err(|e| anyhow::anyhow!("table={table} row_count query failed: {e}"))?;

    // Per-table primary key column(s).
    let (pk_expr, sample_select, extra): (&str, &str, serde_json::Value) = match table {
        "raw_inst_info" => ("inst_id", "inst_id", serde_json::json!({})),
        "raw_inst_relate" => (
            "refno || '/' || inst_id",
            "refno, inst_id",
            serde_json::json!({}),
        ),
        "raw_inst_geo" => {
            let meshed: i64 = conn
                .query_row(
                    &format!("SELECT COUNT(*) FROM {} WHERE meshed IS TRUE", qual),
                    [],
                    |r| r.get(0),
                )
                .unwrap_or(0);
            let bad: i64 = conn
                .query_row(
                    &format!("SELECT COUNT(*) FROM {} WHERE bad IS TRUE", qual),
                    [],
                    |r| r.get(0),
                )
                .unwrap_or(0);
            (
                "geo_hash",
                "geo_hash",
                serde_json::json!({"meshed_rows": meshed, "bad_rows": bad}),
            )
        }
        "raw_geo_relate" => {
            let tubi: i64 = conn
                .query_row(
                    &format!("SELECT COUNT(*) FROM {} WHERE is_tubi IS TRUE", qual),
                    [],
                    |r| r.get(0),
                )
                .unwrap_or(0);
            (
                "inst_id || '/' || geo_hash || '/' || idx",
                "inst_id, geo_hash, idx",
                serde_json::json!({"tubi_rows": tubi}),
            )
        }
        "raw_neg_relate" => {
            let sentinel: i64 = conn
                .query_row(
                    &format!(
                        "SELECT COUNT(*) FROM {} WHERE target_refno = '__reconcile_pending__'",
                        qual
                    ),
                    [],
                    |r| r.get(0),
                )
                .unwrap_or(0);
            let real: i64 = conn
                .query_row(
                    &format!(
                        "SELECT COUNT(*) FROM {} WHERE target_refno <> '__reconcile_pending__'",
                        qual
                    ),
                    [],
                    |r| r.get(0),
                )
                .unwrap_or(0);
            (
                "carrier_refno || '/' || target_refno",
                "carrier_refno, target_refno",
                serde_json::json!({"sentinel_reconcile_rows": sentinel, "real_rows": real}),
            )
        }
        "raw_ngmr_relate" => (
            "carrier_refno || '/' || target_refno || '/' || ngmr_refno",
            "carrier_refno, target_refno, ngmr_refno",
            serde_json::json!({}),
        ),
        "raw_aabb" => {
            let min_x: f64 = conn
                .query_row(&format!("SELECT MIN(min_x) FROM {}", qual), [], |r| {
                    r.get(0)
                })
                .unwrap_or(0.0);
            let max_x: f64 = conn
                .query_row(&format!("SELECT MAX(max_x) FROM {}", qual), [], |r| {
                    r.get(0)
                })
                .unwrap_or(0.0);
            (
                "aabb_id",
                "aabb_id",
                serde_json::json!({"min_x_min": min_x, "max_x_max": max_x}),
            )
        }
        "raw_vec3" => ("vec3_id", "vec3_id", serde_json::json!({})),
        "raw_inst_relate_aabb" => (
            "refno || '/' || aabb_id",
            "refno, aabb_id, source",
            serde_json::json!({}),
        ),
        _ => unreachable!(),
    };

    let pk_distinct: i64 = conn
        .query_row(
            &format!("SELECT COUNT(DISTINCT {}) FROM {}", pk_expr, qual),
            [],
            |r| r.get(0),
        )
        .unwrap_or(-1);

    // 3 samples at first / mid / last.
    let sample_sql = format!(
        "WITH ordered AS (
           SELECT {sample_select}, ROW_NUMBER() OVER (ORDER BY {pk_expr}) AS rn,
                  COUNT(*) OVER () AS total
           FROM {qual}
         )
         SELECT 'sample_first' AS marker, {sample_select} FROM ordered WHERE rn = 1
         UNION ALL
         SELECT 'sample_mid', {sample_select} FROM ordered WHERE rn = GREATEST(1, total / 2)
         UNION ALL
         SELECT 'sample_last', {sample_select} FROM ordered WHERE rn = total"
    );
    let mut stmt = conn
        .prepare(&sample_sql)
        .map_err(|e| anyhow::anyhow!("table={table} sample query prepare failed: {e}"))?;
    let mut rows = stmt
        .query([])
        .map_err(|e| anyhow::anyhow!("table={table} sample query exec failed: {e}"))?;
    let mut samples = Vec::new();
    while let Ok(Some(row)) = rows.next() {
        // Collect every column into a serde_json::Value array.
        let col_count = row.as_ref().column_count();
        let mut row_vals: Vec<serde_json::Value> = Vec::with_capacity(col_count);
        for i in 0..col_count {
            // Best-effort: try string first, fall back to int/float.
            let v: Option<String> = row.get(i).ok();
            match v {
                Some(s) => row_vals.push(serde_json::Value::String(s)),
                None => {
                    let n: Option<i64> = row.get(i).ok();
                    match n {
                        Some(n) => row_vals.push(serde_json::Value::Number(n.into())),
                        None => {
                            let f: Option<f64> = row.get(i).ok();
                            match f.and_then(serde_json::Number::from_f64) {
                                Some(n) => row_vals.push(serde_json::Value::Number(n)),
                                None => row_vals.push(serde_json::Value::Null),
                            }
                        }
                    }
                }
            }
        }
        samples.push(serde_json::Value::Array(row_vals));
    }

    Ok(TableParity {
        ts: now_iso(),
        event: "slice_6_c_parity",
        table,
        row_count,
        pk_distinct,
        extra,
        samples,
    })
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> anyhow::Result<()> {
    let matches = Command::new("ducklake-parity")
        .about("Slice 6 c: DuckLake-side parity counts for 9 in-scope raw tables")
        .arg(
            Arg::new("root")
                .long("root")
                .default_value("output/model_writer_storage/ducklake")
                .help("DuckLake root directory containing metadata.ducklake + data/"),
        )
        .arg(
            Arg::new("catalog")
                .long("catalog")
                .default_value("lake")
                .help("DuckLake ATTACH alias"),
        )
        .arg(
            Arg::new("append-progress")
                .long("append-progress")
                .action(ArgAction::SetTrue)
                .help("Append every per-table JSON line to goals/ducklake-model-writer/progress.jsonl"),
        )
        .get_matches();

    let root: PathBuf = matches.get_one::<String>("root").unwrap().into();
    let catalog: &str = matches.get_one::<String>("catalog").unwrap();
    let append = matches.get_flag("append-progress");

    if !root.join("metadata.ducklake").is_file() {
        anyhow::bail!(
            "ducklake_parity: metadata.ducklake not found under {} \u{2014} run a ducklake generation first \
             (e.g. `cargo run --bin aios-database --features review,model-writer-drain,model-writer-ducklake \
             -- -c db_options/DbOption-cli --regen-model --dbnum 7997 --model-writer ducklake`)",
            root.display()
        );
    }

    let conn = open_ducklake(&root, catalog)?;
    let progress_path = PathBuf::from("goals/ducklake-model-writer/progress.jsonl");
    let mut progress_file = if append {
        Some(
            OpenOptions::new()
                .append(true)
                .create(true)
                .open(&progress_path)
                .map_err(|e| {
                    anyhow::anyhow!(
                        "ducklake_parity: failed to open progress.jsonl {} for append: {e}",
                        progress_path.display()
                    )
                })?,
        )
    } else {
        None
    };

    for table in IN_SCOPE_TABLES {
        let parity = run_table_parity(&conn, table)?;
        let line = serde_json::to_string(&parity)?;
        println!("{}", line);
        if let Some(f) = progress_file.as_mut() {
            writeln!(f, "{}", line).map_err(|e| {
                anyhow::anyhow!("ducklake_parity: write progress.jsonl failed: {e}")
            })?;
        }
    }

    Ok(())
}
