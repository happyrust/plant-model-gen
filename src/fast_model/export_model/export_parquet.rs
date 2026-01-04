use aios_core::types::PlantAabb;
use aios_core::{SUL_DB, SurrealQueryExt};
use polars::prelude::*;
use std::fs;
use std::path::Path;
use surrealdb::types as surrealdb_types;
use surrealdb::types::SurrealValue;

/// 从 SurrealDB 导出 inst_relate_aabb + world_trans 到 Parquet，供空间计算使用
pub async fn export_inst_aabb_parquet(output_path: &Path) -> anyhow::Result<()> {
    let sql = r#"
SELECT
  <string>in as refno,
  in.noun as noun,
  in.dbnum as dbno,
  type::record('inst_relate_aabb', record::id(in)).aabb.d as aabb
FROM inst_relate
WHERE world_trans.d != none
  AND in.dbnum != none
  AND record::exists(type::record('inst_relate_aabb', record::id(in)))
"#;

    let rows: Vec<Row> = match SUL_DB.query_take(sql, 0).await {
        Ok(rows) => rows,
        Err(e) => {
            eprintln!("[parquet] 查询 inst_relate_aabb 失败: {e:?}");
            return Ok(());
        }
    };
    if rows.is_empty() {
        println!("[parquet] inst_relate_aabb 查询为空，跳过导出");
        return Ok(());
    }

    let out_dir = output_path
        .parent()
        .map(|p| p.to_path_buf())
        .unwrap_or_else(|| Path::new("assets/parquet").to_path_buf());
    fs::create_dir_all(&out_dir)?;

    let mut refnos = Vec::with_capacity(rows.len());
    let mut dbnos = Vec::with_capacity(rows.len());
    let mut nouns = Vec::with_capacity(rows.len());
    let mut min_x = Vec::with_capacity(rows.len());
    let mut max_x = Vec::with_capacity(rows.len());
    let mut min_y = Vec::with_capacity(rows.len());
    let mut max_y = Vec::with_capacity(rows.len());
    let mut min_z = Vec::with_capacity(rows.len());
    let mut max_z = Vec::with_capacity(rows.len());

    for row in rows {
        refnos.push(row.refno);
        dbnos.push(row.dbno);
        nouns.push(row.noun.unwrap_or_default());
        let mins = row.aabb.mins();
        let maxs = row.aabb.maxs();
        min_x.push(mins.x as f64);
        max_x.push(maxs.x as f64);
        min_y.push(mins.y as f64);
        max_y.push(maxs.y as f64);
        min_z.push(mins.z as f64);
        max_z.push(maxs.z as f64);
    }

    let mut df = df![
        "refno" => refnos,
        "dbno" => dbnos,
        "noun" => nouns,
        "min_x" => min_x,
        "max_x" => max_x,
        "min_y" => min_y,
        "max_y" => max_y,
        "min_z" => min_z,
        "max_z" => max_z,
    ]?;

    let file_path = out_dir.join(
        output_path
            .file_name()
            .unwrap_or_else(|| std::ffi::OsStr::new("inst_aabb.parquet")),
    );
    let file = std::fs::File::create(&file_path)?;
    ParquetWriter::new(file).finish(&mut df)?;

    println!(
        "[parquet] 导出 inst_relate_aabb -> {} (rows={})",
        file_path.display(),
        df.height()
    );
    Ok(())
}

#[derive(Debug, Clone, serde::Deserialize, SurrealValue)]
struct Row {
    refno: String,
    dbno: i64,
    noun: Option<String>,
    aabb: PlantAabb,
}
