use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};

use aios_core::RefnoEnum;
use aios_core::error::init_save_database_error;
use aios_core::model_primary_db;
use anyhow::Context;
use async_trait::async_trait;
use dashmap::DashMap;
use parry3d::bounding_volume::Aabb;

use super::super::manifold_bool::run_bool_worker_from_tasks;
use super::super::mesh_generate::{MeshResult, run_boolean_worker};
use super::super::mesh_state::flush_aabb_cache;
use super::super::pdms_inst::{self};
use super::super::pdms_inst_surreal;
use super::{
    BaseInstanceBatch, BooleanBridgeReport, BooleanBridgeRequest, CleanupRequest, FinalizeRequest,
    FinalizeSummary, InstRelateAabbBatch, MeshResultBatch, ModelWriterBackend, ModelWriterContext,
    ReconcileRequest, WriteBaseReport,
};
use crate::options::BooleanPipelineMode;

#[derive(Debug, Default)]
pub struct SurrealModelWriterBackend {
    context: OnceLock<ModelWriterContext>,
    /// v3 Phase F.1: 累积每 batch 收集到的 missing_neg_carriers，
    /// orchestrator 通过 `take_missing_neg_carriers` 一次性 drain。
    missing_neg_carriers: Mutex<Vec<RefnoEnum>>,
}

#[async_trait]
impl ModelWriterBackend for SurrealModelWriterBackend {
    fn name(&self) -> &'static str {
        "surreal"
    }

    async fn init(&self, context: &ModelWriterContext) -> anyhow::Result<()> {
        println!(
            "[model-writer:surreal] stage=init project={} use_surrealdb={} defer_db_write={} mode={}",
            context.project_name,
            context.use_surrealdb,
            context.defer_db_write,
            context.mode.as_str()
        );
        anyhow::ensure!(
            context.use_surrealdb,
            "Surreal model writer requires use_surrealdb=true (defense-in-depth)"
        );
        aios_core::rs_surreal::inst::init_model_tables()
            .await
            .context("model_writer surreal init_model_tables failed")?;
        let _ = self.context.set(context.clone());
        Ok(())
    }

    async fn cleanup(&self, request: CleanupRequest<'_>) -> anyhow::Result<()> {
        println!(
            "[model-writer:surreal] stage=cleanup seed_refnos={}",
            request.seed_refnos.len()
        );
        pdms_inst::pre_cleanup_for_regen(request.seed_refnos)
            .await
            .context("model_writer surreal legacy cleanup failed")?;
        pdms_inst_surreal::pre_cleanup_for_regen_surreal(request.seed_refnos)
            .await
            .context("model_writer surreal relation cleanup failed")?;
        println!(
            "[model-writer:surreal] stage=cleanup done seed_refnos={}",
            request.seed_refnos.len()
        );
        Ok(())
    }

    async fn write_base_batch(
        &self,
        batch: BaseInstanceBatch<'_>,
    ) -> anyhow::Result<WriteBaseReport> {
        println!(
            "[model-writer:surreal] stage=base batch={} inst_info={} inst_tubi={} geo_keys={}",
            batch.batch_id,
            batch.shape_insts.inst_info_map.len(),
            batch.shape_insts.inst_tubi_map.len(),
            batch.shape_insts.inst_geos_map.len()
        );
        let mesh_results: HashMap<u64, MeshResult> = HashMap::new();
        let report = pdms_inst::save_instance_data_with_report(
            batch.shape_insts,
            batch.replace_exist,
            &mesh_results,
            batch.mesh_aabb_map,
            batch.write_inst_relate_aabb,
        )
        .await
        .with_context(|| format!("model_writer surreal base batch {} failed", batch.batch_id))?;
        let missing_neg_count = report.missing_neg_carriers.len();
        if !report.missing_neg_carriers.is_empty() {
            let mut guard = self
                .missing_neg_carriers
                .lock()
                .expect("missing_neg_carriers mutex poisoned");
            guard.extend(report.missing_neg_carriers.iter().copied());
        }
        println!(
            "[model-writer:surreal] stage=base batch={} done missing_neg_candidates={}",
            batch.batch_id, missing_neg_count
        );
        Ok(WriteBaseReport {
            batch_id: batch.batch_id,
            missing_neg_count,
        })
    }

    async fn take_missing_neg_carriers(&self) -> anyhow::Result<Vec<RefnoEnum>> {
        let mut guard = self
            .missing_neg_carriers
            .lock()
            .expect("missing_neg_carriers mutex poisoned");
        let drained = std::mem::take(&mut *guard);
        Ok(drained)
    }

    async fn persist_mesh_results(&self, batch: MeshResultBatch<'_>) -> anyhow::Result<()> {
        if batch.file_mesh_state {
            flush_aabb_cache();
            println!(
                "[model-writer:surreal] stage=mesh_results batch={} file_mesh_state=true flushed_aabb_cache=true",
                batch.batch_id
            );
            return Ok(());
        }

        if batch.mesh_results.is_empty() {
            println!(
                "[model-writer:surreal] stage=mesh_results batch={} mesh_results=0",
                batch.batch_id
            );
            return Ok(());
        }

        let pts_written = save_pts_to_surreal_strict(batch.mesh_pts_map)
            .await
            .with_context(|| {
                format!(
                    "model_writer surreal mesh pts batch {} failed",
                    batch.batch_id
                )
            })?;
        let aabb_written = save_aabb_to_surreal_strict(batch.mesh_aabb_map)
            .await
            .with_context(|| {
                format!(
                    "model_writer surreal mesh aabb batch {} failed",
                    batch.batch_id
                )
            })?;

        let mut update_sql = String::new();
        for (geo_hash, mesh_result) in batch.mesh_results {
            update_sql.push_str(&mesh_result.to_update_sql(&geo_hash.to_string()));
        }

        if !update_sql.is_empty() {
            model_primary_db().query(&update_sql).await.map_err(|e| {
                let preview: String = update_sql.chars().take(500).collect();
                anyhow::anyhow!(
                    "model_writer surreal mesh result batch {} failed: error={}, sql_preview={}",
                    batch.batch_id,
                    e,
                    preview
                )
            })?;
        }

        println!(
            "[model-writer:surreal] stage=mesh_results batch={} mesh_results={} pts_rows={} aabb_rows={} update_sql_len={}",
            batch.batch_id,
            batch.mesh_results.len(),
            pts_written,
            aabb_written,
            update_sql.len()
        );
        Ok(())
    }

    async fn write_inst_relate_aabb(&self, batch: InstRelateAabbBatch<'_>) -> anyhow::Result<()> {
        let (aabb_rows_map, inst_relate_aabb_rows, inst_relate_aabb_ids) =
            pdms_inst::build_inst_relate_aabb_rows(
                batch.shape_insts,
                batch.mesh_results,
                batch.mesh_aabb_map,
            )?;
        let aabb_count = aabb_rows_map.len();
        let rel_count = inst_relate_aabb_rows.len();
        pdms_inst::save_inst_relate_aabb_rows(
            &aabb_rows_map,
            &inst_relate_aabb_rows,
            &inst_relate_aabb_ids,
        )
        .await
        .with_context(|| {
            format!(
                "model_writer surreal inst_relate_aabb batch {} failed",
                batch.batch_id
            )
        })?;
        println!(
            "[model-writer:surreal] stage=inst_relate_aabb batch={} aabb_rows={} relation_rows={}",
            batch.batch_id, aabb_count, rel_count
        );
        Ok(())
    }

    async fn reconcile_missing_neg(&self, request: ReconcileRequest<'_>) -> anyhow::Result<usize> {
        println!(
            "[model-writer:surreal] stage=reconcile_missing_neg all_refnos={} candidate_carriers={}",
            request.all_refnos.len(),
            request.candidate_carriers.len()
        );
        let inserted =
            pdms_inst::reconcile_missing_neg_relate(request.all_refnos, request.candidate_carriers)
                .await
                .context("model_writer surreal reconcile_missing_neg failed")?;
        println!(
            "[model-writer:surreal] stage=reconcile_missing_neg done inserted={}",
            inserted
        );
        Ok(inserted)
    }

    async fn run_boolean_bridge(
        &self,
        request: BooleanBridgeRequest,
    ) -> anyhow::Result<BooleanBridgeReport> {
        let Some(ctx) = self.context.get() else {
            return Ok(BooleanBridgeReport::skipped(
                "uninitialized",
                request.bool_tasks.len(),
                "init not called before run_boolean_bridge",
            ));
        };
        // v3 Phase F.2: db_option pulled from cached context (was an
        // explicit field on BooleanBridgeRequest before).
        let db_option = ctx.db_option.clone();
        match request.mode {
            BooleanPipelineMode::DbLegacy => {
                if ctx.use_surrealdb && !ctx.defer_db_write {
                    println!("[model-writer:surreal] stage=boolean_bridge pipeline=db_legacy");
                    run_boolean_worker(db_option, 100)
                        .await
                        .context("model_writer surreal db_legacy boolean bridge failed")?;
                    Ok(BooleanBridgeReport::db_legacy_executed())
                } else {
                    Ok(BooleanBridgeReport::skipped(
                        "db_legacy",
                        0,
                        "use_surrealdb/defer_db_write guard",
                    ))
                }
            }
            BooleanPipelineMode::MemoryTasks => {
                if !ctx.use_surrealdb {
                    return Ok(BooleanBridgeReport::skipped(
                        "memory_tasks",
                        request.bool_tasks.len(),
                        "use_surrealdb=false",
                    ));
                }
                println!(
                    "[model-writer:surreal] stage=boolean_bridge pipeline=memory_tasks total_tasks={}",
                    request.bool_tasks.len()
                );
                let report = run_bool_worker_from_tasks(request.bool_tasks, db_option, None)
                    .await
                    .context("model_writer surreal memory_tasks boolean bridge failed")?;
                Ok(report.into())
            }
        }
    }

    async fn finalize(&self, request: FinalizeRequest) -> anyhow::Result<FinalizeSummary> {
        println!(
            "[model-writer:surreal] stage=finalize total_batches={} completed_batches={} mesh_cache_hits={} mesh_new_generated={} missing_neg_candidates={}",
            request.total_batches,
            request.completed_batches,
            request.mesh_cache_hits,
            request.mesh_new_generated,
            request.missing_neg_candidates
        );
        Ok(FinalizeSummary {
            backend: self.name(),
            total_batches: request.total_batches,
            completed_batches: request.completed_batches,
        })
    }
}

/// SurrealDB record id 的安全包装。
///
/// 构造时强制 ASCII alphanum + `_` / `-`（注意：`:` 被排除以确保 raw_key
/// **不能**包含表前缀），禁止任意 String，避免 SQL 拼接被外部输入污染。
/// 当前 record id 来源是内部 mesh hash，该约束**不会**拒绝合法 key；如需
/// 扩展字符集（如 UTF-8 哈希），改这里。
///
/// 输出统一为 `table:⟨raw_key⟩` 形式（SurrealDB escaped record id 语法），
/// 避免不同分支产出 `table:raw_key` 与 `table:⟨raw_key⟩` 两种格式不一致。
struct SurrealRecordKey(String);

impl SurrealRecordKey {
    fn new(table: &'static str, raw_key: &str) -> anyhow::Result<Self> {
        anyhow::ensure!(
            raw_key
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-')),
            "SurrealRecordKey rejects raw_key (must be ASCII alphanum / `_` / `-`, no `:`) for table {}: {:?}",
            table,
            raw_key
        );
        Ok(Self(format!("{}:⟨{}⟩", table, raw_key)))
    }

    fn as_str(&self) -> &str {
        &self.0
    }
}

async fn save_aabb_to_surreal_strict(aabb_map: &DashMap<String, Aabb>) -> anyhow::Result<usize> {
    if aabb_map.is_empty() {
        return Ok(0);
    }

    let keys = aabb_map
        .iter()
        .map(|kv| kv.key().clone())
        .collect::<Vec<_>>();
    let mut written = 0usize;
    for chunk in keys.chunks(300) {
        let mut rows: Vec<String> = Vec::with_capacity(chunk.len());
        for k in chunk {
            let Some(v) = aabb_map.get(k) else {
                continue;
            };
            let d = serde_json::to_string(v.value())?;
            let id_key = SurrealRecordKey::new("aabb", k)?;
            rows.push(format!("{{'id':{}, 'd':{d}}}", id_key.as_str()));
        }
        if rows.is_empty() {
            continue;
        }
        let sql = format!("INSERT IGNORE INTO aabb [{}];", rows.join(","));
        if let Err(e) = model_primary_db().query(&sql).await {
            init_save_database_error(
                &format!("{sql}\n-- err: {e}"),
                &std::panic::Location::caller().to_string(),
            );
            anyhow::bail!("写入 mesh aabb 失败: {e}");
        }
        written += rows.len();
    }
    Ok(written)
}

async fn save_pts_to_surreal_strict(vec3_map: &DashMap<u64, String>) -> anyhow::Result<usize> {
    if vec3_map.is_empty() {
        return Ok(0);
    }

    let keys = vec3_map.iter().map(|kv| *kv.key()).collect::<Vec<_>>();
    let mut written = 0usize;
    for chunk in keys.chunks(100) {
        let mut rows: Vec<String> = Vec::with_capacity(chunk.len());
        for &k in chunk {
            let Some(v) = vec3_map.get(&k) else {
                continue;
            };
            let id_key = SurrealRecordKey::new("vec3", &k.to_string())?;
            rows.push(format!("{{'id':{}, 'd':{}}}", id_key.as_str(), v.value()));
        }
        if rows.is_empty() {
            continue;
        }
        let sql = format!("INSERT IGNORE INTO vec3 [{}];", rows.join(","));
        if let Err(e) = model_primary_db().query(&sql).await {
            init_save_database_error(
                &format!("{sql}\n-- err: {e}"),
                &std::panic::Location::caller().to_string(),
            );
            anyhow::bail!("写入 mesh pts/vec3 失败: {e}");
        }
        written += rows.len();
    }
    Ok(written)
}
