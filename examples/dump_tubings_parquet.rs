use anyhow::{Result, anyhow};
use arrow_array::{Array, Float64Array, RecordBatch, StringArray, UInt32Array, UInt64Array};
use parquet::arrow::arrow_reader::ParquetRecordBatchReaderBuilder;
use std::collections::HashMap;
use std::fs::File;
use std::path::{Path, PathBuf};

fn batches(path: &Path) -> Result<Vec<RecordBatch>> {
    let reader = ParquetRecordBatchReaderBuilder::try_new(File::open(path)?)?.build()?;
    Ok(reader.collect::<std::result::Result<Vec<_>, _>>()?)
}

fn u64_col(batch: &RecordBatch, name: &str) -> Result<UInt64Array> {
    let col = batch
        .column_by_name(name)
        .ok_or_else(|| anyhow!("missing column {name}"))?;
    Ok(UInt64Array::from(col.to_data()))
}

fn u32_col(batch: &RecordBatch, name: &str) -> Result<UInt32Array> {
    let col = batch
        .column_by_name(name)
        .ok_or_else(|| anyhow!("missing column {name}"))?;
    Ok(UInt32Array::from(col.to_data()))
}

fn str_col(batch: &RecordBatch, name: &str) -> Result<StringArray> {
    let col = batch
        .column_by_name(name)
        .ok_or_else(|| anyhow!("missing column {name}"))?;
    Ok(StringArray::from(col.to_data()))
}

fn f64_col(batch: &RecordBatch, name: &str) -> Result<Float64Array> {
    let col = batch
        .column_by_name(name)
        .ok_or_else(|| anyhow!("missing column {name}"))?;
    Ok(Float64Array::from(col.to_data()))
}

fn main() -> Result<()> {
    let args = std::env::args().collect::<Vec<_>>();
    anyhow::ensure!(
        args.len() == 3,
        "usage: dump_tubings_parquet <parquet-dir> <owner-refno-u64>"
    );
    let dir = PathBuf::from(&args[1]);
    let wanted = args[2].parse::<u64>()?;
    let mut aabbs = HashMap::new();
    for batch in batches(&dir.join("aabb.parquet"))? {
        let hash = str_col(&batch, "aabb_hash")?;
        let min_x = f64_col(&batch, "min_x")?;
        let min_y = f64_col(&batch, "min_y")?;
        let min_z = f64_col(&batch, "min_z")?;
        let max_x = f64_col(&batch, "max_x")?;
        let max_y = f64_col(&batch, "max_y")?;
        let max_z = f64_col(&batch, "max_z")?;
        for i in 0..batch.num_rows() {
            aabbs.insert(
                hash.value(i).to_string(),
                [
                    min_x.value(i),
                    min_y.value(i),
                    min_z.value(i),
                    max_x.value(i),
                    max_y.value(i),
                    max_z.value(i),
                ],
            );
        }
    }
    for batch in batches(&dir.join("tubings.parquet"))? {
        let owner = u64_col(&batch, "owner_refno_u64")?;
        let tubi = u64_col(&batch, "tubi_refno_u64")?;
        let order = u32_col(&batch, "order")?;
        let hash = str_col(&batch, "aabb_hash")?;
        for i in 0..batch.num_rows() {
            if owner.value(i) == wanted {
                println!(
                    "order={} tubi={} aabb={:?}",
                    order.value(i),
                    tubi.value(i),
                    aabbs.get(hash.value(i))
                );
            }
        }
    }
    Ok(())
}
