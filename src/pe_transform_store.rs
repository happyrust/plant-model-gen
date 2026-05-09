use std::collections::HashMap;
use std::time::Instant;

use aios_core::rs_surreal::pe_transform::{
    PeTransformEntry, save_pe_transform_entries,
};
use aios_core::{RefnoEnum, SUL_DB, SurrealQueryExt};
use anyhow::{Context, Result};

use crate::options::{DbOptionExt, TransformReadBackend, TransformWriteBackend};

pub async fn save_entries_with_backend(
    db_option: &DbOptionExt,
    entries: &[PeTransformEntry],
) -> Result<()> {
    if entries.is_empty() {
        return Ok(());
    }

    match db_option.transform_write_backend {
        TransformWriteBackend::Surreal => {
            save_pe_transform_entries(entries).await?;
        }
        TransformWriteBackend::Parquet => {
            save_entries_to_parquet(db_option, entries)?;
        }
        TransformWriteBackend::DuckLake => {
            save_entries_to_parquet(db_option, entries)?;
            #[cfg(feature = "transform-store-ducklake")]
            register_ducklake(db_option).await?;
        }
        TransformWriteBackend::Dual => {
            save_pe_transform_entries(entries).await?;
            save_entries_to_parquet(db_option, entries)?;
            #[cfg(feature = "transform-store-ducklake")]
            if db_option.transform_write_backend.uses_ducklake() {
                register_ducklake(db_option).await?;
            }
        }
    }

    Ok(())
}

pub async fn load_entries_with_backend(
    db_option: &DbOptionExt,
    backend: TransformReadBackend,
    refnos: &[RefnoEnum],
) -> Result<Vec<PeTransformEntry>> {
    if refnos.is_empty() {
        return Ok(Vec::new());
    }

    match backend {
        TransformReadBackend::Auto | TransformReadBackend::Surreal => {
            load_entries_from_surreal(refnos).await
        }
        TransformReadBackend::Parquet | TransformReadBackend::DuckLake => {
            load_entries_from_parquet(db_option, refnos)
        }
        TransformReadBackend::Rkyv => load_entries_from_rkyv(refnos).await,
        TransformReadBackend::Memory => load_entries_from_memory(refnos),
    }
}

pub async fn clear_pe_transform_for_dbnums(dbnums: &[u32]) -> Result<usize> {
    if dbnums.is_empty() {
        return Ok(0);
    }

    let dbnum_list = dbnums
        .iter()
        .map(|d| d.to_string())
        .collect::<Vec<_>>()
        .join(",");

    let count_sql = format!(
        "SELECT count() AS total FROM pe_transform WHERE record::id(id) INSIDE \
         (SELECT VALUE record::id(id) FROM pe WHERE dbnum IN [{}]) GROUP ALL",
        dbnum_list
    );

    #[derive(serde::Deserialize, SurrealValue)]
    struct CountRow {
        total: usize,
    }

    let count: Option<CountRow> = SUL_DB
        .query_take(&count_sql, 0)
        .await
        .unwrap_or(None);
    let total = count.map(|c| c.total).unwrap_or(0);

    if total == 0 {
        return Ok(0);
    }

    let delete_sql = format!(
        "DELETE pe_transform WHERE record::id(id) INSIDE \
         (SELECT VALUE record::id(id) FROM pe WHERE dbnum IN [{}])",
        dbnum_list
    );
    SUL_DB
        .query(&delete_sql)
        .await
        .context("清理 pe_transform 失败")?;

    Ok(total)
}

#[derive(Debug, Clone)]
pub struct BackendCompareStats {
    pub backend: TransformReadBackend,
    pub loaded: usize,
    pub missing: usize,
    pub mismatched: usize,
    pub max_delta: f64,
    pub elapsed_ms: u128,
}

pub async fn compare_backends_for_dbnums(
    db_option: &DbOptionExt,
    dbnums: &[u32],
) -> Result<Vec<BackendCompareStats>> {
    if db_option.transform_compare_backends.is_empty() {
        return Ok(Vec::new());
    }

    let baseline_refnos = collect_refnos_for_dbnums(dbnums).await?;
    if baseline_refnos.is_empty() {
        return Ok(Vec::new());
    }

    let baseline_start = Instant::now();
    let baseline = load_entries_from_surreal(&baseline_refnos).await?;
    let baseline_ms = baseline_start.elapsed().as_millis();

    let baseline_map: HashMap<RefnoEnum, &PeTransformEntry> =
        baseline.iter().map(|e| (e.refno, e)).collect();

    let mut results = vec![BackendCompareStats {
        backend: TransformReadBackend::Surreal,
        loaded: baseline.len(),
        missing: baseline_refnos.len().saturating_sub(baseline.len()),
        mismatched: 0,
        max_delta: 0.0,
        elapsed_ms: baseline_ms,
    }];

    for &cmp_backend in &db_option.transform_compare_backends {
        let start = Instant::now();
        let entries = load_entries_with_backend(db_option, cmp_backend, &baseline_refnos)
            .await
            .unwrap_or_default();
        let elapsed_ms = start.elapsed().as_millis();

        let entry_map: HashMap<RefnoEnum, &PeTransformEntry> =
            entries.iter().map(|e| (e.refno, e)).collect();

        let mut missing = 0usize;
        let mut mismatched = 0usize;
        let mut max_delta = 0.0f64;

        for &refno in &baseline_refnos {
            let Some(bl) = baseline_map.get(&refno) else {
                continue;
            };
            let Some(cmp) = entry_map.get(&refno) else {
                missing += 1;
                continue;
            };

            let delta = transform_delta(bl, cmp);
            if delta > 1e-9 {
                mismatched += 1;
                max_delta = max_delta.max(delta);
            }
        }

        results.push(BackendCompareStats {
            backend: cmp_backend,
            loaded: entries.len(),
            missing,
            mismatched,
            max_delta,
            elapsed_ms,
        });
    }

    Ok(results)
}

// ── SurrealDB backend ──────────────────────────────────────────

use aios_core::rs_surreal::PlantTransform;
use surrealdb::types::SurrealValue;

#[derive(Debug, serde::Deserialize, SurrealValue)]
struct PeTransformSurrealRow {
    #[serde(default)]
    refno: Option<RefnoEnum>,
    #[serde(default)]
    local_trans: Option<PlantTransform>,
    #[serde(default)]
    world_trans: Option<PlantTransform>,
}

async fn load_entries_from_surreal(refnos: &[RefnoEnum]) -> Result<Vec<PeTransformEntry>> {
    const CHUNK: usize = 500;
    let mut out = Vec::with_capacity(refnos.len());

    for chunk in refnos.chunks(CHUNK) {
        let ids = chunk
            .iter()
            .map(|r| r.to_table_key("pe_transform"))
            .collect::<Vec<_>>()
            .join(",");
        let sql = format!(
            "SELECT meta::id(id) AS refno, local_trans.d AS local_trans, \
             world_trans.d AS world_trans FROM {}",
            ids
        );
        let rows: Vec<PeTransformSurrealRow> = SUL_DB
            .query_take(&sql, 0)
            .await
            .unwrap_or_default();

        for row in rows {
            let Some(refno) = row.refno else { continue };
            out.push(PeTransformEntry {
                refno,
                local: row.local_trans.map(|t| t.0),
                world: row.world_trans.map(|t| t.0),
            });
        }
    }

    Ok(out)
}

// ── Parquet backend ────────────────────────────────────────────

#[cfg(feature = "transform-store-parquet")]
fn save_entries_to_parquet(db_option: &DbOptionExt, entries: &[PeTransformEntry]) -> Result<()> {
    use polars::prelude::*;

    if entries.is_empty() {
        return Ok(());
    }

    let dir = db_option.get_transform_parquet_dir();
    std::fs::create_dir_all(&dir)
        .with_context(|| format!("创建 pe_transform parquet 目录失败: {}", dir.display()))?;

    let path = dir.join("pe_transform.parquet");
    let tmp_path = dir.join("pe_transform.parquet.tmp");

    let mut existing_df = if path.exists() {
        let file = std::fs::File::open(&path)?;
        Some(ParquetReader::new(file).finish()?)
    } else {
        None
    };

    let new_df = entries_to_dataframe(entries)?;

    let mut merged = if let Some(ref mut old) = existing_df {
        let mut stacked = old.vstack(&new_df)?;
        stacked
            .unique::<&[String], &String>(
                Some(&["refno".to_string()]),
                UniqueKeepStrategy::Last,
                None,
            )?
    } else {
        new_df
    };

    let file = std::fs::File::create(&tmp_path)?;
    ParquetWriter::new(file).finish(&mut merged)?;
    std::fs::rename(&tmp_path, &path)?;

    Ok(())
}

#[cfg(not(feature = "transform-store-parquet"))]
fn save_entries_to_parquet(_db_option: &DbOptionExt, _entries: &[PeTransformEntry]) -> Result<()> {
    anyhow::bail!("transform-store-parquet feature 未启用")
}

#[cfg(feature = "transform-store-parquet")]
fn load_entries_from_parquet(
    db_option: &DbOptionExt,
    refnos: &[RefnoEnum],
) -> Result<Vec<PeTransformEntry>> {
    use polars::prelude::*;
    use std::collections::HashSet;

    let path = db_option.get_transform_parquet_dir().join("pe_transform.parquet");
    if !path.exists() {
        return Ok(Vec::new());
    }

    let file = std::fs::File::open(&path)?;
    let df = ParquetReader::new(file).finish()?;

    let wanted: HashSet<String> = refnos.iter().map(|r| r.to_string()).collect();
    dataframe_to_entries(&df, Some(&wanted))
}

#[cfg(not(feature = "transform-store-parquet"))]
fn load_entries_from_parquet(
    _db_option: &DbOptionExt,
    _refnos: &[RefnoEnum],
) -> Result<Vec<PeTransformEntry>> {
    anyhow::bail!("transform-store-parquet feature 未启用")
}

#[cfg(feature = "transform-store-parquet")]
fn entries_to_dataframe(entries: &[PeTransformEntry]) -> Result<polars::prelude::DataFrame> {
    use polars::prelude::*;

    let refnos: Vec<String> = entries.iter().map(|e| e.refno.to_string()).collect();

    let local_json: Vec<Option<String>> = entries
        .iter()
        .map(|e| e.local.as_ref().and_then(|t| serde_json::to_string(t).ok()))
        .collect();
    let world_json: Vec<Option<String>> = entries
        .iter()
        .map(|e| e.world.as_ref().and_then(|t| serde_json::to_string(t).ok()))
        .collect();

    let df = df! {
        "refno" => &refnos,
        "local" => &local_json,
        "world" => &world_json,
    }?;

    Ok(df)
}

#[cfg(feature = "transform-store-parquet")]
fn dataframe_to_entries(
    df: &polars::prelude::DataFrame,
    filter_refnos: Option<&std::collections::HashSet<String>>,
) -> Result<Vec<PeTransformEntry>> {
    use aios_core::plant_transform::Transform;

    let refno_col = df.column("refno")?.str()?;
    let local_col = df.column("local")?.str()?;
    let world_col = df.column("world")?.str()?;

    let mut out = Vec::new();
    for i in 0..df.height() {
        let Some(refno_str) = refno_col.get(i) else {
            continue;
        };

        if let Some(filter) = filter_refnos {
            if !filter.contains(refno_str) {
                continue;
            }
        }

        let refno = RefnoEnum::from(refno_str);
        let local: Option<Transform> = local_col
            .get(i)
            .and_then(|s| serde_json::from_str(s).ok());
        let world: Option<Transform> = world_col
            .get(i)
            .and_then(|s| serde_json::from_str(s).ok());

        out.push(PeTransformEntry {
            refno,
            local,
            world,
        });
    }

    Ok(out)
}

// ── Rkyv backend ───────────────────────────────────────────────

#[cfg(feature = "gen_model")]
async fn load_entries_from_rkyv(refnos: &[RefnoEnum]) -> Result<Vec<PeTransformEntry>> {
    let world_map = crate::fast_model::gen_model::transform_rkyv_cache::query_world_transforms_from_pe_transform(refnos).await?;

    Ok(refnos
        .iter()
        .filter_map(|&refno| {
            let world = world_map.get(&refno).cloned();
            if world.is_some() {
                Some(PeTransformEntry {
                    refno,
                    local: None,
                    world,
                })
            } else {
                None
            }
        })
        .collect())
}

#[cfg(not(feature = "gen_model"))]
async fn load_entries_from_rkyv(_refnos: &[RefnoEnum]) -> Result<Vec<PeTransformEntry>> {
    anyhow::bail!("rkyv 读取后端需要 gen_model feature")
}

// ── Memory backend ─────────────────────────────────────────────

#[cfg(feature = "gen_model")]
fn load_entries_from_memory(refnos: &[RefnoEnum]) -> Result<Vec<PeTransformEntry>> {
    use crate::fast_model::gen_model::transform_cache::GLOBAL_TRANSFORM_CACHE;
    use crate::data_interface::db_meta_manager::db_meta;

    let _ = db_meta().ensure_loaded();

    let Some(cache) = GLOBAL_TRANSFORM_CACHE.get() else {
        return Ok(Vec::new());
    };

    Ok(refnos
        .iter()
        .filter_map(|&refno| {
            let dbnum = db_meta().get_dbnum_by_refno(refno)?;
            let world = cache.get_world_transform(dbnum, refno);
            let local = cache.get_local_transform(dbnum, refno);
            if world.is_some() || local.is_some() {
                Some(PeTransformEntry {
                    refno,
                    local,
                    world,
                })
            } else {
                None
            }
        })
        .collect())
}

#[cfg(not(feature = "gen_model"))]
fn load_entries_from_memory(_refnos: &[RefnoEnum]) -> Result<Vec<PeTransformEntry>> {
    anyhow::bail!("memory 读取后端需要 gen_model feature")
}

// ── DuckLake stub ──────────────────────────────────────────────

#[cfg(feature = "transform-store-ducklake")]
async fn register_ducklake(_db_option: &DbOptionExt) -> Result<()> {
    Ok(())
}

// ── Helpers ────────────────────────────────────────────────────

async fn collect_refnos_for_dbnums(dbnums: &[u32]) -> Result<Vec<RefnoEnum>> {
    let mut all_refnos = Vec::new();

    for &dbnum in dbnums {
        let sql = format!(
            "SELECT VALUE record::id(id) FROM pe WHERE dbnum = {}",
            dbnum
        );
        let refnos: Vec<RefnoEnum> = SUL_DB
            .query_take(&sql, 0)
            .await
            .unwrap_or_default();
        all_refnos.extend(refnos);
    }

    Ok(all_refnos)
}

fn transform_delta(a: &PeTransformEntry, b: &PeTransformEntry) -> f64 {
    let mut max = 0.0f64;

    if let (Some(wa), Some(wb)) = (&a.world, &b.world) {
        let d = single_transform_delta(wa, wb);
        max = max.max(d);
    } else if a.world.is_some() != b.world.is_some() {
        return f64::INFINITY;
    }

    if let (Some(la), Some(lb)) = (&a.local, &b.local) {
        let d = single_transform_delta(la, lb);
        max = max.max(d);
    } else if a.local.is_some() != b.local.is_some() {
        return f64::INFINITY;
    }

    max
}

fn single_transform_delta(
    a: &aios_core::plant_transform::Transform,
    b: &aios_core::plant_transform::Transform,
) -> f64 {
    let td = (a.translation - b.translation).length() as f64;

    let qd = 1.0 - (a.rotation.dot(b.rotation) as f64).abs();

    let sd = (a.scale - b.scale).length() as f64;

    td.max(qd).max(sd)
}
