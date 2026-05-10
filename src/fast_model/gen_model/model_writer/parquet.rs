use std::fs::{self, File};
use std::io::{BufWriter, Write};
use std::path::{Path, PathBuf};

use anyhow::Context;
use serde::Serialize;

use super::CanonicalRawBatch;
use crate::fast_model::gen_model::canonical_records::CanonicalRawTable;

/// File-oriented canonical writer scaffold for the future Parquet backend.
///
/// This phase keeps the boundary isolated from production `ModelWriterBackend`
/// selection and emits JSON Lines files with the same table layout the Parquet
/// writer will own. Typed Parquet materialization remains gated to the next
/// phase so the current Surreal/default path is not affected by heavy optional
/// Parquet/Polars dependencies.
#[derive(Debug, Clone)]
pub struct CanonicalParquetWriter {
    config: CanonicalParquetWriterConfig,
}

#[derive(Debug, Clone)]
pub struct CanonicalParquetWriterConfig {
    pub output_dir: PathBuf,
    pub project_name: String,
    pub dbnum: u32,
}

#[derive(Debug, Clone, Serialize)]
pub struct CanonicalParquetBatchSummary {
    pub backend: &'static str,
    pub format: &'static str,
    pub limitation: &'static str,
    pub summary_path: String,
    pub project_name: String,
    pub dbnum: u32,
    pub batch_id: u64,
    pub raw_root: String,
    pub total_rows: usize,
    pub tables: Vec<CanonicalParquetTableSummary>,
}

#[derive(Debug, Clone, Serialize)]
pub struct CanonicalParquetTableSummary {
    pub table: &'static str,
    pub rows: usize,
    pub path: String,
    pub limitation: Option<&'static str>,
}

impl CanonicalParquetWriter {
    pub fn new(config: CanonicalParquetWriterConfig) -> Self {
        Self { config }
    }

    pub fn write_raw_batch(
        &self,
        batch_id: u64,
        batch: &CanonicalRawBatch,
    ) -> anyhow::Result<CanonicalParquetBatchSummary> {
        let raw_root = self
            .config
            .output_dir
            .join("model_writer_storage")
            .join("raw");
        fs::create_dir_all(&raw_root).with_context(|| {
            format!(
                "failed to create canonical parquet raw root {}",
                raw_root.display()
            )
        })?;

        let mut tables = Vec::new();
        tables.push(self.write_table(
            &raw_root,
            CanonicalRawTable::RawInstInfo,
            batch_id,
            &batch.inst_info,
        )?);
        tables.push(self.write_table(
            &raw_root,
            CanonicalRawTable::RawInstRelate,
            batch_id,
            &batch.inst_relate,
        )?);
        tables.push(self.write_table(
            &raw_root,
            CanonicalRawTable::RawInstGeo,
            batch_id,
            &batch.inst_geo,
        )?);
        tables.push(self.write_table(
            &raw_root,
            CanonicalRawTable::RawGeoRelate,
            batch_id,
            &batch.geo_relate,
        )?);
        tables.push(self.write_table(
            &raw_root,
            CanonicalRawTable::RawTubiInfo,
            batch_id,
            &batch.tubi_info,
        )?);
        tables.push(self.write_table(
            &raw_root,
            CanonicalRawTable::RawTubiRelate,
            batch_id,
            &batch.tubi_relate,
        )?);
        tables.push(self.write_table(
            &raw_root,
            CanonicalRawTable::RawNegRelate,
            batch_id,
            &batch.neg_relate,
        )?);
        tables.push(self.write_table(
            &raw_root,
            CanonicalRawTable::RawNgmrRelate,
            batch_id,
            &batch.ngmr_relate,
        )?);
        tables.push(self.write_table(
            &raw_root,
            CanonicalRawTable::RawAabb,
            batch_id,
            &batch.aabb,
        )?);
        tables.push(self.write_table(
            &raw_root,
            CanonicalRawTable::RawTrans,
            batch_id,
            &batch.trans,
        )?);
        tables.push(self.write_table(
            &raw_root,
            CanonicalRawTable::RawVec3,
            batch_id,
            &batch.vec3,
        )?);
        tables.push(self.write_table(
            &raw_root,
            CanonicalRawTable::RawInstRelateAabb,
            batch_id,
            &batch.inst_relate_aabb,
        )?);
        tables.push(self.write_table(
            &raw_root,
            CanonicalRawTable::RawRefnoAssocIndex,
            batch_id,
            &batch.refno_assoc_index,
        )?);

        let summary_path = self.summary_file_path(batch_id);
        let total_rows = tables.iter().map(|table| table.rows).sum();
        let summary = CanonicalParquetBatchSummary {
            backend: "parquet-canonical",
            format: "jsonl-fallback",
            limitation: "typed parquet materialization is intentionally deferred; this scaffold preserves canonical table boundaries and row counts",
            summary_path: stable_path_string(&summary_path),
            project_name: self.config.project_name.clone(),
            dbnum: self.config.dbnum,
            batch_id,
            raw_root: stable_path_string(&raw_root),
            total_rows,
            tables,
        };
        self.write_summary(&summary_path, &summary)?;
        Ok(summary)
    }

    fn write_table<T: Serialize>(
        &self,
        raw_root: &Path,
        table: CanonicalRawTable,
        batch_id: u64,
        rows: &[T],
    ) -> anyhow::Result<CanonicalParquetTableSummary> {
        let path = self.table_file_path(raw_root, table, batch_id);
        write_json_lines(&path, rows)?;
        Ok(CanonicalParquetTableSummary {
            table: table.as_str(),
            rows: rows.len(),
            path: stable_path_string(&path),
            limitation: table.phase1_limitation(),
        })
    }

    fn table_file_path(&self, raw_root: &Path, table: CanonicalRawTable, batch_id: u64) -> PathBuf {
        raw_root
            .join(table.as_str())
            .join(format!("project_name={}", self.config.project_name))
            .join(format!("dbnum={}", self.config.dbnum))
            .join(format!("batch_{batch_id}.jsonl"))
    }

    pub fn summary_file_path(&self, batch_id: u64) -> PathBuf {
        self.config
            .output_dir
            .join("model_writer_storage")
            .join("summary")
            .join(format!("project_name={}", self.config.project_name))
            .join(format!("dbnum={}", self.config.dbnum))
            .join(format!("batch_{batch_id}.json"))
    }

    fn write_summary(
        &self,
        path: &Path,
        summary: &CanonicalParquetBatchSummary,
    ) -> anyhow::Result<()> {
        let summary_dir = path.parent().with_context(|| {
            format!(
                "failed to determine canonical parquet summary dir for {}",
                path.display()
            )
        })?;
        fs::create_dir_all(summary_dir).with_context(|| {
            format!(
                "failed to create canonical parquet summary dir {}",
                summary_dir.display()
            )
        })?;
        let file = File::create(path)
            .with_context(|| format!("failed to create summary file {}", path.display()))?;
        serde_json::to_writer_pretty(BufWriter::new(file), summary)
            .with_context(|| format!("failed to write summary file {}", path.display()))
    }
}

fn stable_path_string(path: &Path) -> String {
    path.display().to_string().replace('\\', "/")
}

fn write_json_lines<T: Serialize>(path: &Path, rows: &[T]) -> anyhow::Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("failed to create table dir {}", parent.display()))?;
    }
    let file = File::create(path)
        .with_context(|| format!("failed to create table file {}", path.display()))?;
    let mut writer = BufWriter::new(file);
    for row in rows {
        serde_json::to_writer(&mut writer, row)
            .with_context(|| format!("failed to serialize row for {}", path.display()))?;
        writer
            .write_all(b"\n")
            .with_context(|| format!("failed to write row for {}", path.display()))?;
    }
    writer
        .flush()
        .with_context(|| format!("failed to flush table file {}", path.display()))
}
