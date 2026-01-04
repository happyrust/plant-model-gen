use aios_core::types::PlantAabb;
use aios_core::{SUL_DB, SurrealQueryExt};
use polars::prelude::*;
use std::fs;
use std::path::Path;
use surrealdb::types as surrealdb_types;
use surrealdb::types::SurrealValue;

/// 从 SurrealDB 导出 inst_relate_aabb + world_trans 到 Parquet，供空间计算使用
pub async fn export_inst_aabb_parquet(output_path: &Path) -> anyhow::Result<()> {
    // 仅查询未导出的记录，且包含 ID 以便后续更新标记
    let sql = r#"
SELECT
  id as row_id,
  <string>in as refno,
  in.noun as noun,
  in.dbnum as dbno,
  type::record('inst_relate_aabb', record::id(in)).aabb.d as aabb
FROM inst_relate
WHERE world_trans.d != none
  AND in.dbnum != none
  AND (exported_to_parquet != true)
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
        println!("[parquet] 没有新的 inst_relate_aabb 需要导出");
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

    let mut row_ids = Vec::with_capacity(rows.len());
    for row in &rows {
        row_ids.push(row.row_id.clone());
        refnos.push(row.refno.clone());
        dbnos.push(row.dbno);
        nouns.push(row.noun.clone().unwrap_or_default());
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

    // 如果文件已存在，则读取旧数据并合并
    let mut final_df = if file_path.exists() {
        let old_df = LazyFrame::scan_parquet(&file_path, Default::default())?.collect()?;
        concat([old_df.lazy(), df.lazy()], UnionArgs::default())?.collect()?
    } else {
        df
    };

    let file = std::fs::File::create(&file_path)?;
    ParquetWriter::new(file).finish(&mut final_df)?;

    // 导出成功后更新数据库标记位
    let update_sql = "UPDATE inst_relate SET exported_to_parquet = true WHERE id IN $ids";
    if let Err(e) = SUL_DB.query(update_sql).bind(("ids", row_ids)).await {
        eprintln!("[parquet] 更新标记位失败: {e:?}");
    }

    println!(
        "[parquet] 增量导出完成 -> {} (新增: {}, 总计: {})",
        file_path.display(),
        rows.len(),
        final_df.height()
    );
    Ok(())
}

#[derive(Debug, Clone, serde::Deserialize, SurrealValue)]
struct Row {
    row_id: surrealdb::types::RecordId,
    refno: String,
    dbno: i64,
    noun: Option<String>,
    aabb: PlantAabb,
}
