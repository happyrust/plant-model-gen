//! spec_info 表：按 IndexTree SITE 层级遍历，只导出 BRAN/HANG/EQUI/WALL/FLOOR 最小交付单元的专业信息

use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;

use aios_core::tool::db_tool::db1_dehash;
use aios_core::tree_query::{TreeQueryFilter, TreeQueryOptions};
use aios_core::{RefU64, RefnoEnum, SUL_DB, SurrealQueryExt};
use anyhow::{Context, Result};
use arrow_array::{ArrayRef, Int64Array, RecordBatch, StringArray, UInt32Array, UInt64Array};
use arrow_schema::{DataType, Field, Schema};
use parquet::arrow::ArrowWriter;
use parquet::basic::Compression;
use parquet::file::properties::WriterProperties;

use crate::fast_model::gen_model::tree_index_manager::load_index_with_large_stack;

/// 最小交付单元 noun 类型
const DELIVERY_UNIT_NOUNS: &[&str] = &["BRAN", "HANG", "EQUI", "WALL", "FLOOR"];

/// SITE name -> spec_value 映射（与 fn::init_site_spec_value 一致）
fn site_name_to_spec_value(name: &str) -> i64 {
    let name = name.to_uppercase();
    if name.contains("PIPE") {
        1
    } else if name.contains("ELEC") {
        2
    } else if name.contains("INST") {
        3
    } else if name.contains("HVAC") {
        4
    } else {
        0
    }
}

/// 构建 spec_info 并写出 Parquet，返回 refno -> spec_value 映射供导出使用
pub async fn build_spec_info_parquet(
    dbnum: u32,
    tree_dir: &Path,
    output_path: &Path,
    verbose: bool,
) -> Result<HashMap<u64, i64>> {
    let index = load_index_with_large_stack(tree_dir, dbnum)
        .with_context(|| format!("加载 TreeIndex dbnum={} 失败", dbnum))?;

    let all_count = index.all_refnos().len();
    let site_refnos: Vec<RefU64> = index
        .all_refnos()
        .into_iter()
        .filter_map(|r| {
            index.node_meta(r).and_then(|m| {
                if m.noun == aios_core::tool::db_tool::db1_hash("SITE") {
                    Some(r)
                } else {
                    None
                }
            })
        })
        .collect();

    if verbose {
        println!(
            "   📋 spec_info: TreeIndex {} 节点, SITE {} 个",
            all_count,
            site_refnos.len()
        );
    }

    if site_refnos.is_empty() {
        if verbose {
            println!("   ⚠️ 未找到 SITE 节点，spec_info 为空");
        }
        return Ok(HashMap::new());
    }

    // 方案 B：批量查询 pe 仅取 name，SELECT value name FROM [pe:⟨...⟩, ...]
    // 按顺序与 site_refnos 对应，用 site_name_to_spec_value 推断专业
    let mut site_spec_map: HashMap<u64, i64> = HashMap::new();
    let batch_size = 500;
    for chunk in site_refnos.chunks(batch_size) {
        let pe_keys: Vec<String> = chunk
            .iter()
            .map(|r| format!("pe:⟨{}⟩", r.to_string()))
            .collect();
        let keys_joined = pe_keys.join(", ");
        let query = format!(r#"SELECT value name FROM [{}]"#, keys_joined);
        let names: Vec<Option<String>> = SUL_DB.query_take::<Vec<Option<String>>>(&query, 0).await?;
        for (i, r) in chunk.iter().enumerate() {
            let name = names.get(i).and_then(|o| o.as_deref()).unwrap_or("");
            site_spec_map.insert(r.0, site_name_to_spec_value(name));
        }
    }

    // 层级遍历：每个 SITE 向下收集 BRAN/HANG/EQUI/WALL/FLOOR
    let noun_hashes: std::collections::HashSet<u32> =
        DELIVERY_UNIT_NOUNS.iter().map(|n| aios_core::tool::db_tool::db1_hash(n)).collect();

    let mut spec_map: HashMap<u64, i64> = HashMap::new();
    let mut rows: Vec<(String, u64, String, i64, u32)> = Vec::new();

    for site_refno in &site_refnos {
        let site_u64 = site_refno.0;
        let spec_value = *site_spec_map.get(&site_u64).unwrap_or(&0);

        let options = TreeQueryOptions {
            include_self: false,
            max_depth: None,
            filter: TreeQueryFilter {
                noun_hashes: Some(noun_hashes.clone()),
                ..Default::default()
            },
            prune_on_match: false,
        };

        let grouped = index.collect_descendants_bfs_grouped(*site_refno, &options);
        for (noun_hash, refnos) in grouped {
            let noun = db1_dehash(noun_hash);
            for r in refnos {
                let u = r.0;
                if spec_map.insert(u, spec_value).is_none() {
                    let refno_str = RefnoEnum::from(r).to_string();
                    rows.push((refno_str, u, noun.to_string(), spec_value, dbnum));
                }
            }
        }
    }

    if verbose {
        println!(
            "   📋 spec_info: 收集到 {} 个 BRAN/HANG/EQUI/WALL/FLOOR, site_spec_map {} 条",
            rows.len(),
            site_spec_map.len()
        );
    }

    if rows.is_empty() {
        if verbose {
            println!("   ⚠️ 无 BRAN/HANG/EQUI/WALL/FLOOR 节点，spec_info 为空");
        }
        return Ok(spec_map);
    }

    // 写出 Parquet
    let schema = Schema::new(vec![
        Field::new("refno_str", DataType::Utf8, false),
        Field::new("refno_u64", DataType::UInt64, false),
        Field::new("noun", DataType::Utf8, false),
        Field::new("spec_value", DataType::Int64, false),
        Field::new("dbnum", DataType::UInt32, false),
    ]);

    let refno_str_arr: Vec<&str> = rows.iter().map(|r| r.0.as_str()).collect();
    let refno_u64_arr: Vec<u64> = rows.iter().map(|r| r.1).collect();
    let noun_arr: Vec<&str> = rows.iter().map(|r| r.2.as_str()).collect();
    let spec_value_arr: Vec<i64> = rows.iter().map(|r| r.3).collect();
    let dbnum_arr: Vec<u32> = rows.iter().map(|r| r.4).collect();

    let batch = RecordBatch::try_new(
        Arc::new(schema.clone()),
        vec![
            Arc::new(StringArray::from(refno_str_arr)) as ArrayRef,
            Arc::new(UInt64Array::from(refno_u64_arr)) as ArrayRef,
            Arc::new(StringArray::from(noun_arr)) as ArrayRef,
            Arc::new(Int64Array::from(spec_value_arr)) as ArrayRef,
            Arc::new(UInt32Array::from(dbnum_arr)) as ArrayRef,
        ],
    )?;

    let props = WriterProperties::builder()
        .set_compression(Compression::ZSTD(
            parquet::basic::ZstdLevel::try_new(3).unwrap(),
        ))
        .build();
    let file = std::fs::File::create(output_path)
        .with_context(|| format!("创建 spec_info 文件失败: {}", output_path.display()))?;
    let mut writer = ArrowWriter::try_new(file, batch.schema(), Some(props))?;
    writer.write(&batch)?;
    writer.close()?;

    if verbose {
        println!(
            "   ✅ spec_info 已写出: {} 行 -> {}",
            rows.len(),
            output_path.display()
        );
    }

    Ok(spec_map)
}

/// 若 spec_info 文件已存在则加载，否则构建并保存；返回 refno -> spec_value
pub async fn load_or_build_spec_info(
    dbnum: u32,
    tree_dir: &Path,
    output_dir: &Path,
    verbose: bool,
) -> Result<HashMap<u64, i64>> {
    let spec_path = output_dir.join(format!("spec_info_{}.parquet", dbnum));
    if spec_path.exists() {
        // 从 Parquet 加载
        return load_spec_info_from_parquet(&spec_path).await;
    }
    // 构建并保存；output_dir 可能是 parquet 输出目录，spec_info 与 instances 同目录
    build_spec_info_parquet(dbnum, tree_dir, &spec_path, verbose).await
}

/// 从 Parquet 加载 spec_info，返回 refno_u64 -> spec_value
async fn load_spec_info_from_parquet(path: &Path) -> Result<HashMap<u64, i64>> {
    use std::fs::File;

    // 用 parquet 读
    let file = File::open(path)
        .with_context(|| format!("打开 spec_info 失败: {}", path.display()))?;
    let reader = parquet::arrow::arrow_reader::ParquetRecordBatchReaderBuilder::try_new(file)?
        .build()?;

    let mut map = HashMap::new();
    for batch in reader {
        let batch = batch?;
        let refno_col = batch
            .column_by_name("refno_u64")
            .and_then(|c| c.as_any().downcast_ref::<UInt64Array>());
        let spec_col = batch
            .column_by_name("spec_value")
            .and_then(|c| c.as_any().downcast_ref::<Int64Array>());

        if let (Some(refno_arr), Some(spec_arr)) = (refno_col, spec_col) {
            for i in 0..refno_arr.len() {
                let r = refno_arr.value(i);
                let s = spec_arr.value(i);
                map.insert(r, s);
            }
        }
    }
    Ok(map)
}
