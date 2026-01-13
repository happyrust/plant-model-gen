use aios_core::SUL_DB;
use aios_core::SurrealQueryExt;
use polars::prelude::*;
use std::fs;
use std::path::Path;
use surrealdb::types as surrealdb_types;
use surrealdb::types::SurrealValue;

/// 按 dbnum 导出前端所需的模型数据 (refno, dbnum, noun, matrix, geo_hash)
pub async fn export_db_models_parquet(
    target_path: &Path,
    db_nums: Option<Vec<i64>>,
) -> anyhow::Result<()> {
    println!("[parquet] 开始导出前端模型数据, 目标路径: {}", target_path.display());

    let db_filter = if let Some(nums) = db_nums {
        if nums.is_empty() {
            "".to_string()
        } else {
            format!("AND in.dbnum IN [{}]", nums.iter().map(|n| n.to_string()).collect::<Vec<_>>().join(","))
        }
    } else {
        "".to_string()
    };

    // SQL 逻辑：
    // 1. 从 inst_relate 获取实例
    // 2. 联合 trans 获取世界变换矩阵 (16位)
    // 3. 联合 geo_relate -> inst_geo 获取几何体哈希
    // 我们需要一个扁平化的结构，如果一个实例有多个几何体，会产生多行（前端 loader 已支持这种格式）
    let sql = format!(r#"
SELECT
    <string>in as refno,
    in.dbnum as dbno,
    in.noun as noun,
    world_trans.d.matrix as matrix,
    out.out.id as geo_id,
    out.out.id as geo_hash,
    out.trans.d.matrix as geo_matrix
FROM inst_relate
WHERE world_trans != none 
  AND in.dbnum != none
  {db_filter}
"#);

    let rows: Vec<ModelDataRow> = match SUL_DB.query_take(&sql, 0).await {
        Ok(rows) => rows,
        Err(e) => {
            eprintln!("[parquet] 查询模型数据失败: {e:?}");
            return Err(anyhow::anyhow!("SQL query failed: {e}"));
        }
    };

    if rows.is_empty() {
        println!("[parquet] 未找到匹配的模型数据");
        return Ok(());
    }

    println!("[parquet] 查询到 {} 条原始数据，正在按 dbnum 分组...", rows.len());

    // 按 dbnum 分组
    let mut db_groups: std::collections::HashMap<i64, Vec<ModelDataRow>> = std::collections::HashMap::new();
    for row in rows {
        db_groups.entry(row.dbno).or_default().push(row);
    }

    fs::create_dir_all(target_path)?;

    for (dbno, group) in db_groups {
        let mut refnos = Vec::with_capacity(group.len());
        let mut nouns = Vec::with_capacity(group.len());
        let mut geo_hashes = Vec::with_capacity(group.len());
        
        // 矩阵导出为 16 个单独的列，或者扁平化的数组？
        // 前端 loader 看起来在查 t0...t15，或者期待矩阵在 row 中。
        // 这里我们采用前端 useParquetModelLoader.ts 兼容的格式。
        let mut t0 = Vec::with_capacity(group.len());
        let mut t1 = Vec::with_capacity(group.len());
        let mut t2 = Vec::with_capacity(group.len());
        let mut t3 = Vec::with_capacity(group.len());
        let mut t4 = Vec::with_capacity(group.len());
        let mut t5 = Vec::with_capacity(group.len());
        let mut t6 = Vec::with_capacity(group.len());
        let mut t7 = Vec::with_capacity(group.len());
        let mut t8 = Vec::with_capacity(group.len());
        let mut t9 = Vec::with_capacity(group.len());
        let mut t10 = Vec::with_capacity(group.len());
        let mut t11 = Vec::with_capacity(group.len());
        let mut t12 = Vec::with_capacity(group.len());
        let mut t13 = Vec::with_capacity(group.len());
        let mut t14 = Vec::with_capacity(group.len());
        let mut t15 = Vec::with_capacity(group.len());

        for row in group {
            refnos.push(row.refno);
            nouns.push(row.noun.unwrap_or_default());
            // 几何哈希去掉前缀 inst_geo:
            let hash = row.geo_hash.replace("inst_geo:⟨", "").replace("⟩", "");
            geo_hashes.push(hash);

            // 矩阵处理
            let m = row.matrix.unwrap_or_else(|| vec![1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0]);
            t0.push(m.get(0).copied().unwrap_or(0.0));
            t1.push(m.get(1).copied().unwrap_or(0.0));
            t2.push(m.get(2).copied().unwrap_or(0.0));
            t3.push(m.get(3).copied().unwrap_or(0.0));
            t4.push(m.get(4).copied().unwrap_or(0.0));
            t5.push(m.get(5).copied().unwrap_or(1.0));
            t6.push(m.get(6).copied().unwrap_or(0.0));
            t7.push(m.get(7).copied().unwrap_or(0.0));
            t8.push(m.get(8).copied().unwrap_or(0.0));
            t9.push(m.get(9).copied().unwrap_or(0.0));
            t10.push(m.get(10).copied().unwrap_or(1.0));
            t11.push(m.get(11).copied().unwrap_or(0.0));
            t12.push(m.get(12).copied().unwrap_or(0.0));
            t13.push(m.get(13).copied().unwrap_or(0.0));
            t14.push(m.get(14).copied().unwrap_or(0.0));
            t15.push(m.get(15).copied().unwrap_or(1.0));
        }

        let mut df = df![
            "refno" => refnos,
            "noun" => nouns,
            "geo_hash" => geo_hashes,
            "t0" => t0, "t1" => t1, "t2" => t2, "t3" => t3,
            "t4" => t4, "t5" => t5, "t6" => t6, "t7" => t7,
            "t8" => t8, "t9" => t9, "t10" => t10, "t11" => t11,
            "t12" => t12, "t13" => t13, "t14" => t14, "t15" => t15,
        ]?;

        let file_name = format!("db_models_{}.parquet", dbno);
        let file_path = target_path.join(&file_name);
        let file = fs::File::create(&file_path)?;
        ParquetWriter::new(file).finish(&mut df)?;
        
        println!("[parquet] 成功导出: {} ({} 条记录)", file_name, df.height());
    }

    Ok(())
}

#[derive(Debug, Clone, serde::Deserialize, SurrealValue)]
struct ModelDataRow {
    refno: String,
    dbno: i64,
    noun: Option<String>,
    matrix: Option<Vec<f64>>,
    geo_hash: String,
}
