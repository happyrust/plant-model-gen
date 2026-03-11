//! dbnum 级实例导出 Parquet
//!
//! 从 SurrealDB 读取 inst_relate / geo_relate / tubi_relate / trans / aabb 数据，
//! 生成多表 Parquet 文件组，供前端直接查询。
//!
//! 输出表（按 dbnum 分目录，文件名固定）：
//! - `instances.parquet`     — 一行一个实例 refno
//! - `geo_instances.parquet` — 一行一个几何引用 (refno × geo_index)
//! - `tubings.parquet`       — 一行一个 TUBI 段
//! - `transforms.parquet`    — 一行一个唯一 trans_hash
//! - `aabb.parquet`          — 一行一个唯一 aabb_hash
//! - `manifest.json`         — 元信息

use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use aios_core::options::DbOption;
use aios_core::pdms_types::RefnoEnum;
use aios_core::SurrealQueryExt;
use anyhow::{Context, Result};
use arrow_array::{
    ArrayRef, BooleanArray, Float64Array, RecordBatch, StringArray, UInt32Array, UInt64Array,
};
use arrow_schema::{DataType, Field, Schema};
use chrono::{SecondsFormat, Utc};
use parquet::arrow::ArrowWriter;
use parquet::basic::Compression;
use parquet::file::properties::WriterProperties;
use serde_json::json;

// 注: trans/aabb 查询在本模块内自行实现（避免跨模块耦合）
use crate::fast_model::gen_model::tree_index_manager::{
    load_index_with_large_stack, TreeIndexManager,
};
use crate::fast_model::unit_converter::{LengthUnit, UnitConverter};

// =============================================================================
// Parquet 行结构体
// =============================================================================

/// instances.parquet 的一行
struct InstanceRow {
    refno_str: String,
    refno_u64: u64,
    noun: String,
    owner_refno_str: Option<String>,
    owner_refno_u64: Option<u64>,
    owner_noun: String,
    trans_hash: String,
    aabb_hash: String,
    spec_value: i64,
    has_neg: bool,
    dbnum: u32,
}

/// geo_instances.parquet 的一行
struct GeoInstanceRow {
    refno_str: String,
    refno_u64: u64,
    geo_index: u32,
    geo_hash: String,
    geo_trans_hash: String,
}

/// tubings.parquet 的一行
struct TubingRow {
    tubi_refno_str: String,
    tubi_refno_u64: u64,
    owner_refno_str: String,
    owner_refno_u64: u64,
    order: u32,
    geo_hash: String,
    trans_hash: String,
    aabb_hash: String,
    spec_value: i64,
    dbnum: u32,
}

/// transforms.parquet 的一行
struct TransformRow {
    trans_hash: String,
    m00: f64, m10: f64, m20: f64, m30: f64,
    m01: f64, m11: f64, m21: f64, m31: f64,
    m02: f64, m12: f64, m22: f64, m32: f64,
    m03: f64, m13: f64, m23: f64, m33: f64,
}

/// aabb.parquet 的一行
struct AabbRow {
    aabb_hash: String,
    min_x: f64,
    min_y: f64,
    min_z: f64,
    max_x: f64,
    max_y: f64,
    max_z: f64,
}

// =============================================================================
// 辅助函数
// =============================================================================

fn refno_to_u64(r: &RefnoEnum) -> u64 {
    *r.refno()
}

fn writer_props() -> WriterProperties {
    WriterProperties::builder()
        .set_compression(Compression::ZSTD(
            parquet::basic::ZstdLevel::try_new(3).unwrap(),
        ))
        .build()
}

fn write_parquet(path: &Path, batch: &RecordBatch) -> Result<u64> {
    let file = fs::File::create(path)
        .with_context(|| format!("创建 Parquet 文件失败: {}", path.display()))?;
    let mut writer = ArrowWriter::try_new(file, batch.schema(), Some(writer_props()))?;
    writer.write(batch)?;
    writer.close()?;
    let size = fs::metadata(path).map(|m| m.len()).unwrap_or(0);
    Ok(size)
}

const MESH_CHECK_LOD_TAG: &str = "L1";
const MESH_REPORT_REFNO_SAMPLE_LIMIT: usize = 50;

struct MissingMeshReportSummary {
    report_file: String,
    checked_geo_hashes: usize,
    missing_geo_hashes: usize,
    missing_owner_refnos: usize,
}

fn mesh_base_dir_from_db_option(db_option: &DbOption) -> PathBuf {
    db_option
        .meshes_path
        .as_ref()
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("assets/meshes"))
}

fn normalize_mesh_base_dir(mesh_dir: &Path) -> PathBuf {
    let is_lod_dir = mesh_dir
        .file_name()
        .map(|n| n.to_string_lossy().starts_with("lod_"))
        .unwrap_or(false);
    if is_lod_dir {
        mesh_dir.parent().unwrap_or(mesh_dir).to_path_buf()
    } else {
        mesh_dir.to_path_buf()
    }
}

fn mesh_candidates_for_geo_hash(mesh_base_dir: &Path, geo_hash: &str, lod_tag: &str) -> [PathBuf; 3] {
    let lod_dir = mesh_base_dir.join(format!("lod_{}", lod_tag));
    [
        lod_dir.join(format!("{}_{}.glb", geo_hash, lod_tag)),
        lod_dir.join(format!("{}.glb", geo_hash)),
        mesh_base_dir.join(format!("{}.glb", geo_hash)),
    ]
}

fn is_builtin_geo_hash(geo_hash: &str) -> bool {
    matches!(geo_hash.trim(), "1" | "2" | "3")
}

fn record_geo_hash_usage(
    geo_hash: &str,
    owner_refno: &str,
    owner_refnos_by_hash: &mut HashMap<String, HashSet<String>>,
    row_count_by_hash: &mut HashMap<String, usize>,
) {
    let hash = geo_hash.trim();
    if hash.is_empty() || owner_refno.trim().is_empty() {
        return;
    }
    owner_refnos_by_hash
        .entry(hash.to_string())
        .or_default()
        .insert(owner_refno.to_string());
    *row_count_by_hash.entry(hash.to_string()).or_insert(0) += 1;
}

fn write_missing_mesh_report(
    output_dir: &Path,
    dbnum: u32,
    mesh_base_dir: &Path,
    lod_tag: &str,
    owner_refnos_by_hash: &HashMap<String, HashSet<String>>,
    row_count_by_hash: &HashMap<String, usize>,
    verbose: bool,
) -> Result<MissingMeshReportSummary> {
    let mut checked_geo_hashes = 0usize;
    let mut missing_owner_union: HashSet<String> = HashSet::new();
    let mut missing_entries: Vec<(String, usize, usize, Vec<String>, Vec<String>)> = Vec::new();

    for geo_hash in owner_refnos_by_hash.keys() {
        let hash = geo_hash.trim();
        if hash.is_empty() || is_builtin_geo_hash(hash) {
            continue;
        }
        checked_geo_hashes += 1;

        let candidates = mesh_candidates_for_geo_hash(mesh_base_dir, hash, lod_tag);
        let exists = candidates.iter().any(|p| p.exists());
        if exists {
            continue;
        }

        let mut owners = owner_refnos_by_hash
            .get(hash)
            .cloned()
            .unwrap_or_default()
            .into_iter()
            .collect::<Vec<_>>();
        owners.sort();
        for r in &owners {
            missing_owner_union.insert(r.clone());
        }
        let owner_sample = owners
            .iter()
            .take(MESH_REPORT_REFNO_SAMPLE_LIMIT)
            .cloned()
            .collect::<Vec<_>>();
        let row_count = *row_count_by_hash.get(hash).unwrap_or(&0);
        let candidate_paths = candidates
            .iter()
            .map(|p| p.display().to_string())
            .collect::<Vec<_>>();
        missing_entries.push((hash.to_string(), row_count, owners.len(), owner_sample, candidate_paths));
    }

    missing_entries.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));

    let generated_at = Utc::now().to_rfc3339_opts(SecondsFormat::Millis, true);
    let missing_geo_hashes_json = missing_entries
        .iter()
        .map(|(geo_hash, row_count, owner_count, owner_sample, candidate_paths)| {
            json!({
                "geo_hash": geo_hash,
                "row_count": row_count,
                "owner_refno_count": owner_count,
                "owner_refno_sample": owner_sample,
                "owner_refno_sample_count": owner_sample.len(),
                "mesh_candidates": candidate_paths,
            })
        })
        .collect::<Vec<_>>();

    let report = json!({
        "version": 1,
        "generated_at": generated_at,
        "dbnum": dbnum,
        "mesh_base_dir": mesh_base_dir.display().to_string(),
        "lod_tag": lod_tag,
        "checked_geo_hashes": checked_geo_hashes,
        "missing_geo_hashes": missing_entries.len(),
        "missing_owner_refnos": missing_owner_union.len(),
        "missing_geo_hash_list": missing_geo_hashes_json,
    });

    let report_file = format!("missing_mesh_report_{}.json", dbnum);
    let report_path = output_dir.join(&report_file);
    fs::write(&report_path, serde_json::to_string_pretty(&report)?)
        .with_context(|| format!("写入缺失 mesh 报告失败: {}", report_path.display()))?;

    if !missing_entries.is_empty() {
        eprintln!(
            "[parquet] dbnum={} 检测到缺失 mesh: geo_hashes={}, owner_refnos={}，报告={}",
            dbnum,
            missing_entries.len(),
            missing_owner_union.len(),
            report_path.display()
        );
    } else if verbose {
        println!(
            "   ✅ mesh 校验通过: checked_geo_hashes={} (lod={})",
            checked_geo_hashes, lod_tag
        );
    }

    Ok(MissingMeshReportSummary {
        report_file,
        checked_geo_hashes,
        missing_geo_hashes: missing_entries.len(),
        missing_owner_refnos: missing_owner_union.len(),
    })
}

// =============================================================================
// Schema 定义
// =============================================================================

fn instances_schema() -> Schema {
    Schema::new(vec![
        Field::new("refno_str", DataType::Utf8, false),
        Field::new("refno_u64", DataType::UInt64, false),
        Field::new("noun", DataType::Utf8, false),
        Field::new("owner_refno_str", DataType::Utf8, true),
        Field::new("owner_refno_u64", DataType::UInt64, true),
        Field::new("owner_noun", DataType::Utf8, false),
        Field::new("trans_hash", DataType::Utf8, false),
        Field::new("aabb_hash", DataType::Utf8, false),
        Field::new("spec_value", DataType::UInt64, false),
        Field::new("has_neg", DataType::Boolean, false),
        Field::new("dbnum", DataType::UInt32, false),
    ])
}

fn geo_instances_schema() -> Schema {
    Schema::new(vec![
        Field::new("refno_str", DataType::Utf8, false),
        Field::new("refno_u64", DataType::UInt64, false),
        Field::new("geo_index", DataType::UInt32, false),
        Field::new("geo_hash", DataType::Utf8, false),
        Field::new("geo_trans_hash", DataType::Utf8, false),
    ])
}

fn tubings_schema() -> Schema {
    Schema::new(vec![
        Field::new("tubi_refno_str", DataType::Utf8, false),
        Field::new("tubi_refno_u64", DataType::UInt64, false),
        Field::new("owner_refno_str", DataType::Utf8, false),
        Field::new("owner_refno_u64", DataType::UInt64, false),
        Field::new("order", DataType::UInt32, false),
        Field::new("geo_hash", DataType::Utf8, false),
        Field::new("trans_hash", DataType::Utf8, false),
        Field::new("aabb_hash", DataType::Utf8, false),
        Field::new("spec_value", DataType::UInt64, false),
        Field::new("dbnum", DataType::UInt32, false),
    ])
}

fn transforms_schema() -> Schema {
    Schema::new(vec![
        Field::new("trans_hash", DataType::Utf8, false),
        Field::new("m00", DataType::Float64, false),
        Field::new("m10", DataType::Float64, false),
        Field::new("m20", DataType::Float64, false),
        Field::new("m30", DataType::Float64, false),
        Field::new("m01", DataType::Float64, false),
        Field::new("m11", DataType::Float64, false),
        Field::new("m21", DataType::Float64, false),
        Field::new("m31", DataType::Float64, false),
        Field::new("m02", DataType::Float64, false),
        Field::new("m12", DataType::Float64, false),
        Field::new("m22", DataType::Float64, false),
        Field::new("m32", DataType::Float64, false),
        Field::new("m03", DataType::Float64, false),
        Field::new("m13", DataType::Float64, false),
        Field::new("m23", DataType::Float64, false),
        Field::new("m33", DataType::Float64, false),
    ])
}

fn aabb_schema() -> Schema {
    Schema::new(vec![
        Field::new("aabb_hash", DataType::Utf8, false),
        Field::new("min_x", DataType::Float64, false),
        Field::new("min_y", DataType::Float64, false),
        Field::new("min_z", DataType::Float64, false),
        Field::new("max_x", DataType::Float64, false),
        Field::new("max_y", DataType::Float64, false),
        Field::new("max_z", DataType::Float64, false),
    ])
}

// =============================================================================
// RecordBatch 构建
// =============================================================================

fn build_instances_batch(rows: &[InstanceRow]) -> Result<RecordBatch> {
    let schema = Arc::new(instances_schema());
    let batch = RecordBatch::try_new(
        schema,
        vec![
            Arc::new(StringArray::from(rows.iter().map(|r| r.refno_str.as_str()).collect::<Vec<_>>())) as ArrayRef,
            Arc::new(UInt64Array::from(rows.iter().map(|r| r.refno_u64).collect::<Vec<_>>())) as ArrayRef,
            Arc::new(StringArray::from(rows.iter().map(|r| r.noun.as_str()).collect::<Vec<_>>())) as ArrayRef,
            Arc::new(StringArray::from(rows.iter().map(|r| r.owner_refno_str.as_deref()).collect::<Vec<_>>())) as ArrayRef,
            Arc::new(UInt64Array::from(rows.iter().map(|r| r.owner_refno_u64).collect::<Vec<Option<u64>>>())) as ArrayRef,
            Arc::new(StringArray::from(rows.iter().map(|r| r.owner_noun.as_str()).collect::<Vec<_>>())) as ArrayRef,
            Arc::new(StringArray::from(rows.iter().map(|r| r.trans_hash.as_str()).collect::<Vec<_>>())) as ArrayRef,
            Arc::new(StringArray::from(rows.iter().map(|r| r.aabb_hash.as_str()).collect::<Vec<_>>())) as ArrayRef,
            Arc::new(UInt64Array::from(rows.iter().map(|r| r.spec_value as u64).collect::<Vec<_>>())) as ArrayRef,
            Arc::new(BooleanArray::from(rows.iter().map(|r| r.has_neg).collect::<Vec<_>>())) as ArrayRef,
            Arc::new(UInt32Array::from(rows.iter().map(|r| r.dbnum).collect::<Vec<_>>())) as ArrayRef,
        ],
    )?;
    Ok(batch)
}

fn build_geo_instances_batch(rows: &[GeoInstanceRow]) -> Result<RecordBatch> {
    let schema = Arc::new(geo_instances_schema());
    let batch = RecordBatch::try_new(
        schema,
        vec![
            Arc::new(StringArray::from(rows.iter().map(|r| r.refno_str.as_str()).collect::<Vec<_>>())) as ArrayRef,
            Arc::new(UInt64Array::from(rows.iter().map(|r| r.refno_u64).collect::<Vec<_>>())) as ArrayRef,
            Arc::new(UInt32Array::from(rows.iter().map(|r| r.geo_index).collect::<Vec<_>>())) as ArrayRef,
            Arc::new(StringArray::from(rows.iter().map(|r| r.geo_hash.as_str()).collect::<Vec<_>>())) as ArrayRef,
            Arc::new(StringArray::from(rows.iter().map(|r| r.geo_trans_hash.as_str()).collect::<Vec<_>>())) as ArrayRef,
        ],
    )?;
    Ok(batch)
}

fn build_tubings_batch(rows: &[TubingRow]) -> Result<RecordBatch> {
    let schema = Arc::new(tubings_schema());
    let batch = RecordBatch::try_new(
        schema,
        vec![
            Arc::new(StringArray::from(rows.iter().map(|r| r.tubi_refno_str.as_str()).collect::<Vec<_>>())) as ArrayRef,
            Arc::new(UInt64Array::from(rows.iter().map(|r| r.tubi_refno_u64).collect::<Vec<_>>())) as ArrayRef,
            Arc::new(StringArray::from(rows.iter().map(|r| r.owner_refno_str.as_str()).collect::<Vec<_>>())) as ArrayRef,
            Arc::new(UInt64Array::from(rows.iter().map(|r| r.owner_refno_u64).collect::<Vec<_>>())) as ArrayRef,
            Arc::new(UInt32Array::from(rows.iter().map(|r| r.order).collect::<Vec<_>>())) as ArrayRef,
            Arc::new(StringArray::from(rows.iter().map(|r| r.geo_hash.as_str()).collect::<Vec<_>>())) as ArrayRef,
            Arc::new(StringArray::from(rows.iter().map(|r| r.trans_hash.as_str()).collect::<Vec<_>>())) as ArrayRef,
            Arc::new(StringArray::from(rows.iter().map(|r| r.aabb_hash.as_str()).collect::<Vec<_>>())) as ArrayRef,
            Arc::new(UInt64Array::from(rows.iter().map(|r| r.spec_value as u64).collect::<Vec<_>>())) as ArrayRef,
            Arc::new(UInt32Array::from(rows.iter().map(|r| r.dbnum).collect::<Vec<_>>())) as ArrayRef,
        ],
    )?;
    Ok(batch)
}

fn build_transforms_batch(rows: &[TransformRow]) -> Result<RecordBatch> {
    let schema = Arc::new(transforms_schema());
    let batch = RecordBatch::try_new(
        schema,
        vec![
            Arc::new(StringArray::from(rows.iter().map(|r| r.trans_hash.as_str()).collect::<Vec<_>>())) as ArrayRef,
            Arc::new(Float64Array::from(rows.iter().map(|r| r.m00).collect::<Vec<_>>())) as ArrayRef,
            Arc::new(Float64Array::from(rows.iter().map(|r| r.m10).collect::<Vec<_>>())) as ArrayRef,
            Arc::new(Float64Array::from(rows.iter().map(|r| r.m20).collect::<Vec<_>>())) as ArrayRef,
            Arc::new(Float64Array::from(rows.iter().map(|r| r.m30).collect::<Vec<_>>())) as ArrayRef,
            Arc::new(Float64Array::from(rows.iter().map(|r| r.m01).collect::<Vec<_>>())) as ArrayRef,
            Arc::new(Float64Array::from(rows.iter().map(|r| r.m11).collect::<Vec<_>>())) as ArrayRef,
            Arc::new(Float64Array::from(rows.iter().map(|r| r.m21).collect::<Vec<_>>())) as ArrayRef,
            Arc::new(Float64Array::from(rows.iter().map(|r| r.m31).collect::<Vec<_>>())) as ArrayRef,
            Arc::new(Float64Array::from(rows.iter().map(|r| r.m02).collect::<Vec<_>>())) as ArrayRef,
            Arc::new(Float64Array::from(rows.iter().map(|r| r.m12).collect::<Vec<_>>())) as ArrayRef,
            Arc::new(Float64Array::from(rows.iter().map(|r| r.m22).collect::<Vec<_>>())) as ArrayRef,
            Arc::new(Float64Array::from(rows.iter().map(|r| r.m32).collect::<Vec<_>>())) as ArrayRef,
            Arc::new(Float64Array::from(rows.iter().map(|r| r.m03).collect::<Vec<_>>())) as ArrayRef,
            Arc::new(Float64Array::from(rows.iter().map(|r| r.m13).collect::<Vec<_>>())) as ArrayRef,
            Arc::new(Float64Array::from(rows.iter().map(|r| r.m23).collect::<Vec<_>>())) as ArrayRef,
            Arc::new(Float64Array::from(rows.iter().map(|r| r.m33).collect::<Vec<_>>())) as ArrayRef,
        ],
    )?;
    Ok(batch)
}

fn build_aabb_batch(rows: &[AabbRow]) -> Result<RecordBatch> {
    let schema = Arc::new(aabb_schema());
    let batch = RecordBatch::try_new(
        schema,
        vec![
            Arc::new(StringArray::from(rows.iter().map(|r| r.aabb_hash.as_str()).collect::<Vec<_>>())) as ArrayRef,
            Arc::new(Float64Array::from(rows.iter().map(|r| r.min_x).collect::<Vec<_>>())) as ArrayRef,
            Arc::new(Float64Array::from(rows.iter().map(|r| r.min_y).collect::<Vec<_>>())) as ArrayRef,
            Arc::new(Float64Array::from(rows.iter().map(|r| r.min_z).collect::<Vec<_>>())) as ArrayRef,
            Arc::new(Float64Array::from(rows.iter().map(|r| r.max_x).collect::<Vec<_>>())) as ArrayRef,
            Arc::new(Float64Array::from(rows.iter().map(|r| r.max_y).collect::<Vec<_>>())) as ArrayRef,
            Arc::new(Float64Array::from(rows.iter().map(|r| r.max_z).collect::<Vec<_>>())) as ArrayRef,
        ],
    )?;
    Ok(batch)
}

// =============================================================================
// SurrealDB 查询结构体
// =============================================================================

use serde::{Deserialize, Serialize};
use surrealdb::types::{self as surrealdb_types, SurrealValue};

// InstRelateRow 使用 export_common 中的共享定义
use super::InstRelateRow;

#[derive(Clone, Debug, Serialize, Deserialize, SurrealValue)]
struct TubiQueryResult {
    pub refno: RefnoEnum,
    pub index: Option<i64>,
    pub leave: RefnoEnum,
    pub world_aabb_hash: Option<String>,
    pub world_trans_hash: Option<String>,
    pub geo_hash: Option<String>,
    pub spec_value: Option<i64>,
}

#[derive(Debug, Deserialize, SurrealValue)]
struct TransQueryRow {
    hash: String,
    d: serde_json::Value,
}

#[derive(Debug, Deserialize, SurrealValue)]
struct AabbQueryRow {
    hash: String,
    d: Option<aios_core::types::PlantAabb>,
}

// =============================================================================
// SurrealDB 查询函数
// =============================================================================

/// 从 inst_relate 查询所有 distinct dbnum（排序后返回）
///
/// 用于 `--export-parquet` 未指定 `--dbnum` 时自动扫描并逐一导出。
pub async fn query_distinct_dbnums_from_inst_relate() -> Result<Vec<u32>> {
    use aios_core::SurrealQueryExt;

    let sql = "SELECT VALUE array::distinct(dbnum) FROM inst_relate GROUP ALL;";
    let result: Vec<Vec<i64>> = aios_core::project_primary_db()
        .query_take(sql, 0)
        .await?;

    let mut dbnums: Vec<u32> = result
        .into_iter()
        .flatten()
        .filter_map(|v| if v >= 0 { Some(v as u32) } else { None })
        .collect();
    dbnums.sort_unstable();
    dbnums.dedup();
    Ok(dbnums)
}

/// 通过 dbnum 字段过滤 inst_relate（命中 dbnum 索引路径）
async fn query_inst_relate_by_dbnum(
    dbnum: u32,
    verbose: bool,
) -> Result<Vec<InstRelateRow>> {
    if verbose {
        println!(
            "🔍 扫描 inst_relate（索引路径: WHERE dbnum = {}）...",
            dbnum
        );
    }

    let sql = r#"
        SELECT
            owner_refno,
            owner_type,
            in as refno,
            in.noun as noun,
            spec_value as spec_value
        FROM inst_relate
        WHERE dbnum = $dbnum
    "#;

    let mut resp = aios_core::project_primary_db()
        .query(sql)
        .bind(("dbnum", dbnum))
        .await?;
    let rows: Vec<InstRelateRow> = resp.take(0)?;

    if verbose {
        println!("✅ inst_relate 命中记录: {}", rows.len());
    }

    Ok(rows)
}

/// 使用 refno 列表分批查询 inst_relate（root_refno 子树模式使用）
async fn query_inst_relate_by_refnos(
    refnos: &[RefnoEnum],
    verbose: bool,
) -> Result<Vec<InstRelateRow>> {
    if refnos.is_empty() {
        return Ok(Vec::new());
    }

    const BATCH_SIZE: usize = 500;
    let mut rows = Vec::new();

    for (idx, chunk) in refnos.chunks(BATCH_SIZE).enumerate() {
        if verbose {
            println!(
                "   - 查询 inst_relate 分批 {}/{} (批大小 {})",
                idx + 1,
                (refnos.len() + BATCH_SIZE - 1) / BATCH_SIZE,
                chunk.len()
            );
        }

        let pe_list = chunk
            .iter()
            .map(|r| format!("pe:⟨{}⟩", r.to_string()))
            .collect::<Vec<_>>()
            .join(", ");

        let sql = format!(
            r#"
            SELECT
                owner_refno,
                owner_type,
                in as refno,
                in.noun as noun,
                spec_value as spec_value
            FROM [{pe_list}]->inst_relate
            "#
        );

        let mut chunk_rows: Vec<InstRelateRow> =
            aios_core::project_primary_db().query_take(&sql, 0).await?;
        rows.append(&mut chunk_rows);
    }

    Ok(rows)
}

/// 批量查询 tubi_relate
async fn query_tubi_relate(
    owner_refnos: &[RefnoEnum],
    verbose: bool,
) -> Result<HashMap<RefnoEnum, Vec<TubiQueryResult>>> {
    let mut tubings_map: HashMap<RefnoEnum, Vec<TubiQueryResult>> = HashMap::new();

    if owner_refnos.is_empty() {
        return Ok(tubings_map);
    }

    for owners_chunk in owner_refnos.chunks(200) {
        let mut sql_batch = String::new();
        for owner_refno in owners_chunk {
            let pe_key = owner_refno.to_pe_key();
            sql_batch.push_str(&format!(
                r#"
                SELECT
                    id[0] as refno,
                    id[1] as index,
                    in as leave,
                    record::id(aabb) as world_aabb_hash,
                    record::id(world_trans) as world_trans_hash,
                    record::id(geo) as geo_hash,
                    spec_value
                FROM tubi_relate:[{pe_key}, 0]..[{pe_key}, ..];
                "#,
            ));
        }

        let mut resp = aios_core::project_primary_db().query_response(&sql_batch).await?;
        for (stmt_idx, owner_refno) in owners_chunk.iter().enumerate() {
            let raw_rows: Vec<TubiQueryResult> = resp.take(stmt_idx)?;
            for row in raw_rows {
                if row.geo_hash.is_some() {
                    tubings_map.entry(*owner_refno).or_default().push(row);
                }
            }
        }
    }

    // 排序：按 index 保序
    for tubis in tubings_map.values_mut() {
        tubis.sort_by_key(|t| t.index.unwrap_or(0));
    }

    if verbose {
        let total: usize = tubings_map.values().map(|v| v.len()).sum();
        println!("   ✅ 查询到 {} 条 tubi_relate 记录", total);
    }

    Ok(tubings_map)
}

/// 批量查询 trans 表，返回 TransformRow 列表
async fn query_trans_rows(
    hashes: &HashSet<String>,
    unit_converter: &UnitConverter,
    verbose: bool,
) -> Result<Vec<TransformRow>> {
    use aios_core::project_primary_db;

    let mut result = Vec::new();
    if hashes.is_empty() {
        return Ok(result);
    }

    let hashes_vec: Vec<&String> = hashes.iter().collect();
    for chunk in hashes_vec.chunks(500) {
        let keys: Vec<String> = chunk.iter()
            .map(|h| format!("trans:⟨{}⟩", h))
            .collect();
        let sql = format!(
            "SELECT record::id(id) as hash, d FROM [{}]",
            keys.join(", ")
        );

        if verbose {
            println!("   查询 trans: {} 个", chunk.len());
        }

        let rows: Vec<TransQueryRow> = project_primary_db().query_take(&sql, 0).await.unwrap_or_default();
        for row in rows {
            if let Some(obj) = row.d.as_object() {
                let translation = obj.get("translation")
                    .and_then(|v| v.as_array())
                    .map(|arr| {
                        let x = arr.get(0).and_then(|v| v.as_f64()).unwrap_or(0.0);
                        let y = arr.get(1).and_then(|v| v.as_f64()).unwrap_or(0.0);
                        let z = arr.get(2).and_then(|v| v.as_f64()).unwrap_or(0.0);
                        glam::DVec3::new(x, y, z)
                    })
                    .unwrap_or(glam::DVec3::ZERO);

                let rotation = obj.get("rotation")
                    .and_then(|v| v.as_array())
                    .map(|arr| {
                        let x = arr.get(0).and_then(|v| v.as_f64()).unwrap_or(0.0);
                        let y = arr.get(1).and_then(|v| v.as_f64()).unwrap_or(0.0);
                        let z = arr.get(2).and_then(|v| v.as_f64()).unwrap_or(0.0);
                        let w = arr.get(3).and_then(|v| v.as_f64()).unwrap_or(1.0);
                        glam::DQuat::from_xyzw(x, y, z, w)
                    })
                    .unwrap_or(glam::DQuat::IDENTITY);

                let scale = obj.get("scale")
                    .and_then(|v| v.as_array())
                    .map(|arr| {
                        let x = arr.get(0).and_then(|v| v.as_f64()).unwrap_or(1.0);
                        let y = arr.get(1).and_then(|v| v.as_f64()).unwrap_or(1.0);
                        let z = arr.get(2).and_then(|v| v.as_f64()).unwrap_or(1.0);
                        glam::DVec3::new(x, y, z)
                    })
                    .unwrap_or(glam::DVec3::ONE);

                // 单位转换（仅平移部分）
                let factor = unit_converter.conversion_factor() as f64;
                let converted_translation = glam::DVec3::new(
                    translation.x * factor,
                    translation.y * factor,
                    translation.z * factor,
                );

                let mat = glam::DMat4::from_scale_rotation_translation(
                    scale, rotation, converted_translation,
                );
                let cols = mat.to_cols_array();

                result.push(TransformRow {
                    trans_hash: row.hash,
                    m00: cols[0], m10: cols[1], m20: cols[2], m30: cols[3],
                    m01: cols[4], m11: cols[5], m21: cols[6], m31: cols[7],
                    m02: cols[8], m12: cols[9], m22: cols[10], m32: cols[11],
                    m03: cols[12], m13: cols[13], m23: cols[14], m33: cols[15],
                });
            }
        }
    }

    Ok(result)
}

/// 批量查询 aabb 表，返回 AabbRow 列表
async fn query_aabb_rows(
    hashes: &HashSet<String>,
    unit_converter: &UnitConverter,
    verbose: bool,
) -> Result<Vec<AabbRow>> {
    use aios_core::project_primary_db;

    let mut result = Vec::new();
    if hashes.is_empty() {
        return Ok(result);
    }

    let hashes_vec: Vec<&String> = hashes.iter().collect();
    for chunk in hashes_vec.chunks(500) {
        let keys: Vec<String> = chunk.iter()
            .map(|h| format!("aabb:⟨{}⟩", h))
            .collect();
        let sql = format!(
            "SELECT record::id(id) as hash, d FROM [{}]",
            keys.join(", ")
        );

        if verbose {
            println!("   查询 aabb: {} 个", chunk.len());
        }

        let rows: Vec<AabbQueryRow> = project_primary_db().query_take(&sql, 0).await.unwrap_or_default();
        for row in rows {
            if let Some(aabb) = row.d {
                let mins = aabb.0.mins;
                let maxs = aabb.0.maxs;
                let factor = unit_converter.conversion_factor() as f64;
                result.push(AabbRow {
                    aabb_hash: row.hash,
                    min_x: mins.x as f64 * factor,
                    min_y: mins.y as f64 * factor,
                    min_z: mins.z as f64 * factor,
                    max_x: maxs.x as f64 * factor,
                    max_y: maxs.y as f64 * factor,
                    max_z: maxs.z as f64 * factor,
                });
            }
        }
    }

    Ok(result)
}

// =============================================================================
// 主导出函数
// =============================================================================

/// Parquet 导出统计信息
pub struct ParquetExportStats {
    pub instance_count: usize,
    pub geo_instance_count: usize,
    pub tubing_count: usize,
    pub transform_count: usize,
    pub aabb_count: usize,
    pub total_bytes: u64,
    pub elapsed: std::time::Duration,
}

/// 从 SurrealDB 导出指定 dbnum 的实例数据为多表 Parquet 格式
///
/// # 参数
/// - `dbnum`: 数据库编号
/// - `output_dir`: 输出目录
/// - `db_option`: 数据库选项
/// - `verbose`: 是否输出详细日志
/// - `target_unit`: 目标单位（可选，默认毫米）
/// - `root_refno`: 若提供，则仅导出该 refno 下的 visible 子孙节点
///
/// # 返回
/// 导出统计信息
#[cfg_attr(feature = "profile", tracing::instrument(skip_all, name = "export_dbnum_instances_parquet"))]
pub async fn export_dbnum_instances_parquet(
    dbnum: u32,
    output_dir: &Path,
    db_option: Arc<DbOption>,
    verbose: bool,
    target_unit: Option<LengthUnit>,
    root_refno: Option<RefnoEnum>,
) -> Result<ParquetExportStats> {
    let start_time = std::time::Instant::now();

    let target = target_unit.unwrap_or(LengthUnit::Millimeter);
    let unit_converter = UnitConverter::new(LengthUnit::Millimeter, target);

    if verbose {
        println!("🚀 开始导出 dbnum={} 的实例数据为 Parquet，目标单位: {:?}", dbnum, target);
    }

    // 确保输出目录存在
    fs::create_dir_all(output_dir)
        .with_context(|| format!("创建输出目录失败: {}", output_dir.display()))?;
    let mesh_base_dir = mesh_base_dir_from_db_option(&db_option);

    // 构建/加载 spec_info（BRAN/HANG/EQUI/WALL/FLOOR 专业信息），用于 spec_value=0 时回填
    let tree_manager = TreeIndexManager::with_default_dir(vec![dbnum]);
    let tree_dir = tree_manager.tree_dir();
    let spec_info_map = match crate::fast_model::export_model::spec_info::load_or_build_spec_info(
        dbnum,
        tree_dir,
        output_dir,
        verbose,
    )
    .await
    {
        Ok(m) => {
            if verbose && !m.is_empty() {
                println!("   📋 spec_info: {} 条 refno->spec_value 映射", m.len());
            }
            m
        }
        Err(e) => {
            eprintln!("   ⚠️ spec_info 加载/构建失败 (将使用 spec_value=0): {}", e);
            HashMap::new()
        }
    };

    // =========================================================================
    // 1-2. 扫描 inst_relate（按 dbnum 对应的 ref0 前缀过滤）
    // =========================================================================
    let inst_rows = if let Some(root) = root_refno {
        // root_refno 模式：先查子树 refno，再分批查 inst_relate
        use crate::fast_model::query_compat::query_deep_visible_inst_refnos;
        if verbose {
            println!("🔍 查询 {} 的可见实例节点...", root);
        }
        let sub_refnos = query_deep_visible_inst_refnos(root).await?;
        if verbose {
            println!("✅ 子树 refno 数量: {}", sub_refnos.len());
        }
        query_inst_relate_by_refnos(&sub_refnos, verbose).await?
    } else {
        query_inst_relate_by_dbnum(dbnum, verbose).await?
    };

    // 按 owner 分组
    struct ChildInfo {
        refno: RefnoEnum,
        noun: String,
        spec_value: i64,
        owner_refno: Option<RefnoEnum>,
        owner_type: String,
    }

    let mut grouped_children: HashMap<RefnoEnum, Vec<ChildInfo>> = HashMap::new();
    let mut ungrouped: Vec<ChildInfo> = Vec::new();
    let mut in_refnos: Vec<RefnoEnum> = Vec::new();
    let mut in_refno_set: HashSet<RefnoEnum> = HashSet::new();

    for row in inst_rows {
        let owner_type = row
            .owner_type
            .as_deref()
            .unwrap_or_default()
            .to_ascii_uppercase();

        let mut spec_value = row.spec_value.unwrap_or(0);
        if spec_value == 0 {
            spec_value = *spec_info_map.get(&refno_to_u64(&row.refno)).unwrap_or(&0);
            // 组件(ELBO/BEND等)的 refno 不在 spec_info，用 owner(BRAN/HANG) 查
            if spec_value == 0 {
                if let Some(owner) = &row.owner_refno {
                    spec_value = *spec_info_map.get(&refno_to_u64(owner)).unwrap_or(&0);
                }
            }
        }
        let child = ChildInfo {
            refno: row.refno,
            noun: row.noun.unwrap_or_default(),
            spec_value,
            owner_refno: row.owner_refno,
            owner_type: owner_type.clone(),
        };

        if in_refno_set.insert(row.refno) {
            in_refnos.push(row.refno);
        }

        if matches!(owner_type.as_str(), "BRAN" | "HANG" | "EQUI") {
            if let Some(owner) = row.owner_refno {
                grouped_children.entry(owner).or_default().push(child);
            } else {
                ungrouped.push(child);
            }
        } else {
            ungrouped.push(child);
        }
    }

    // =========================================================================
    // 3. 查询几何体实例 hash（geo_relate / inst_relate_bool）
    // =========================================================================
    if verbose {
        println!("🔍 查询 {} 个 refno 的几何体实例 hash...", in_refnos.len());
    }
    let mut export_inst_map: HashMap<RefnoEnum, aios_core::ExportInstQuery> = HashMap::new();
    if !in_refnos.is_empty() {
        match aios_core::query_insts_for_export(&in_refnos, true).await {
            Ok(export_insts) => {
                for inst in export_insts {
                    export_inst_map.insert(inst.refno, inst);
                }
                if verbose {
                    println!("✅ 查询到 {} 个 refno 有几何体实例 (inst_relate 共 {} 个)", export_inst_map.len(), in_refnos.len());
                    let in_set: HashSet<_> = in_refnos.iter().collect();
                    let exported_set: HashSet<_> = export_inst_map.keys().collect();
                    let missing_geo: Vec<_> = in_set.difference(&exported_set).collect();
                    if !missing_geo.is_empty() && missing_geo.len() <= 20 {
                        println!("   ⚠️ 以下 refno 在 inst_relate 但无几何体(geo_relate/inst_relate_bool): {:?}", missing_geo.iter().map(|r| r.to_string()).collect::<Vec<_>>());
                    } else if !missing_geo.is_empty() {
                        println!("   ⚠️ {} 个 refno 在 inst_relate 但无几何体，样例: {:?}", missing_geo.len(), missing_geo.iter().take(5).map(|r| r.to_string()).collect::<Vec<_>>());
                    }
                }
            }
            Err(e) => {
                if verbose {
                    println!("⚠️ 几何体实例查询失败: {:?}", e);
                }
            }
        }
    }

    // =========================================================================
    // 4. 查询 tubi_relate
    // =========================================================================
    let tubi_owner_refnos: Vec<RefnoEnum> = grouped_children
        .iter()
        .filter(|(_, children)| {
            children.first().map_or(false, |c| matches!(c.owner_type.as_str(), "BRAN" | "HANG"))
        })
        .map(|(k, _)| *k)
        .collect();

    if verbose {
        println!("🔍 查询 {} 个 BRAN/HANG owner 的 tubi_relate...", tubi_owner_refnos.len());
    }
    let tubings_map = query_tubi_relate(&tubi_owner_refnos, verbose).await?;

    // =========================================================================
    // 5. 构建 Parquet 行数据
    // =========================================================================
    let mut instance_rows: Vec<InstanceRow> = Vec::new();
    let mut geo_instance_rows: Vec<GeoInstanceRow> = Vec::new();
    let mut tubing_rows: Vec<TubingRow> = Vec::new();
    let mut trans_hashes: HashSet<String> = HashSet::new();
    let mut aabb_hashes: HashSet<String> = HashSet::new();
    let mut owner_refnos_by_hash: HashMap<String, HashSet<String>> = HashMap::new();
    let mut row_count_by_hash: HashMap<String, usize> = HashMap::new();

    // 处理 grouped children
    for (owner_refno, children) in &grouped_children {
        let owner_type = children.first().map(|c| c.owner_type.as_str()).unwrap_or("");

        for child in children {
            let export_inst = export_inst_map.get(&child.refno);
            let Some(export_inst) = export_inst else { continue };
            if export_inst.insts.is_empty() { continue }

            let child_aabb_hash = export_inst.world_aabb_hash.clone()
                .unwrap_or_default();

            let trans_hash = export_inst.world_trans_hash.clone().unwrap_or_default();

            // 收集 hash
            if !child_aabb_hash.is_empty() { aabb_hashes.insert(child_aabb_hash.clone()); }
            if !trans_hash.is_empty() { trans_hashes.insert(trans_hash.clone()); }
            for inst in &export_inst.insts {
                if let Some(ref th) = inst.trans_hash {
                    if !th.is_empty() { trans_hashes.insert(th.clone()); }
                }
            }

            instance_rows.push(InstanceRow {
                refno_str: child.refno.to_string(),
                refno_u64: refno_to_u64(&child.refno),
                noun: child.noun.clone(),
                owner_refno_str: Some(owner_refno.to_string()),
                owner_refno_u64: Some(refno_to_u64(owner_refno)),
                owner_noun: owner_type.to_string(),
                trans_hash: trans_hash.clone(),
                aabb_hash: child_aabb_hash,
                spec_value: child.spec_value,
                has_neg: export_inst.has_neg,
                dbnum,
            });

            // geo_instances
            for (geo_idx, inst) in export_inst.insts.iter().enumerate() {
                geo_instance_rows.push(GeoInstanceRow {
                    refno_str: child.refno.to_string(),
                    refno_u64: refno_to_u64(&child.refno),
                    geo_index: geo_idx as u32,
                    geo_hash: inst.geo_hash.clone(),
                    geo_trans_hash: inst.trans_hash.clone().unwrap_or_default(),
                });
                record_geo_hash_usage(
                    &inst.geo_hash,
                    &child.refno.to_string(),
                    &mut owner_refnos_by_hash,
                    &mut row_count_by_hash,
                );
            }
        }

        // tubings
        if let Some(tubis) = tubings_map.get(owner_refno) {
            for tubi in tubis {
                let aabb_hash = tubi.world_aabb_hash.clone().unwrap_or_default();
                let trans_hash = tubi.world_trans_hash.clone().unwrap_or_default();
                let geo_hash = tubi.geo_hash.clone().unwrap_or_default();

                if aabb_hash.is_empty() || geo_hash.is_empty() { continue }

                if !aabb_hash.is_empty() { aabb_hashes.insert(aabb_hash.clone()); }
                if !trans_hash.is_empty() { trans_hashes.insert(trans_hash.clone()); }

                let index = tubi.index
                    .and_then(|v| u32::try_from(v).ok())
                    .unwrap_or(0);

                let mut tubi_spec = tubi.spec_value.unwrap_or(0);
                if tubi_spec == 0 {
                    tubi_spec = *spec_info_map.get(&refno_to_u64(owner_refno)).unwrap_or(&0);
                }

                tubing_rows.push(TubingRow {
                    tubi_refno_str: tubi.leave.to_string(),
                    tubi_refno_u64: refno_to_u64(&tubi.leave),
                    owner_refno_str: owner_refno.to_string(),
                    owner_refno_u64: refno_to_u64(owner_refno),
                    order: index,
                    geo_hash,
                    trans_hash,
                    aabb_hash,
                    spec_value: tubi_spec,
                    dbnum,
                });
                record_geo_hash_usage(
                    &tubi.geo_hash.clone().unwrap_or_default(),
                    &owner_refno.to_string(),
                    &mut owner_refnos_by_hash,
                    &mut row_count_by_hash,
                );
            }
        }
    }

    // 处理 ungrouped instances
    for child in &ungrouped {
        let export_inst = export_inst_map.get(&child.refno);
        let Some(export_inst) = export_inst else { continue };
        if export_inst.insts.is_empty() { continue }

        let child_aabb_hash = export_inst.world_aabb_hash.clone()
            .unwrap_or_default();

        let trans_hash = export_inst.world_trans_hash.clone().unwrap_or_default();

        if !child_aabb_hash.is_empty() { aabb_hashes.insert(child_aabb_hash.clone()); }
        if !trans_hash.is_empty() { trans_hashes.insert(trans_hash.clone()); }
        for inst in &export_inst.insts {
            if let Some(ref th) = inst.trans_hash {
                if !th.is_empty() { trans_hashes.insert(th.clone()); }
            }
        }

        instance_rows.push(InstanceRow {
            refno_str: child.refno.to_string(),
            refno_u64: refno_to_u64(&child.refno),
            noun: child.noun.clone(),
            owner_refno_str: child.owner_refno.map(|r| r.to_string()),
            owner_refno_u64: child.owner_refno.map(|r| refno_to_u64(&r)),
            owner_noun: child.owner_type.clone(),
            trans_hash: trans_hash.clone(),
            aabb_hash: child_aabb_hash,
            spec_value: child.spec_value,
            has_neg: export_inst.has_neg,
            dbnum,
        });

        for (geo_idx, inst) in export_inst.insts.iter().enumerate() {
            geo_instance_rows.push(GeoInstanceRow {
                refno_str: child.refno.to_string(),
                refno_u64: refno_to_u64(&child.refno),
                geo_index: geo_idx as u32,
                geo_hash: inst.geo_hash.clone(),
                geo_trans_hash: inst.trans_hash.clone().unwrap_or_default(),
            });
            record_geo_hash_usage(
                &inst.geo_hash,
                &child.refno.to_string(),
                &mut owner_refnos_by_hash,
                &mut row_count_by_hash,
            );
        }
    }

    let missing_mesh_report = write_missing_mesh_report(
        output_dir,
        dbnum,
        &mesh_base_dir,
        MESH_CHECK_LOD_TAG,
        &owner_refnos_by_hash,
        &row_count_by_hash,
        verbose,
    )?;

    // =========================================================================
    // 6. 查询 trans/aabb 实际数据
    // =========================================================================
    if verbose {
        println!("🔍 查询 {} 个 trans, {} 个 aabb...", trans_hashes.len(), aabb_hashes.len());
    }
    let (transform_rows_result, aabb_rows_result) = tokio::join!(
        query_trans_rows(&trans_hashes, &unit_converter, verbose),
        query_aabb_rows(&aabb_hashes, &unit_converter, verbose),
    );
    let transform_rows = transform_rows_result?;
    let aabb_row_data = aabb_rows_result?;

    if verbose {
        println!("✅ trans 命中: {}, aabb 命中: {}", transform_rows.len(), aabb_row_data.len());
    }

    // =========================================================================
    // 7. 写入 Parquet 文件
    // =========================================================================
    if verbose {
        println!("\n📝 写入 Parquet 文件...");
    }

    let mut total_bytes: u64 = 0;

    // instances.parquet
    {
        let batch = build_instances_batch(&instance_rows)?;
        let path = output_dir.join("instances.parquet");
        let size = write_parquet(&path, &batch)?;
        total_bytes += size;
        if verbose {
            println!("   ✅ instances.parquet: {} 行, {} 字节", instance_rows.len(), size);
        }
    }

    // geo_instances.parquet
    {
        let batch = build_geo_instances_batch(&geo_instance_rows)?;
        let path = output_dir.join("geo_instances.parquet");
        let size = write_parquet(&path, &batch)?;
        total_bytes += size;
        if verbose {
            println!(
                "   ✅ geo_instances.parquet: {} 行, {} 字节",
                geo_instance_rows.len(),
                size
            );
        }
    }

    // tubings.parquet
    {
        let batch = build_tubings_batch(&tubing_rows)?;
        let path = output_dir.join("tubings.parquet");
        let size = write_parquet(&path, &batch)?;
        total_bytes += size;
        if verbose {
            println!("   ✅ tubings.parquet: {} 行, {} 字节", tubing_rows.len(), size);
        }
    }

    // transforms.parquet
    {
        let batch = build_transforms_batch(&transform_rows)?;
        let path = output_dir.join("transforms.parquet");
        let size = write_parquet(&path, &batch)?;
        total_bytes += size;
        if verbose {
            println!(
                "   ✅ transforms.parquet: {} 行, {} 字节",
                transform_rows.len(),
                size
            );
        }
    }

    // aabb.parquet
    {
        let batch = build_aabb_batch(&aabb_row_data)?;
        let path = output_dir.join("aabb.parquet");
        let size = write_parquet(&path, &batch)?;
        total_bytes += size;
        if verbose {
            println!("   ✅ aabb.parquet: {} 行, {} 字节", aabb_row_data.len(), size);
        }
    }

    // =========================================================================
    // 8. 写入 manifest.json
    // =========================================================================
    let generated_at = Utc::now().to_rfc3339_opts(SecondsFormat::Millis, true);
    let manifest = json!({
        "version": 1,
        "format": "parquet",
        "generated_at": generated_at,
        "dbnum": dbnum,
        "root_refno": root_refno.map(|r| r.to_string()),
        "tables": {
            "instances": {
                "file": "instances.parquet",
                "rows": instance_rows.len(),
            },
            "geo_instances": {
                "file": "geo_instances.parquet",
                "rows": geo_instance_rows.len(),
            },
            "tubings": {
                "file": "tubings.parquet",
                "rows": tubing_rows.len(),
            },
            "transforms": {
                "file": "transforms.parquet",
                "rows": transform_rows.len(),
            },
            "aabb": {
                "file": "aabb.parquet",
                "rows": aabb_row_data.len(),
            },
        },
        "mesh_validation": {
            "lod_tag": MESH_CHECK_LOD_TAG,
            "report_file": missing_mesh_report.report_file,
            "checked_geo_hashes": missing_mesh_report.checked_geo_hashes,
            "missing_geo_hashes": missing_mesh_report.missing_geo_hashes,
            "missing_owner_refnos": missing_mesh_report.missing_owner_refnos,
        },
        "total_bytes": total_bytes,
    });

    let manifest_path = output_dir.join("manifest.json");
    fs::write(&manifest_path, serde_json::to_string_pretty(&manifest)?)?;
    if verbose {
        println!("   ✅ manifest.json 已写入");
    }

    // 写一份 manifest_{dbnum}.json 到 parquet/ 根目录，文件路径加 {dbnum}/ 前缀，
    // 与前端 buildFilesOutputUrl(`parquet/manifest_${dbno}.json`) 路径对齐。
    if let Some(parent) = output_dir.parent() {
        let subdir = output_dir
            .file_name()
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_else(|| dbnum.to_string());
        let web_manifest = json!({
            "version": 1,
            "format": "parquet",
            "generated_at": generated_at,
            "dbnum": dbnum,
            "root_refno": root_refno.map(|r| r.to_string()),
            "tables": {
                "instances": {
                    "file": format!("{}/instances.parquet", subdir),
                    "rows": instance_rows.len(),
                },
                "geo_instances": {
                    "file": format!("{}/geo_instances.parquet", subdir),
                    "rows": geo_instance_rows.len(),
                },
                "tubings": {
                    "file": format!("{}/tubings.parquet", subdir),
                    "rows": tubing_rows.len(),
                },
                "transforms": {
                    "file": format!("{}/transforms.parquet", subdir),
                    "rows": transform_rows.len(),
                },
                "aabb": {
                    "file": format!("{}/aabb.parquet", subdir),
                    "rows": aabb_row_data.len(),
                },
            },
            "mesh_validation": {
                "lod_tag": MESH_CHECK_LOD_TAG,
                "report_file": format!("{}/{}", subdir, missing_mesh_report.report_file),
                "checked_geo_hashes": missing_mesh_report.checked_geo_hashes,
                "missing_geo_hashes": missing_mesh_report.missing_geo_hashes,
                "missing_owner_refnos": missing_mesh_report.missing_owner_refnos,
            },
            "total_bytes": total_bytes,
        });
        let web_manifest_path = parent.join(format!("manifest_{}.json", dbnum));
        fs::write(&web_manifest_path, serde_json::to_string_pretty(&web_manifest)?)?;
        if verbose {
            println!("   ✅ manifest_{}.json 已写入 (web 兼容)", dbnum);
        }
    }

    let elapsed = start_time.elapsed();

    Ok(ParquetExportStats {
        instance_count: instance_rows.len(),
        geo_instance_count: geo_instance_rows.len(),
        tubing_count: tubing_rows.len(),
        transform_count: transform_rows.len(),
        aabb_count: aabb_row_data.len(),
        total_bytes,
        elapsed,
    })
}

// =============================================================================
// Cache → Parquet 导出
// =============================================================================

