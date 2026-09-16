use std::path::Path;

use crate::web_server::managed_project_sites::{DeployValidationCheck, deploy_validation_check};

#[cfg(feature = "parquet-export")]
use std::{collections::HashSet, fs::File, path::PathBuf};

#[cfg(feature = "parquet-export")]
use anyhow::{Context, Result};
#[cfg(feature = "parquet-export")]
use arrow_array::{RecordBatch, StringArray, UInt64Array};
#[cfg(feature = "parquet-export")]
use arrow_schema::Schema;
#[cfg(feature = "parquet-export")]
use parquet::arrow::arrow_reader::ParquetRecordBatchReaderBuilder;

const SAMPLE_ROW_LIMIT: usize = 1000;
const LOOKUP_ROW_LIMIT: usize = 100_000;
const MESH_SAMPLE_LIMIT: usize = 100;
const DEFAULT_LOD_TAG: &str = "L1";

pub(crate) fn validate_dbnum_parquet_data(
    dbnum: u32,
    dbnum_dir: &Path,
    mesh_root: &Path,
) -> Vec<DeployValidationCheck> {
    #[cfg(feature = "parquet-export")]
    {
        validate_dbnum_parquet_data_impl(dbnum, dbnum_dir, mesh_root)
    }

    #[cfg(not(feature = "parquet-export"))]
    {
        let _ = dbnum_dir;
        let _ = mesh_root;
        vec![deploy_validation_check(
            format!("parquet_data_validation_{dbnum}"),
            format!("Parquet 数据一致性 {dbnum}"),
            "warning",
            "当前构建未启用 parquet-export，跳过 Parquet schema 和抽样一致性校验",
            None,
            None,
            None,
        )]
    }
}

#[cfg(feature = "parquet-export")]
fn validate_dbnum_parquet_data_impl(
    dbnum: u32,
    dbnum_dir: &Path,
    mesh_root: &Path,
) -> Vec<DeployValidationCheck> {
    let mut checks = Vec::new();
    let instances_path = dbnum_dir.join("instances.parquet");
    let geo_instances_path = dbnum_dir.join("geo_instances.parquet");
    let transforms_path = dbnum_dir.join("transforms.parquet");
    let aabb_path = dbnum_dir.join("aabb.parquet");

    checks.push(schema_check(
        dbnum,
        "parquet_instances_schema",
        "instances.parquet schema",
        &instances_path,
        &[
            "refno_str",
            "refno_u64",
            "noun",
            "trans_hash",
            "aabb_hash",
            "dbnum",
        ],
    ));
    checks.push(schema_check(
        dbnum,
        "parquet_geo_instances_schema",
        "geo_instances.parquet schema",
        &geo_instances_path,
        &[
            "refno_str",
            "refno_u64",
            "geo_index",
            "geo_hash",
            "geo_trans_hash",
        ],
    ));
    checks.push(schema_check(
        dbnum,
        "parquet_transforms_schema",
        "transforms.parquet schema",
        &transforms_path,
        &[
            "trans_hash",
            "m00",
            "m10",
            "m20",
            "m30",
            "m01",
            "m11",
            "m21",
            "m31",
            "m02",
            "m12",
            "m22",
            "m32",
            "m03",
            "m13",
            "m23",
            "m33",
        ],
    ));
    checks.push(schema_check(
        dbnum,
        "parquet_aabb_schema",
        "aabb.parquet schema",
        &aabb_path,
        &[
            "aabb_hash",
            "min_x",
            "min_y",
            "min_z",
            "max_x",
            "max_y",
            "max_z",
        ],
    ));

    checks.push(reference_sample_check(
        dbnum,
        &instances_path,
        &geo_instances_path,
        &transforms_path,
        &aabb_path,
    ));
    checks.push(mesh_sample_check(dbnum, &geo_instances_path, mesh_root));

    checks
}

#[cfg(feature = "parquet-export")]
fn schema_check(
    dbnum: u32,
    key_prefix: &str,
    label: &str,
    path: &Path,
    required_fields: &[&str],
) -> DeployValidationCheck {
    match parquet_schema(path) {
        Ok((schema, row_count)) => {
            let fields = schema
                .fields()
                .iter()
                .map(|field| field.name().as_str())
                .collect::<HashSet<_>>();
            let missing = required_fields
                .iter()
                .copied()
                .filter(|field| !fields.contains(field))
                .collect::<Vec<_>>();
            if missing.is_empty() {
                deploy_validation_check(
                    format!("{key_prefix}_{dbnum}"),
                    format!("{label} {dbnum}"),
                    "pass",
                    format!("schema 字段完整，rows={row_count}"),
                    Some(path.display().to_string()),
                    None,
                    None,
                )
            } else {
                deploy_validation_check(
                    format!("{key_prefix}_{dbnum}"),
                    format!("{label} {dbnum}"),
                    "blocking",
                    format!("schema 缺少字段: {}", missing.join(", ")),
                    Some(path.display().to_string()),
                    None,
                    None,
                )
            }
        }
        Err(err) => deploy_validation_check(
            format!("{key_prefix}_{dbnum}"),
            format!("{label} {dbnum}"),
            "blocking",
            format!("读取 Parquet schema 失败: {err:#}"),
            Some(path.display().to_string()),
            None,
            None,
        ),
    }
}

#[cfg(feature = "parquet-export")]
fn parquet_schema(path: &Path) -> Result<(Schema, i64)> {
    let file =
        File::open(path).with_context(|| format!("打开 Parquet 文件失败: {}", path.display()))?;
    let builder = ParquetRecordBatchReaderBuilder::try_new(file)
        .with_context(|| format!("创建 Parquet reader 失败: {}", path.display()))?;
    let row_count = builder.metadata().file_metadata().num_rows();
    Ok((builder.schema().as_ref().clone(), row_count))
}

#[cfg(feature = "parquet-export")]
fn reference_sample_check(
    dbnum: u32,
    instances_path: &Path,
    geo_instances_path: &Path,
    transforms_path: &Path,
    aabb_path: &Path,
) -> DeployValidationCheck {
    match reference_sample_check_result(
        instances_path,
        geo_instances_path,
        transforms_path,
        aabb_path,
    ) {
        Ok(summary) if summary.is_clean() => deploy_validation_check(
            format!("parquet_refs_sample_{dbnum}"),
            format!("Parquet 抽样引用一致性 {dbnum}"),
            "pass",
            format!(
                "抽样 instances={}、geo_instances={}，引用一致",
                summary.instances_sampled, summary.geo_instances_sampled
            ),
            Some(format!(
                "{} | {}",
                instances_path.display(),
                geo_instances_path.display()
            )),
            None,
            None,
        ),
        Ok(summary) => deploy_validation_check(
            format!("parquet_refs_sample_{dbnum}"),
            format!("Parquet 抽样引用一致性 {dbnum}"),
            "blocking",
            format!(
                "抽样发现引用缺失: missing_trans={} missing_aabb={} missing_geo_refno={}",
                summary.missing_trans, summary.missing_aabb, summary.missing_geo_refno
            ),
            Some(format!(
                "instances_sampled={}, geo_instances_sampled={}, lookup_cap={LOOKUP_ROW_LIMIT}",
                summary.instances_sampled, summary.geo_instances_sampled
            )),
            None,
            None,
        ),
        Err(err) => deploy_validation_check(
            format!("parquet_refs_sample_{dbnum}"),
            format!("Parquet 抽样引用一致性 {dbnum}"),
            "blocking",
            format!("抽样引用校验失败: {err:#}"),
            Some(format!(
                "{} | {}",
                instances_path.display(),
                geo_instances_path.display()
            )),
            None,
            None,
        ),
    }
}

#[cfg(feature = "parquet-export")]
#[derive(Default)]
struct ReferenceSampleSummary {
    instances_sampled: usize,
    geo_instances_sampled: usize,
    missing_trans: usize,
    missing_aabb: usize,
    missing_geo_refno: usize,
}

#[cfg(feature = "parquet-export")]
impl ReferenceSampleSummary {
    fn is_clean(&self) -> bool {
        self.missing_trans == 0 && self.missing_aabb == 0 && self.missing_geo_refno == 0
    }
}

#[cfg(feature = "parquet-export")]
fn reference_sample_check_result(
    instances_path: &Path,
    geo_instances_path: &Path,
    transforms_path: &Path,
    aabb_path: &Path,
) -> Result<ReferenceSampleSummary> {
    let transform_hashes = collect_string_values(transforms_path, "trans_hash", LOOKUP_ROW_LIMIT)?;
    let aabb_hashes = collect_string_values(aabb_path, "aabb_hash", LOOKUP_ROW_LIMIT)?;
    let instance_refnos = collect_u64_values(instances_path, "refno_u64", LOOKUP_ROW_LIMIT)?;

    let mut summary = ReferenceSampleSummary::default();
    for batch in read_sample_batches(instances_path, SAMPLE_ROW_LIMIT)? {
        let trans_hash = string_column(&batch, "trans_hash")?;
        let aabb_hash = string_column(&batch, "aabb_hash")?;
        for row in 0..batch.num_rows() {
            if summary.instances_sampled >= SAMPLE_ROW_LIMIT {
                break;
            }
            summary.instances_sampled += 1;
            let trans = trans_hash.value(row).trim();
            if !trans.is_empty() && !transform_hashes.contains(trans) {
                summary.missing_trans += 1;
            }
            let aabb = aabb_hash.value(row).trim();
            if !aabb.is_empty() && !aabb_hashes.contains(aabb) {
                summary.missing_aabb += 1;
            }
        }
    }

    for batch in read_sample_batches(geo_instances_path, SAMPLE_ROW_LIMIT)? {
        let refnos = u64_column(&batch, "refno_u64")?;
        for row in 0..batch.num_rows() {
            if summary.geo_instances_sampled >= SAMPLE_ROW_LIMIT {
                break;
            }
            summary.geo_instances_sampled += 1;
            if !instance_refnos.contains(&refnos.value(row)) {
                summary.missing_geo_refno += 1;
            }
        }
    }

    Ok(summary)
}

#[cfg(feature = "parquet-export")]
fn mesh_sample_check(
    dbnum: u32,
    geo_instances_path: &Path,
    mesh_root: &Path,
) -> DeployValidationCheck {
    match collect_string_values(geo_instances_path, "geo_hash", MESH_SAMPLE_LIMIT) {
        Ok(geo_hashes) => {
            let mut checked = 0usize;
            let mut missing = Vec::new();
            for geo_hash in geo_hashes {
                if is_builtin_geo_hash(&geo_hash) {
                    continue;
                }
                checked += 1;
                if !mesh_candidates(mesh_root, &geo_hash)
                    .iter()
                    .any(|path| path.exists())
                {
                    missing.push(geo_hash);
                }
            }
            if missing.is_empty() {
                deploy_validation_check(
                    format!("mesh_refs_sample_{dbnum}"),
                    format!("Mesh 抽样引用 {dbnum}"),
                    "pass",
                    format!("抽样 {checked} 个 geo_hash，mesh 文件可匹配"),
                    Some(mesh_root.display().to_string()),
                    None,
                    None,
                )
            } else {
                deploy_validation_check(
                    format!("mesh_refs_sample_{dbnum}"),
                    format!("Mesh 抽样引用 {dbnum}"),
                    "warning",
                    format!("抽样发现 {} 个 geo_hash 缺少 mesh 文件", missing.len()),
                    Some(format!(
                        "mesh_root={}, sample_missing={}",
                        mesh_root.display(),
                        missing.into_iter().take(10).collect::<Vec<_>>().join(", ")
                    )),
                    None,
                    None,
                )
            }
        }
        Err(err) => deploy_validation_check(
            format!("mesh_refs_sample_{dbnum}"),
            format!("Mesh 抽样引用 {dbnum}"),
            "warning",
            format!("读取 geo_hash 抽样失败: {err:#}"),
            Some(geo_instances_path.display().to_string()),
            None,
            None,
        ),
    }
}

#[cfg(feature = "parquet-export")]
fn is_builtin_geo_hash(geo_hash: &str) -> bool {
    matches!(geo_hash.trim(), "0" | "1" | "2" | "3")
}

#[cfg(feature = "parquet-export")]
fn mesh_candidates(mesh_root: &Path, geo_hash: &str) -> [PathBuf; 3] {
    let lod_dir = mesh_root.join(format!("lod_{DEFAULT_LOD_TAG}"));
    [
        lod_dir.join(format!("{geo_hash}_{DEFAULT_LOD_TAG}.glb")),
        lod_dir.join(format!("{geo_hash}.glb")),
        mesh_root.join(format!("{geo_hash}.glb")),
    ]
}

#[cfg(feature = "parquet-export")]
fn read_sample_batches(path: &Path, limit: usize) -> Result<Vec<RecordBatch>> {
    let file =
        File::open(path).with_context(|| format!("打开 Parquet 文件失败: {}", path.display()))?;
    let reader = ParquetRecordBatchReaderBuilder::try_new(file)
        .with_context(|| format!("创建 Parquet reader 失败: {}", path.display()))?
        .with_batch_size(limit.max(1))
        .build()
        .with_context(|| format!("创建 Parquet batch reader 失败: {}", path.display()))?;
    let mut batches = Vec::new();
    let mut rows = 0usize;
    for batch in reader {
        let batch =
            batch.with_context(|| format!("读取 Parquet batch 失败: {}", path.display()))?;
        rows += batch.num_rows();
        batches.push(batch);
        if rows >= limit {
            break;
        }
    }
    Ok(batches)
}

#[cfg(feature = "parquet-export")]
fn collect_string_values(path: &Path, column: &str, limit: usize) -> Result<HashSet<String>> {
    let mut values = HashSet::new();
    for batch in read_sample_batches(path, limit)? {
        let array = string_column(&batch, column)?;
        for row in 0..batch.num_rows() {
            if values.len() >= limit {
                break;
            }
            let value = array.value(row).trim();
            if !value.is_empty() {
                values.insert(value.to_string());
            }
        }
    }
    Ok(values)
}

#[cfg(feature = "parquet-export")]
fn collect_u64_values(path: &Path, column: &str, limit: usize) -> Result<HashSet<u64>> {
    let mut values = HashSet::new();
    for batch in read_sample_batches(path, limit)? {
        let array = u64_column(&batch, column)?;
        for row in 0..batch.num_rows() {
            if values.len() >= limit {
                break;
            }
            values.insert(array.value(row));
        }
    }
    Ok(values)
}

#[cfg(feature = "parquet-export")]
fn column_index(batch: &RecordBatch, column: &str) -> Result<usize> {
    batch
        .schema()
        .fields()
        .iter()
        .position(|field| field.name() == column)
        .with_context(|| format!("缺少列: {column}"))
}

#[cfg(feature = "parquet-export")]
fn string_column<'a>(batch: &'a RecordBatch, column: &str) -> Result<&'a StringArray> {
    let index = column_index(batch, column)?;
    batch
        .column(index)
        .as_any()
        .downcast_ref::<StringArray>()
        .with_context(|| format!("列 {column} 不是 String 类型"))
}

#[cfg(feature = "parquet-export")]
fn u64_column<'a>(batch: &'a RecordBatch, column: &str) -> Result<&'a UInt64Array> {
    let index = column_index(batch, column)?;
    batch
        .column(index)
        .as_any()
        .downcast_ref::<UInt64Array>()
        .with_context(|| format!("列 {column} 不是 UInt64 类型"))
}
