//! dbnum 级实例导出 Parquet
//!
//! 从 SurrealDB 读取 inst_relate / geo_relate / tubi_relate / trans / aabb 数据，
//! 生成多表 Parquet 文件组，供前端 DuckDB (含 duckdb-wasm) 直接查询。
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
    ensure_tree_index_exists, load_index_with_large_stack, TreeIndexManager,
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
    name: String,
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
    let s = r.to_string().replace('/', "_");
    // 将 "ref0_sesno" 格式转换为一个唯一的 u64
    // 方法: ref0 * 1_000_000 + sesno
    // 注意：这里的 ref0 不是 dbnum，仅用于生成唯一 ID
    let parts: Vec<&str> = s.split('_').collect();
    if parts.len() == 2 {
        let db = parts[0].parse::<u64>().unwrap_or(0);
        let ses = parts[1].parse::<u64>().unwrap_or(0);
        db * 1_000_000 + ses
    } else {
        // fallback: 使用 hash
        use std::collections::hash_map::DefaultHasher;
        use std::hash::{Hash, Hasher};
        let mut hasher = DefaultHasher::new();
        s.hash(&mut hasher);
        hasher.finish()
    }
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
        Field::new("name", DataType::Utf8, true),
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
            Arc::new(StringArray::from(rows.iter().map(|r| Some(r.name.as_str())).collect::<Vec<_>>())) as ArrayRef,
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
// SurrealDB 查询结构体（复用 export_prepack_lod 中的定义）
// =============================================================================

use serde::{Deserialize, Serialize};
use surrealdb::types::{self as surrealdb_types, SurrealValue};

#[derive(Clone, Debug, Serialize, Deserialize, SurrealValue)]
struct InstRelateRow {
    pub owner_refno: Option<RefnoEnum>,
    pub owner_type: Option<String>,
    pub refno: RefnoEnum,
    pub noun: Option<String>,
    pub name: Option<String>,
    pub aabb_hash: Option<String>,
    pub spec_value: Option<i64>,
}

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

/// 使用 TreeIndex refno 列表分批查询 inst_relate
async fn query_inst_relate_rows(
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
                fn::default_full_name(in) as name,
                IF in->inst_relate_aabb[0].out != NONE THEN record::id(in->inst_relate_aabb[0].out) END as aabb_hash,
                spec_value as spec_value
            FROM inst_relate
            WHERE in IN [{pe_list}]
            "#
        );

        let mut chunk_rows: Vec<InstRelateRow> =
            aios_core::SUL_DB.query_take(&sql, 0).await?;
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

    for owners_chunk in owner_refnos.chunks(50) {
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

        let mut resp = aios_core::SUL_DB.query_response(&sql_batch).await?;
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
    use aios_core::SUL_DB;

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

        let rows: Vec<TransQueryRow> = SUL_DB.query_take(&sql, 0).await.unwrap_or_default();
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
    use aios_core::SUL_DB;

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

        let rows: Vec<AabbQueryRow> = SUL_DB.query_take(&sql, 0).await.unwrap_or_default();
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

    // =========================================================================
    // 1. 使用 TreeIndex 获取 refno 列表
    // =========================================================================
    if verbose {
        println!("🔍 加载 TreeIndex...");
    }
    let tree_manager = TreeIndexManager::with_default_dir(vec![dbnum]);
    let tree_dir = tree_manager.tree_dir().to_path_buf();
    let tree_path = tree_dir.join(format!("{}.tree", dbnum));

    ensure_tree_index_exists(dbnum, &tree_dir)
        .await
        .with_context(|| format!("按需生成 TreeIndex 失败: {}", tree_path.display()))?;

    let tree_index = load_index_with_large_stack(&tree_dir, dbnum)
        .with_context(|| format!("加载 TreeIndex 失败: {}", tree_path.display()))?;

    let mut all_refnos: Vec<RefnoEnum> = if let Some(root) = root_refno {
        use crate::fast_model::query_compat::query_deep_visible_inst_refnos;
        if verbose {
            println!("🔍 查询 {} 的可见实例节点...", root);
        }
        query_deep_visible_inst_refnos(root).await?
    } else {
        tree_index
            .all_refnos()
            .into_iter()
            .map(RefnoEnum::from)
            .collect()
    };
    all_refnos.sort_by_key(|r| r.to_string());

    if verbose {
        println!("✅ TreeIndex 加载完成，refno 数量: {}", all_refnos.len());
    }

    // =========================================================================
    // 2. 分批查询 inst_relate
    // =========================================================================
    if verbose {
        println!("🔍 按 TreeIndex refno 查询 inst_relate...");
    }
    let inst_rows = query_inst_relate_rows(&all_refnos, verbose).await?;
    if verbose {
        println!("✅ inst_relate 命中记录: {}", inst_rows.len());
    }

    // 按 owner 分组
    struct ChildInfo {
        refno: RefnoEnum,
        noun: String,
        name: String,
        aabb_hash: Option<String>,
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

        let child = ChildInfo {
            refno: row.refno,
            noun: row.noun.unwrap_or_default(),
            name: row.name.unwrap_or_default().trim().trim_start_matches('/').to_string(),
            aabb_hash: row.aabb_hash,
            spec_value: row.spec_value.unwrap_or(0),
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
                    println!("✅ 查询到 {} 个 refno 有几何体实例", export_inst_map.len());
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
                .or_else(|| child.aabb_hash.clone())
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
                name: child.name.clone(),
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

                tubing_rows.push(TubingRow {
                    tubi_refno_str: tubi.leave.to_string(),
                    tubi_refno_u64: refno_to_u64(&tubi.leave),
                    owner_refno_str: owner_refno.to_string(),
                    owner_refno_u64: refno_to_u64(owner_refno),
                    order: index,
                    geo_hash,
                    trans_hash,
                    aabb_hash,
                    spec_value: tubi.spec_value.unwrap_or(0),
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
            .or_else(|| child.aabb_hash.clone())
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
            name: child.name.clone(),
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
    let transform_rows = query_trans_rows(&trans_hashes, &unit_converter, verbose).await?;
    let aabb_row_data = query_aabb_rows(&aabb_hashes, &unit_converter, verbose).await?;

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
    if !instance_rows.is_empty() {
        let batch = build_instances_batch(&instance_rows)?;
        let path = output_dir.join("instances.parquet");
        let size = write_parquet(&path, &batch)?;
        total_bytes += size;
        if verbose {
            println!("   ✅ instances.parquet: {} 行, {} 字节", instance_rows.len(), size);
        }
    }

    // geo_instances.parquet
    if !geo_instance_rows.is_empty() {
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
    if !tubing_rows.is_empty() {
        let batch = build_tubings_batch(&tubing_rows)?;
        let path = output_dir.join("tubings.parquet");
        let size = write_parquet(&path, &batch)?;
        total_bytes += size;
        if verbose {
            println!("   ✅ tubings.parquet: {} 行, {} 字节", tubing_rows.len(), size);
        }
    }

    // transforms.parquet
    if !transform_rows.is_empty() {
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
    if !aabb_row_data.is_empty() {
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

/// 从 model cache 导出指定 dbnum 的实例数据为多表 Parquet 格式
///
/// 与 `export_dbnum_instances_parquet()` 输出**完全相同 schema** 的 Parquet 文件，
/// 但数据源是 model cache 而非 SurrealDB，适用于 cache-only 模式。
///
/// # 输出文件
/// - `instances.parquet`
/// - `geo_instances.parquet`
/// - `tubings.parquet`
/// - `transforms.parquet`
/// - `aabb.parquet`
/// - `manifest.json`
pub async fn export_dbnum_instances_parquet_from_cache(
    dbnum: u32,
    output_dir: &Path,
    cache_dir: &Path,
    mesh_dir: Option<&Path>,
    mesh_lod_tag: Option<&str>,
    verbose: bool,
    target_unit: Option<LengthUnit>,
) -> Result<ParquetExportStats> {
    use aios_core::geometry::{EleGeosInfo, EleInstGeosData};
    use aios_core::Transform;
    use glam::DMat4;
    use parry3d::bounding_volume::{Aabb, BoundingVolume};
    use parry3d::math::Point;
    use sha2::{Digest, Sha256};

    let start_time = std::time::Instant::now();
    let target = target_unit.unwrap_or(LengthUnit::Millimeter);
    let unit_converter = UnitConverter::new(LengthUnit::Millimeter, target);

    if verbose {
        println!(
            "🚀 [cache→parquet] 开始导出 dbnum={} 的实例数据，目标单位: {:?}",
            dbnum, target
        );
        println!("   - 缓存目录: {}", cache_dir.display());
    }

    fs::create_dir_all(output_dir)
        .with_context(|| format!("创建输出目录失败: {}", output_dir.display()))?;
    let mesh_base_dir = mesh_dir
        .map(normalize_mesh_base_dir)
        .unwrap_or_else(|| PathBuf::from("assets/meshes"));

    // =========================================================================
    // 1. 从 per-refno 缓存加载数据
    // =========================================================================
    let cache_manager =
        crate::fast_model::instance_cache::InstanceCacheManager::new(cache_dir).await?;
    let refnos = cache_manager.list_refnos(dbnum);
    if refnos.is_empty() {
        return Err(anyhow::anyhow!(
            "缓存中未找到 dbnum={} 的实例数据（请先生成模型并写入缓存）",
            dbnum
        ));
    }

    let mut inst_info_map: HashMap<RefnoEnum, EleGeosInfo> = HashMap::new();
    let mut inst_tubi_map: HashMap<RefnoEnum, EleGeosInfo> = HashMap::new();
    let mut inst_geos_map: HashMap<String, EleInstGeosData> = HashMap::new();
    let mut neg_relate_map: HashMap<RefnoEnum, Vec<RefnoEnum>> = HashMap::new();

    let mut seen_inst_keys: std::collections::HashSet<String> = std::collections::HashSet::new();
    for &refno in &refnos {
        let Some(cached) = cache_manager.get_inst_info(dbnum, refno).await else {
            continue;
        };
        inst_info_map.insert(refno, cached.info.clone());
        if cached.info.tubi.is_some() {
            inst_tubi_map.insert(refno, cached.info.clone());
        }
        if !cached.neg_relates.is_empty() {
            neg_relate_map.insert(refno, cached.neg_relates.clone());
        }
        // inst_geos 按 inst_key 去重
        if !cached.inst_key.is_empty() && seen_inst_keys.insert(cached.inst_key.clone()) {
            if let Some(geos) = cache_manager.get_inst_geos(dbnum, &cached.inst_key).await {
                inst_geos_map.insert(cached.inst_key.clone(), geos.geos_data);
            }
        }
    }

    if verbose {
        println!(
            "✅ 缓存加载完成: inst_info={}, inst_geos={}, inst_tubi={}, refnos={}",
            inst_info_map.len(),
            inst_geos_map.len(),
            inst_tubi_map.len(),
            refnos.len(),
        );
    }

    // =========================================================================
    // 辅助函数
    // =========================================================================
    fn cache_hash_json_value(value: &serde_json::Value) -> String {
        let mut hasher = Sha256::new();
        let bytes = serde_json::to_vec(value).unwrap_or_default();
        hasher.update(bytes);
        hex::encode(hasher.finalize())
    }

    fn cache_to_dmat4(t: &Transform) -> DMat4 {
        let mat = t.to_matrix();
        let cols = mat.to_cols_array();
        let mut cols64 = [0f64; 16];
        for i in 0..16 {
            cols64[i] = cols[i] as f64;
        }
        DMat4::from_cols_array(&cols64)
    }

    fn cache_mat4_to_cols(mat: &DMat4, uc: &UnitConverter) -> [f64; 16] {
        let cols = mat.to_cols_array();
        let scale = uc.conversion_factor() as f64;
        let mut out = cols;
        // 平移列（列3的前3个分量）做单位换算
        out[12] *= scale;
        out[13] *= scale;
        out[14] *= scale;
        out
    }

    fn cache_insert_trans(
        trans_table: &mut HashMap<String, [f64; 16]>,
        mat: &DMat4,
        uc: &UnitConverter,
    ) -> String {
        let cols = cache_mat4_to_cols(mat, uc);
        let value = json!(cols.to_vec());
        let hash = cache_hash_json_value(&value);
        trans_table.entry(hash.clone()).or_insert(cols);
        hash
    }

    fn cache_insert_aabb(
        aabb_table: &mut HashMap<String, AabbRow>,
        aabb: &Aabb,
        uc: &UnitConverter,
    ) -> String {
        let scale = uc.conversion_factor() as f64;
        let row = AabbRow {
            aabb_hash: String::new(), // 占位，后面填
            min_x: aabb.mins.x as f64 * scale,
            min_y: aabb.mins.y as f64 * scale,
            min_z: aabb.mins.z as f64 * scale,
            max_x: aabb.maxs.x as f64 * scale,
            max_y: aabb.maxs.y as f64 * scale,
            max_z: aabb.maxs.z as f64 * scale,
        };
        let value = json!([row.min_x, row.min_y, row.min_z, row.max_x, row.max_y, row.max_z]);
        let hash = cache_hash_json_value(&value);
        aabb_table.entry(hash.clone()).or_insert(AabbRow {
            aabb_hash: hash.clone(),
            ..row
        });
        hash
    }

    fn cache_resolve_aabb(
        info: &EleGeosInfo,
        inst_geos: &EleInstGeosData,
        mesh_dir: Option<&Path>,
        mesh_lod_tag: Option<&str>,
        mesh_aabb_cache: &mut HashMap<u64, Aabb>,
    ) -> Option<Aabb> {
        use crate::fast_model::shared;

        if let Some(aabb) = info.aabb.clone() {
            return Some(aabb);
        }
        if let Some(aabb) = inst_geos.aabb.clone() {
            return Some(aabb);
        }
        let mut merged: Option<Aabb> = None;
        for inst in &inst_geos.insts {
            let world_t = info.get_geo_world_transform(inst);
            let world_aabb = if let Some(local_aabb) = inst.aabb {
                shared::aabb_apply_transform(&local_aabb, &world_t)
            } else {
                let points = inst.geo_param.key_points();
                if points.is_empty() {
                    // 尝试从 mesh 文件加载 AABB
                    if let (Some(mdir), Some(lod)) = (mesh_dir, mesh_lod_tag) {
                        if let Some(local_aabb) =
                            cache_load_geo_aabb_from_mesh(inst.geo_hash, mdir, lod, mesh_aabb_cache)
                        {
                            shared::aabb_apply_transform(&local_aabb, &world_t)
                        } else {
                            continue;
                        }
                    } else {
                        continue;
                    }
                } else {
                    let mut aabb = Aabb::new_invalid();
                    for p in &points {
                        let wp = world_t.transform_point(p.0);
                        aabb.take_point(Point::new(wp.x, wp.y, wp.z));
                    }
                    aabb
                }
            };
            let ext = world_aabb.extents().magnitude();
            if ext.is_nan() || ext.is_infinite() {
                continue;
            }
            merged = match merged {
                None => Some(world_aabb),
                Some(mut current) => {
                    current.merge(&world_aabb);
                    Some(current)
                }
            };
        }
        merged
    }

    fn cache_load_geo_aabb_from_mesh(
        geo_hash: u64,
        mesh_dir: &Path,
        lod_tag: &str,
        cache: &mut HashMap<u64, Aabb>,
    ) -> Option<Aabb> {
        if let Some(aabb) = cache.get(&geo_hash) {
            return Some(*aabb);
        }
        let base_dir = if mesh_dir
            .file_name()
            .map(|n| n.to_string_lossy().starts_with("lod_"))
            .unwrap_or(false)
        {
            mesh_dir.parent().unwrap_or(mesh_dir)
        } else {
            mesh_dir
        };
        let lod_dir = if mesh_dir
            .file_name()
            .map(|n| n.to_string_lossy().starts_with("lod_"))
            .unwrap_or(false)
        {
            mesh_dir.to_path_buf()
        } else {
            base_dir.join(format!("lod_{}", lod_tag))
        };
        let gh = geo_hash.to_string();
        let candidates = [
            lod_dir.join(format!("{}_{}.glb", gh, lod_tag)),
            lod_dir.join(format!("{}.glb", gh)),
            base_dir.join(format!("{}.glb", gh)),
        ];
        for path in candidates {
            if path.exists() {
                if let Ok(aabb) = cache_load_glb_aabb(&path) {
                    cache.insert(geo_hash, aabb);
                    return Some(aabb);
                }
            }
        }
        None
    }

    fn cache_load_glb_aabb(path: &Path) -> anyhow::Result<Aabb> {
        use std::io::BufReader;
        let file = std::fs::File::open(path)?;
        let reader = BufReader::new(file);
        let glb = gltf::Gltf::from_reader(reader)?;
        let mut aabb = Aabb::new_invalid();
        let mut has = false;
        for mesh in glb.meshes() {
            for primitive in mesh.primitives() {
                let reader = primitive.reader(|_| glb.blob.as_ref().map(|b| b.as_slice()));
                if let Some(iter) = reader.read_positions() {
                    for v in iter {
                        aabb.take_point(Point::new(v[0], v[1], v[2]));
                        has = true;
                    }
                }
            }
        }
        if !has {
            anyhow::bail!("GLB 无顶点数据");
        }
        Ok(aabb)
    }

    // =========================================================================
    // 2. 加载 TreeIndex（用于获取 noun/name）
    // =========================================================================
    let tree_manager = TreeIndexManager::with_default_dir(vec![dbnum]);
    let tree_dir = tree_manager.tree_dir().to_path_buf();
    let tree_index = load_index_with_large_stack(&tree_dir, dbnum).ok();

    // =========================================================================
    // 2.5 从 TreeIndex 构建 default_name 映射（与 fn::default_name 一致）
    // =========================================================================
    let order_map: HashMap<aios_core::pdms_types::RefU64, usize> = if let Some(ref idx) = tree_index {
        let all_u64s = idx.all_refnos();
        let mut children_by_owner: HashMap<aios_core::pdms_types::RefU64, Vec<aios_core::pdms_types::RefU64>> = HashMap::new();
        for r in &all_u64s {
            if let Some(meta) = idx.node_meta(*r) {
                if meta.owner.0 != 0 {
                    children_by_owner.entry(meta.owner).or_default().push(*r);
                }
            }
        }
        for children in children_by_owner.values_mut() {
            children.sort_by_key(|r| r.0);
        }
        let mut map = HashMap::new();
        for children in children_by_owner.values() {
            for (i, r) in children.iter().enumerate() {
                map.insert(*r, i);
            }
        }
        map
    } else {
        HashMap::new()
    };

    // =========================================================================
    // 3. 按 owner_type 分组
    // =========================================================================
    let mut grouped_children: HashMap<RefnoEnum, Vec<RefnoEnum>> = HashMap::new();
    let mut ungrouped: Vec<RefnoEnum> = Vec::new();

    for (refno, info) in &inst_info_map {
        let owner_type = info.owner_type.to_ascii_uppercase();
        if matches!(owner_type.as_str(), "BRAN" | "HANG" | "EQUI")
            && info.owner_refno != RefnoEnum::default()
        {
            grouped_children
                .entry(info.owner_refno)
                .or_default()
                .push(*refno);
        } else {
            ungrouped.push(*refno);
        }
    }

    // =========================================================================
    // 4. 构建 Parquet 行数据
    // =========================================================================
    let mut instance_rows: Vec<InstanceRow> = Vec::new();
    let mut geo_instance_rows: Vec<GeoInstanceRow> = Vec::new();
    let mut tubing_rows: Vec<TubingRow> = Vec::new();
    let mut trans_table: HashMap<String, [f64; 16]> = HashMap::new();
    let mut aabb_table: HashMap<String, AabbRow> = HashMap::new();
    let mut mesh_aabb_cache: HashMap<u64, Aabb> = HashMap::new();
    let mut owner_refnos_by_hash: HashMap<String, HashSet<String>> = HashMap::new();
    let mut row_count_by_hash: HashMap<String, usize> = HashMap::new();

    // 处理所有 inst_info_map 中的 refno（grouped + ungrouped 统一处理）
    let all_refnos: Vec<RefnoEnum> = inst_info_map.keys().copied().collect();

    for refno in &all_refnos {
        let Some(info) = inst_info_map.get(refno) else {
            continue;
        };

        let inst_key = info.get_inst_key();
        let inst_geos = inst_geos_map.get(&inst_key);

        // 跳过没有几何数据的实例
        let has_geos = inst_geos.map_or(false, |g| !g.insts.is_empty());
        if !has_geos {
            continue;
        }
        let inst_geos = inst_geos.unwrap();

        // 计算 world transform hash
        let world_mat = cache_to_dmat4(&info.world_transform);
        let trans_hash = cache_insert_trans(&mut trans_table, &world_mat, &unit_converter);

        // 计算 aabb hash
        let aabb_hash = cache_resolve_aabb(info, inst_geos, mesh_dir, mesh_lod_tag, &mut mesh_aabb_cache)
            .map(|aabb| cache_insert_aabb(&mut aabb_table, &aabb, &unit_converter))
            .unwrap_or_default();

        // 判断 has_neg
        let has_neg = neg_relate_map.contains_key(refno)
            || inst_geos.insts.iter().any(|g| !g.cata_neg_refnos.is_empty());

        // 获取 noun/name（name 为空时生成 default_name: "{NOUN} {order+1}"）
        let (noun, name) = tree_index
            .as_ref()
            .and_then(|idx| {
                idx.node_meta(refno.refno()).map(|meta| {
                    let noun_str = aios_core::tool::db_tool::db1_dehash(meta.noun);
                    let order = order_map.get(&refno.refno()).copied().unwrap_or(0);
                    let default_name = format!("{} {}", noun_str, order + 1);
                    (noun_str, default_name)
                })
            })
            .unwrap_or_default();

        let owner_refno = if info.owner_refno != RefnoEnum::default() {
            Some(info.owner_refno)
        } else {
            None
        };

        instance_rows.push(InstanceRow {
            refno_str: refno.to_string(),
            refno_u64: refno_to_u64(refno),
            noun,
            name,
            owner_refno_str: owner_refno.map(|r| r.to_string()),
            owner_refno_u64: owner_refno.map(|r| refno_to_u64(&r)),
            owner_noun: info.owner_type.to_ascii_uppercase(),
            trans_hash,
            aabb_hash,
            spec_value: 0,
            has_neg,
            dbnum,
        });

        // geo_instances
        for (geo_idx, inst) in inst_geos.insts.iter().enumerate() {
            let geo_mat = cache_to_dmat4(&inst.geo_transform);
            let geo_trans_hash = cache_insert_trans(&mut trans_table, &geo_mat, &unit_converter);

            geo_instance_rows.push(GeoInstanceRow {
                refno_str: refno.to_string(),
                refno_u64: refno_to_u64(refno),
                geo_index: geo_idx as u32,
                geo_hash: inst.geo_hash.to_string(),
                geo_trans_hash,
            });
            record_geo_hash_usage(
                &inst.geo_hash.to_string(),
                &refno.to_string(),
                &mut owner_refnos_by_hash,
                &mut row_count_by_hash,
            );
        }
    }

    // =========================================================================
    // 5. 处理 tubi（管道段）
    // =========================================================================
    for (tubi_refno, tubi_info) in &inst_tubi_map {
        let world_mat = cache_to_dmat4(&tubi_info.world_transform);
        let trans_hash = cache_insert_trans(&mut trans_table, &world_mat, &unit_converter);

        let aabb_hash = tubi_info
            .aabb
            .as_ref()
            .map(|aabb| cache_insert_aabb(&mut aabb_table, aabb, &unit_converter))
            .unwrap_or_default();

        let owner_refno = if tubi_info.owner_refno != RefnoEnum::default() {
            tubi_info.owner_refno
        } else {
            *tubi_refno
        };

        let inst_key = tubi_info.get_inst_key();
        let geo_hash = inst_geos_map
            .get(&inst_key)
            .and_then(|g| g.insts.first())
            .map(|inst| inst.geo_hash.to_string())
            .unwrap_or_default();

        let index = tubi_info
            .tubi
            .as_ref()
            .and_then(|t| t.index)
            .and_then(|v| u32::try_from(v).ok())
            .unwrap_or(0);

        tubing_rows.push(TubingRow {
            tubi_refno_str: tubi_refno.to_string(),
            tubi_refno_u64: refno_to_u64(tubi_refno),
            owner_refno_str: owner_refno.to_string(),
            owner_refno_u64: refno_to_u64(&owner_refno),
            order: index,
            geo_hash: geo_hash.clone(),
            trans_hash,
            aabb_hash,
            spec_value: 0,
            dbnum,
        });
        record_geo_hash_usage(
            &geo_hash,
            &owner_refno.to_string(),
            &mut owner_refnos_by_hash,
            &mut row_count_by_hash,
        );
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
    // 6. 写入 Parquet 文件（复用已有 schema + write_parquet）
    // =========================================================================
    let mut total_bytes: u64 = 0;

    if !instance_rows.is_empty() {
        let batch = build_instances_batch(&instance_rows)?;
        let path = output_dir.join("instances.parquet");
        let size = write_parquet(&path, &batch)?;
        total_bytes += size;
        if verbose {
            println!(
                "   ✅ instances.parquet: {} 行, {} 字节",
                instance_rows.len(),
                size
            );
        }
    }

    if !geo_instance_rows.is_empty() {
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

    if !tubing_rows.is_empty() {
        let batch = build_tubings_batch(&tubing_rows)?;
        let path = output_dir.join("tubings.parquet");
        let size = write_parquet(&path, &batch)?;
        total_bytes += size;
        if verbose {
            println!(
                "   ✅ tubings.parquet: {} 行, {} 字节",
                tubing_rows.len(),
                size
            );
        }
    }

    // 构建 TransformRow
    let transform_rows: Vec<TransformRow> = trans_table
        .into_iter()
        .map(|(hash, cols)| TransformRow {
            trans_hash: hash,
            m00: cols[0],  m10: cols[1],  m20: cols[2],  m30: cols[3],
            m01: cols[4],  m11: cols[5],  m21: cols[6],  m31: cols[7],
            m02: cols[8],  m12: cols[9],  m22: cols[10], m32: cols[11],
            m03: cols[12], m13: cols[13], m23: cols[14], m33: cols[15],
        })
        .collect();

    if !transform_rows.is_empty() {
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

    // 构建 AabbRow
    let aabb_row_data: Vec<AabbRow> = aabb_table.into_values().collect();

    if !aabb_row_data.is_empty() {
        let batch = build_aabb_batch(&aabb_row_data)?;
        let path = output_dir.join("aabb.parquet");
        let size = write_parquet(&path, &batch)?;
        total_bytes += size;
        if verbose {
            println!(
                "   ✅ aabb.parquet: {} 行, {} 字节",
                aabb_row_data.len(),
                size
            );
        }
    }

    // =========================================================================
    // 7. 写入 manifest.json
    // =========================================================================
    let generated_at = Utc::now().to_rfc3339_opts(SecondsFormat::Millis, true);
    let manifest = json!({
        "version": 1,
        "format": "parquet",
        "source": "model_cache",
        "generated_at": generated_at,
        "dbnum": dbnum,
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
    fs::write(
        &manifest_path,
        serde_json::to_string_pretty(&manifest)?,
    )?;
    if verbose {
        println!("   ✅ manifest.json 已写入");
    }

    let elapsed = start_time.elapsed();

    if verbose {
        println!(
            "\n🎉 [cache→parquet] dbnum={} 导出完成，耗时 {:.1}s",
            dbnum,
            elapsed.as_secs_f64()
        );
    }

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

