//! 模型生成编排器
//!
//! 负责协调整个模型生成流程：
//! - IndexTree 单管线路由（Full / Manual / Debug / Incremental）
//! - 几何体生成、Mesh 生成、布尔运算的编排
//! - 增量更新、手动 refno、调试模式的处理
//! - 空间索引和截图捕获的触发
use crate::data_interface::increment_record::IncrGeoUpdateLog;
use crate::data_interface::sesno_increment::get_changes_at_sesno;
use crate::fast_model::export_model::export_prepack_lod::export_instances_json_for_dbnos;
use crate::fast_model::export_model::export_prepack_lod::export_instances_json_for_refnos_grouped_by_dbno;
use crate::fast_model::export_model::export_prepack_lod::export_prepack_lod_for_refnos;
use crate::fast_model::unit_converter::LengthUnit;
use crate::fast_model::utils::{save_aabb_to_surreal, save_pts_to_surreal};
use aios_core::RefnoEnum;
use std::collections::{BTreeSet, HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Instant;
// use crate::fast_model::capture::capture_refnos_if_enabled; // removed on foyer-cache-cleanup
use crate::data_interface::db_meta_manager::db_meta;
use crate::fast_model::gen_model::boolean_task::{BooleanTask, BooleanTaskAccumulator};
use crate::fast_model::gen_model::manifold_bool::run_bool_worker_from_tasks;
use crate::fast_model::gen_model::mesh_state::{flush_aabb_cache, use_file_mesh_state};
use crate::fast_model::mesh_generate::{
    MeshResult, query_existing_meshed_inst_geo_ids, run_boolean_worker,
};
use crate::fast_model::pdms_inst::{
    build_inst_relate_aabb_rows, reconcile_missing_neg_relate, save_inst_relate_aabb_rows,
    save_instance_data_with_report,
};
use crate::options::{BooleanPipelineMode, DbOptionExt, MeshFormat};
use dashmap::DashMap;
use flume::{Receiver, Sender};
use tokio::sync::{OwnedSemaphorePermit, Semaphore};

#[cfg(feature = "parquet-export")]
use crate::fast_model::export_model::ParquetStreamWriter;

use super::cache_miss_report;
use super::config::IndexTreeConfig;
use super::errors::{IndexTreeError, Result};
use super::index_tree_mode::gen_index_tree_geos_optimized;
use super::models::NounCategory;
use crate::fast_model::gen_model::tree_index_manager::TreeIndexManager;
use aios_core::tool::db_tool::db1_hash;

/// 按 dbnum 拆分一个 batch，保证写入 InstanceCache 时“一个 batch 只落到一个 dbnum 分桶”。
///
/// 说明：
/// - 这里不尝试“从 ref0 推 dbnum”，必须通过 TreeIndexManager 映射。
/// - 若某个 refno 无法映射 dbnum：直接返回 Err（避免悄然写错桶）。
pub(crate) async fn split_shape_instances_by_dbnum(
    shape_insts: &aios_core::geometry::ShapeInstancesData,
) -> anyhow::Result<HashMap<u32, aios_core::geometry::ShapeInstancesData>> {
    use aios_core::geometry::ShapeInstancesData;
    let mut out: HashMap<u32, ShapeInstancesData> = HashMap::new();
    let mut cache: HashMap<RefnoEnum, u32> = HashMap::new();
    let mut missing_by_source: HashMap<&'static str, usize> = HashMap::new();
    let mut missing_refnos: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();

    async fn get_dbnum_cached(
        refno: RefnoEnum,
        source: &'static str,
        cache: &mut HashMap<RefnoEnum, u32>,
        missing_by_source: &mut HashMap<&'static str, usize>,
        missing_refnos: &mut std::collections::BTreeSet<String>,
    ) -> Option<u32> {
        if let Some(v) = cache.get(&refno) {
            return Some(*v);
        }
        let dbnum = TreeIndexManager::resolve_dbnum_for_refno(refno).ok();
        if let Some(dbnum) = dbnum {
            cache.insert(refno, dbnum);
            return Some(dbnum);
        }
        *missing_by_source.entry(source).or_insert(0) += 1;
        missing_refnos.insert(refno.to_string());
        None
    }

    fn summarize_missing_sources(missing_by_source: &HashMap<&'static str, usize>) -> String {
        let mut parts: Vec<String> = missing_by_source
            .iter()
            .map(|(k, v)| format!("{k}:{v}"))
            .collect();
        parts.sort();
        parts.join(", ")
    }

    fn summarize_missing_samples(
        missing_refnos: &std::collections::BTreeSet<String>,
        max_n: usize,
    ) -> String {
        missing_refnos
            .iter()
            .take(max_n)
            .cloned()
            .collect::<Vec<_>>()
            .join(", ")
    }

    // inst_info
    for (refno, info) in shape_insts.inst_info_map.iter() {
        let refno = *refno;
        let Some(dbnum) = get_dbnum_cached(
            refno,
            "inst_info.key",
            &mut cache,
            &mut missing_by_source,
            &mut missing_refnos,
        )
        .await
        else {
            continue;
        };
        out.entry(dbnum)
            .or_insert_with(ShapeInstancesData::default)
            .inst_info_map
            .insert(refno, info.clone());
    }

    // inst_tubi
    for (refno, tubi) in shape_insts.inst_tubi_map.iter() {
        let refno = *refno;
        let Some(dbnum) = get_dbnum_cached(
            refno,
            "inst_tubi.key",
            &mut cache,
            &mut missing_by_source,
            &mut missing_refnos,
        )
        .await
        else {
            continue;
        };
        out.entry(dbnum)
            .or_insert_with(ShapeInstancesData::default)
            .inst_tubi_map
            .insert(refno, tubi.clone());
    }

    // inst_geos：每条 geos_data 都绑定一个 refno（元素），直接按 geos_data.refno 分桶。
    for (inst_key, geos_data) in shape_insts.inst_geos_map.iter() {
        let inst_key = inst_key.clone();
        let geos_data = geos_data.clone();
        let Some(dbnum) = get_dbnum_cached(
            geos_data.refno,
            "inst_geos.refno",
            &mut cache,
            &mut missing_by_source,
            &mut missing_refnos,
        )
        .await
        else {
            continue;
        };
        out.entry(dbnum)
            .or_insert_with(ShapeInstancesData::default)
            .inst_geos_map
            .insert(inst_key, geos_data);
    }

    // neg_relate / ngmr_neg_relate：按 key(refno) 分桶
    for (refno, v) in &shape_insts.neg_relate_map {
        let Some(dbnum) = get_dbnum_cached(
            *refno,
            "neg_relate.key",
            &mut cache,
            &mut missing_by_source,
            &mut missing_refnos,
        )
        .await
        else {
            continue;
        };
        out.entry(dbnum)
            .or_insert_with(ShapeInstancesData::default)
            .neg_relate_map
            .insert(*refno, v.clone());
    }
    for (refno, v) in &shape_insts.ngmr_neg_relate_map {
        let Some(dbnum) = get_dbnum_cached(
            *refno,
            "ngmr_neg_relate.key",
            &mut cache,
            &mut missing_by_source,
            &mut missing_refnos,
        )
        .await
        else {
            continue;
        };
        out.entry(dbnum)
            .or_insert_with(ShapeInstancesData::default)
            .ngmr_neg_relate_map
            .insert(*refno, v.clone());
    }

    if !missing_refnos.is_empty() {
        let source_summary = summarize_missing_sources(&missing_by_source);
        let sample = summarize_missing_samples(&missing_refnos, 8);
        return Err(anyhow::anyhow!(
            "缺少 ref0->dbnum 映射: unique_refnos={}, sources=[{}], sample=[{}]",
            missing_refnos.len(),
            source_summary,
            sample
        ));
    }

    Ok(out)
}

#[derive(Debug, Clone)]
enum GenerationScope {
    Full,
    Manual { roots: Vec<RefnoEnum> },
    Debug { roots: Vec<RefnoEnum> },
    Incremental { log: IncrGeoUpdateLog },
}

fn decide_generation_scope(
    manual_refnos: &[RefnoEnum],
    debug_roots: &[RefnoEnum],
    has_incr_log: bool,
    incr_visible_roots: &[RefnoEnum],
    incr_updates: Option<&IncrGeoUpdateLog>,
) -> GenerationScope {
    let has_manual = !manual_refnos.is_empty();
    let has_debug = !debug_roots.is_empty();
    if has_manual && !has_debug && !has_incr_log {
        return GenerationScope::Manual {
            roots: manual_refnos.to_vec(),
        };
    }

    if has_debug && !has_manual && !has_incr_log {
        return GenerationScope::Debug {
            roots: debug_roots.to_vec(),
        };
    }

    if has_incr_log && !has_manual && !has_debug {
        return GenerationScope::Incremental {
            log: incr_updates.cloned().unwrap_or_default(),
        };
    }

    if has_manual || has_debug || has_incr_log {
        let mut merged: HashSet<RefnoEnum> = HashSet::new();
        merged.extend(manual_refnos.iter().copied());
        merged.extend(debug_roots.iter().copied());
        merged.extend(incr_visible_roots.iter().copied());
        return GenerationScope::Manual {
            roots: merged.into_iter().collect(),
        };
    }

    GenerationScope::Full
}

async fn collect_db_write_failures(db_write_handles: Vec<tokio::task::JoinHandle<bool>>) -> usize {
    let mut db_write_failures = 0usize;
    for h in db_write_handles {
        match h.await {
            Ok(true) => {}
            Ok(false) => db_write_failures += 1,
            Err(e) => {
                eprintln!("等待写库任务失败: {}", e);
                db_write_failures += 1;
            }
        }
    }
    db_write_failures
}

fn ensure_no_db_write_failures(db_write_failures: usize) -> anyhow::Result<()> {
    if db_write_failures > 0 {
        return Err(anyhow::anyhow!(
            "SurrealDB 批量写入存在失败任务: {}",
            db_write_failures
        ));
    }
    Ok(())
}

#[derive(Debug, Default)]
struct InsertHandleReport {
    batch_cnt: u64,
    bool_tasks: Vec<BooleanTask>,
}

#[derive(Debug, Clone)]
struct PipelineBatch {
    batch_id: u64,
    shape_insts: Arc<aios_core::geometry::ShapeInstancesData>,
    batch_started_at: Instant,
}

#[derive(Debug, Clone)]
struct BatchMeshOutput {
    batch_id: u64,
    shape_insts: Arc<aios_core::geometry::ShapeInstancesData>,
    mesh_results: HashMap<u64, MeshResult>,
    mesh_task_count: usize,
    mesh_cache_hits: usize,
    mesh_new_generated: usize,
    mesh_ms: u128,
    mesh_wait_ms: u128,
    batch_started_at: Instant,
}

#[derive(Debug, Clone, Copy)]
struct BaseWriteMetrics {
    base_wait_ms: u128,
    base_write_ms: u128,
}

#[derive(Debug, Clone)]
struct JoinedBatchOutput {
    batch_id: u64,
    shape_insts: Arc<aios_core::geometry::ShapeInstancesData>,
    mesh_results: HashMap<u64, MeshResult>,
    mesh_task_count: usize,
    mesh_cache_hits: usize,
    mesh_new_generated: usize,
    base_write_ms: u128,
    base_wait_ms: u128,
    mesh_ms: u128,
    mesh_wait_ms: u128,
    batch_started_at: Instant,
}

#[derive(Debug, Default)]
struct BatchStageJoiner {
    pending_mesh_outputs: HashMap<u64, BatchMeshOutput>,
    pending_base_metrics: HashMap<u64, BaseWriteMetrics>,
}

impl BatchStageJoiner {
    fn push_mesh_output(&mut self, batch: BatchMeshOutput) -> Option<JoinedBatchOutput> {
        let batch_id = batch.batch_id;
        if let Some(base_metrics) = self.pending_base_metrics.remove(&batch_id) {
            return Some(Self::join_batch(batch, base_metrics));
        }
        self.pending_mesh_outputs.insert(batch_id, batch);
        None
    }

    fn push_base_metrics(
        &mut self,
        batch_id: u64,
        base_wait_ms: u128,
        base_write_ms: u128,
    ) -> Option<JoinedBatchOutput> {
        let base_metrics = BaseWriteMetrics {
            base_wait_ms,
            base_write_ms,
        };
        if let Some(batch) = self.pending_mesh_outputs.remove(&batch_id) {
            return Some(Self::join_batch(batch, base_metrics));
        }
        self.pending_base_metrics.insert(batch_id, base_metrics);
        None
    }

    fn join_batch(batch: BatchMeshOutput, base_metrics: BaseWriteMetrics) -> JoinedBatchOutput {
        JoinedBatchOutput {
            batch_id: batch.batch_id,
            shape_insts: batch.shape_insts,
            mesh_results: batch.mesh_results,
            mesh_task_count: batch.mesh_task_count,
            mesh_cache_hits: batch.mesh_cache_hits,
            mesh_new_generated: batch.mesh_new_generated,
            base_write_ms: base_metrics.base_write_ms,
            base_wait_ms: base_metrics.base_wait_ms,
            mesh_ms: batch.mesh_ms,
            mesh_wait_ms: batch.mesh_wait_ms,
            batch_started_at: batch.batch_started_at,
        }
    }

    fn is_empty(&self) -> bool {
        self.pending_mesh_outputs.is_empty() && self.pending_base_metrics.is_empty()
    }

    fn pending_counts(&self) -> (usize, usize) {
        (
            self.pending_mesh_outputs.len(),
            self.pending_base_metrics.len(),
        )
    }
}

#[derive(Debug)]
struct BatchCompletion {
    batch_id: u64,
    mesh_task_count: usize,
    mesh_cache_hits: usize,
    mesh_new_generated: usize,
    base_write_ms: u128,
    base_wait_ms: u128,
    mesh_ms: u128,
    mesh_wait_ms: u128,
    inst_aabb_ms: u128,
    inst_aabb_wait_ms: u128,
    total_ms: u128,
}

async fn acquire_with_wait(
    semaphore: Arc<Semaphore>,
) -> anyhow::Result<(OwnedSemaphorePermit, u128)> {
    let wait_start = Instant::now();
    let permit = semaphore
        .acquire_owned()
        .await
        .map_err(|e| anyhow::anyhow!("获取 semaphore 失败: {}", e))?;
    Ok((permit, wait_start.elapsed().as_millis()))
}

async fn run_batch_sink(
    receiver: Receiver<aios_core::geometry::ShapeInstancesData>,
    base_writer_sender: Sender<PipelineBatch>,
    mesh_stage_sender: Sender<PipelineBatch>,
    touched_refnos: Arc<std::sync::Mutex<HashSet<RefnoEnum>>>,
) -> anyhow::Result<InsertHandleReport> {
    let mut batch_cnt: u64 = 0;
    let mut bool_accumulator = BooleanTaskAccumulator::default();

    while let Ok(shape_insts) = receiver.recv_async().await {
        batch_cnt += 1;
        let batch = PipelineBatch {
            batch_id: batch_cnt,
            shape_insts: Arc::new(shape_insts),
            batch_started_at: Instant::now(),
        };

        {
            let mut guard = touched_refnos.lock().unwrap();
            for r in batch.shape_insts.inst_info_map.keys() {
                guard.insert(*r);
            }
            for r in batch.shape_insts.inst_tubi_map.keys() {
                guard.insert(*r);
            }
        }

        bool_accumulator.merge_batch(&batch.shape_insts);
        let base_send_start = Instant::now();
        base_writer_sender.send_async(batch.clone()).await?;
        let base_send_wait_ms = base_send_start.elapsed().as_millis();
        if base_send_wait_ms > 0 {
            println!(
                "[batch_stage] batch={} stage=sink target=base_writer send_wait_ms={} inst_cnt={}",
                batch.batch_id,
                base_send_wait_ms,
                batch.shape_insts.inst_cnt()
            );
        }

        let mesh_send_start = Instant::now();
        mesh_stage_sender.send_async(batch.clone()).await?;
        let mesh_send_wait_ms = mesh_send_start.elapsed().as_millis();
        if mesh_send_wait_ms > 0 {
            println!(
                "[batch_stage] batch={} stage=sink target=mesh_stage send_wait_ms={} inst_cnt={}",
                batch.batch_id,
                mesh_send_wait_ms,
                batch.shape_insts.inst_cnt()
            );
        }
    }

    drop(base_writer_sender);
    drop(mesh_stage_sender);

    Ok(InsertHandleReport {
        batch_cnt,
        bool_tasks: bool_accumulator.build_tasks(),
    })
}

async fn run_base_writer(
    receiver: Receiver<PipelineBatch>,
    result_sender: Sender<(u64, u128, u128)>,
    replace_exist: bool,
    base_write_semaphore: Arc<Semaphore>,
    mesh_aabb_map: Arc<DashMap<String, parry3d::bounding_volume::Aabb>>,
    missing_neg_carriers: Arc<std::sync::Mutex<HashSet<RefnoEnum>>>,
) -> anyhow::Result<()> {
    let mut handles = Vec::new();
    while let Ok(batch) = receiver.recv_async().await {
        let semaphore = base_write_semaphore.clone();
        let mesh_aabb_map = mesh_aabb_map.clone();
        let result_sender = result_sender.clone();
        let missing_neg_carriers = missing_neg_carriers.clone();
        handles.push(tokio::spawn(async move {
            let (permit, wait_ms) = acquire_with_wait(semaphore).await?;
            let base_start = Instant::now();
            let save_report = save_instance_data_with_report(
                &batch.shape_insts,
                replace_exist,
                &HashMap::new(),
                &mesh_aabb_map,
                false,
            )
            .await?;
            if !save_report.missing_neg_carriers.is_empty() {
                let mut guard = missing_neg_carriers.lock().unwrap();
                guard.extend(save_report.missing_neg_carriers.iter().copied());
            }
            let base_ms = base_start.elapsed().as_millis();
            drop(permit);
            println!(
                "[batch_stage] batch={} stage=base wait_ms={} base_write_ms={} missing_neg_candidates={}",
                batch.batch_id,
                wait_ms,
                base_ms,
                save_report.missing_neg_carriers.len()
            );
            result_sender
                .send_async((batch.batch_id, wait_ms, base_ms))
                .await?;
            Ok::<(), anyhow::Error>(())
        }));
    }

    for handle in handles {
        handle.await.map_err(|e| anyhow::anyhow!(e))??;
    }
    drop(result_sender);
    Ok(())
}

async fn run_mesh_stage(
    receiver: Receiver<PipelineBatch>,
    output_sender: Sender<BatchMeshOutput>,
    mesh_compute_semaphore: Arc<Semaphore>,
    db_option: DbOptionExt,
    replace_exist: bool,
    gen_mesh: bool,
    mesh_aabb_map: Arc<DashMap<String, parry3d::bounding_volume::Aabb>>,
    mesh_pts_map: Arc<DashMap<u64, String>>,
) -> anyhow::Result<()> {
    let deduper = Arc::new(crate::fast_model::mesh_generate::RecentGeoDeduper::new(
        200_000,
    ));
    if gen_mesh && !replace_exist {
        crate::fast_model::preload_mesh_cache();
        let ids = query_existing_meshed_inst_geo_ids();
        let count = ids.len();
        deduper.preload(ids);
        println!(
            "[mesh_pipeline] 预加载 {} 个已 meshed inst_geo ID 到去重器 (size={})",
            count,
            deduper.len()
        );
    } else if gen_mesh {
        println!("[mesh_pipeline] replace_exist 模式，跳过去重器预加载，强制重新生成 mesh");
    }

    let mut handles = Vec::new();
    while let Ok(batch) = receiver.recv_async().await {
        let semaphore = mesh_compute_semaphore.clone();
        let deduper = deduper.clone();
        let mesh_aabb_map = mesh_aabb_map.clone();
        let mesh_pts_map = mesh_pts_map.clone();
        let output_sender = output_sender.clone();
        let db_option_inner = db_option.inner.clone();
        handles.push(tokio::spawn(async move {
            let (permit, wait_ms) = acquire_with_wait(semaphore).await?;
            let mesh_start = Instant::now();
            let tasks = crate::fast_model::mesh_generate::extract_mesh_tasks(&batch.shape_insts);
            let mesh_task_count = tasks.len();

            let mut mesh_results = HashMap::new();
            let mut mesh_cache_hits = 0usize;
            let mut mesh_new_generated = 0usize;

            if gen_mesh && !tasks.is_empty() {
                mesh_results = crate::fast_model::mesh_generate::generate_meshes_for_batch(
                    &tasks,
                    &db_option_inner,
                    &deduper,
                    &mesh_aabb_map,
                    &mesh_pts_map,
                )
                .await;
                mesh_cache_hits = mesh_results
                    .values()
                    .filter(|mr| mr.meshed && !mr.bad && mr.pts_hashes.is_empty())
                    .count();
                mesh_new_generated = mesh_results.len().saturating_sub(mesh_cache_hits);
            }

            let mesh_ms = mesh_start.elapsed().as_millis();
            drop(permit);
            println!(
                "[batch_stage] batch={} stage=mesh wait_ms={} mesh_ms={} mesh_tasks={} mesh_cache_hit={} mesh_new_generated={}",
                batch.batch_id, wait_ms, mesh_ms, mesh_task_count, mesh_cache_hits, mesh_new_generated
            );

            let output_send_start = Instant::now();
            output_sender
                .send_async(BatchMeshOutput {
                    batch_id: batch.batch_id,
                    shape_insts: batch.shape_insts,
                    mesh_results,
                    mesh_task_count,
                    mesh_cache_hits,
                    mesh_new_generated,
                    mesh_ms,
                    mesh_wait_ms: wait_ms,
                    batch_started_at: batch.batch_started_at,
                })
                .await?;
            let output_send_wait_ms = output_send_start.elapsed().as_millis();
            if output_send_wait_ms > 0 {
                println!(
                    "[batch_stage] batch={} stage=mesh_output send_wait_ms={}",
                    batch.batch_id, output_send_wait_ms
                );
            }
            Ok::<(), anyhow::Error>(())
        }));
    }

    for handle in handles {
        handle.await.map_err(|e| anyhow::anyhow!(e))??;
    }
    drop(output_sender);
    Ok(())
}

async fn run_inst_aabb_writer(
    receiver: Receiver<BatchMeshOutput>,
    base_result_receiver: Receiver<(u64, u128, u128)>,
    completion_sender: Sender<BatchCompletion>,
    inst_aabb_semaphore: Arc<Semaphore>,
    mesh_aabb_map: Arc<DashMap<String, parry3d::bounding_volume::Aabb>>,
    mesh_pts_map: Arc<DashMap<u64, String>>,
) -> anyhow::Result<()> {
    let skip_inst_relate_aabb = std::env::var_os("AIOS_SKIP_INST_RELATE_AABB").is_some();
    let mut handles = Vec::new();
    let mut joiner = BatchStageJoiner::default();
    let mut mesh_closed = false;
    let mut base_closed = false;

    while !mesh_closed || !base_closed {
        tokio::select! {
            mesh_result = receiver.recv_async(), if !mesh_closed => {
                match mesh_result {
                    Ok(batch) => {
                        let batch_id = batch.batch_id;
                        if let Some(batch) = joiner.push_mesh_output(batch) {
                            let inst_aabb_semaphore = inst_aabb_semaphore.clone();
                            let mesh_aabb_map = mesh_aabb_map.clone();
                            let mesh_pts_map = mesh_pts_map.clone();
                            let completion_sender = completion_sender.clone();
                            let skip_inst_relate_aabb = skip_inst_relate_aabb;
                            handles.push(tokio::spawn(async move {
                                let (aabb_permit, inst_aabb_wait_ms) = acquire_with_wait(inst_aabb_semaphore).await?;
                                let inst_aabb_start = Instant::now();
                                persist_inst_geo_mesh_results(&batch.mesh_results, &mesh_aabb_map, &mesh_pts_map).await?;
                                if skip_inst_relate_aabb {
                                    println!(
                                        "[batch_stage] batch={} stage=inst_aabb skipped=inst_relate_aabb env=AIOS_SKIP_INST_RELATE_AABB",
                                        batch.batch_id
                                    );
                                } else {
                                    let (aabb_rows_map, inst_relate_aabb_rows) =
                                        build_inst_relate_aabb_rows(&batch.shape_insts, &batch.mesh_results, &mesh_aabb_map)?;
                                    save_inst_relate_aabb_rows(&aabb_rows_map, &inst_relate_aabb_rows).await?;
                                }
                                let inst_aabb_ms = inst_aabb_start.elapsed().as_millis();
                                drop(aabb_permit);

                                let total_ms = batch.batch_started_at.elapsed().as_millis();
                                println!(
                                    "[batch_perf] batch={} base_wait_ms={} base_write_ms={} mesh_wait_ms={} mesh_ms={} inst_aabb_wait_ms={} inst_aabb_ms={} total_ms={} mesh_cache_hit={} mesh_new_generated={} mesh_tasks={}",
                                    batch.batch_id,
                                    batch.base_wait_ms,
                                    batch.base_write_ms,
                                    batch.mesh_wait_ms,
                                    batch.mesh_ms,
                                    inst_aabb_wait_ms,
                                    inst_aabb_ms,
                                    total_ms,
                                    batch.mesh_cache_hits,
                                    batch.mesh_new_generated,
                                    batch.mesh_task_count
                                );

                                completion_sender
                                    .send_async(BatchCompletion {
                                        batch_id: batch.batch_id,
                                        mesh_task_count: batch.mesh_task_count,
                                        mesh_cache_hits: batch.mesh_cache_hits,
                                        mesh_new_generated: batch.mesh_new_generated,
                                        base_write_ms: batch.base_write_ms,
                                        base_wait_ms: batch.base_wait_ms,
                                        mesh_ms: batch.mesh_ms,
                                        mesh_wait_ms: batch.mesh_wait_ms,
                                        inst_aabb_ms,
                                        inst_aabb_wait_ms,
                                        total_ms,
                                    })
                                    .await?;
                                Ok::<(), anyhow::Error>(())
                            }));
                        } else {
                            println!(
                                "[batch_stage] batch={} stage=join waiting=base_result",
                                batch_id
                            );
                        }
                    }
                    Err(_) => {
                        mesh_closed = true;
                    }
                }
            }
            base_result = base_result_receiver.recv_async(), if !base_closed => {
                match base_result {
                    Ok((batch_id, base_wait_ms, base_write_ms)) => {
                        if let Some(batch) = joiner.push_base_metrics(batch_id, base_wait_ms, base_write_ms) {
                            let inst_aabb_semaphore = inst_aabb_semaphore.clone();
                            let mesh_aabb_map = mesh_aabb_map.clone();
                            let mesh_pts_map = mesh_pts_map.clone();
                            let completion_sender = completion_sender.clone();
                            let skip_inst_relate_aabb = skip_inst_relate_aabb;
                            handles.push(tokio::spawn(async move {
                                let (aabb_permit, inst_aabb_wait_ms) = acquire_with_wait(inst_aabb_semaphore).await?;
                                let inst_aabb_start = Instant::now();
                                persist_inst_geo_mesh_results(&batch.mesh_results, &mesh_aabb_map, &mesh_pts_map).await?;
                                if skip_inst_relate_aabb {
                                    println!(
                                        "[batch_stage] batch={} stage=inst_aabb skipped=inst_relate_aabb env=AIOS_SKIP_INST_RELATE_AABB",
                                        batch.batch_id
                                    );
                                } else {
                                    let (aabb_rows_map, inst_relate_aabb_rows) =
                                        build_inst_relate_aabb_rows(&batch.shape_insts, &batch.mesh_results, &mesh_aabb_map)?;
                                    save_inst_relate_aabb_rows(&aabb_rows_map, &inst_relate_aabb_rows).await?;
                                }
                                let inst_aabb_ms = inst_aabb_start.elapsed().as_millis();
                                drop(aabb_permit);

                                let total_ms = batch.batch_started_at.elapsed().as_millis();
                                println!(
                                    "[batch_perf] batch={} base_wait_ms={} base_write_ms={} mesh_wait_ms={} mesh_ms={} inst_aabb_wait_ms={} inst_aabb_ms={} total_ms={} mesh_cache_hit={} mesh_new_generated={} mesh_tasks={}",
                                    batch.batch_id,
                                    batch.base_wait_ms,
                                    batch.base_write_ms,
                                    batch.mesh_wait_ms,
                                    batch.mesh_ms,
                                    inst_aabb_wait_ms,
                                    inst_aabb_ms,
                                    total_ms,
                                    batch.mesh_cache_hits,
                                    batch.mesh_new_generated,
                                    batch.mesh_task_count
                                );

                                completion_sender
                                    .send_async(BatchCompletion {
                                        batch_id: batch.batch_id,
                                        mesh_task_count: batch.mesh_task_count,
                                        mesh_cache_hits: batch.mesh_cache_hits,
                                        mesh_new_generated: batch.mesh_new_generated,
                                        base_write_ms: batch.base_write_ms,
                                        base_wait_ms: batch.base_wait_ms,
                                        mesh_ms: batch.mesh_ms,
                                        mesh_wait_ms: batch.mesh_wait_ms,
                                        inst_aabb_ms,
                                        inst_aabb_wait_ms,
                                        total_ms,
                                    })
                                    .await?;
                                Ok::<(), anyhow::Error>(())
                            }));
                        } else {
                            println!(
                                "[batch_stage] batch={} stage=join waiting=mesh_output",
                                batch_id
                            );
                        }
                    }
                    Err(_) => {
                        base_closed = true;
                    }
                }
            }
        }
    }

    if !joiner.is_empty() {
        let (pending_mesh, pending_base) = joiner.pending_counts();
        return Err(anyhow::anyhow!(
            "batch stage join 未收敛: pending_mesh_outputs={}, pending_base_metrics={}",
            pending_mesh,
            pending_base
        ));
    }

    for handle in handles {
        handle.await.map_err(|e| anyhow::anyhow!(e))??;
    }
    drop(completion_sender);
    Ok(())
}

async fn persist_inst_geo_mesh_results(
    mesh_results: &HashMap<u64, MeshResult>,
    mesh_aabb_map: &DashMap<String, parry3d::bounding_volume::Aabb>,
    mesh_pts_map: &DashMap<u64, String>,
) -> anyhow::Result<()> {
    if use_file_mesh_state() {
        flush_aabb_cache();
        return Ok(());
    }

    if mesh_results.is_empty() {
        return Ok(());
    }

    // 先落 aabb / pts 实体，再把 inst_geo 指向这些记录，避免引用到尚不存在的实体。
    save_pts_to_surreal(mesh_pts_map).await;
    save_aabb_to_surreal(mesh_aabb_map).await;

    let mut update_sql = String::new();
    for (geo_hash, mesh_result) in mesh_results {
        update_sql.push_str(&mesh_result.to_update_sql(&geo_hash.to_string()));
    }

    if update_sql.is_empty() {
        return Ok(());
    }

    aios_core::model_primary_db()
        .query(&update_sql)
        .await
        .map_err(|e| {
            let preview: String = update_sql.chars().take(500).collect();
            anyhow::anyhow!(
                "回写 inst_geo mesh 结果失败: error={}, sql_preview={}",
                e,
                preview
            )
        })?;

    Ok(())
}

#[derive(Debug, Clone)]
pub struct GenModelResult {
    pub success: bool,
}

/// 主入口函数：生成所有几何体数据
///
/// 这是主要的公共 API，统一收敛到 IndexTree 生成管线：
/// - Full：按 `index_tree_enabled_target_types` 从 TreeIndex 提取入口 roots
/// - Manual / Debug / Incremental：构造 roots 并集后以 seed_roots 直入
///
/// # Arguments
/// * `manual_refnos` - 手动指定的 refno 列表
/// * `db_option` - 数据库配置
/// * `incr_updates` - 增量更新日志
/// * `target_sesno` - 目标 sesno
#[cfg_attr(
    feature = "profile",
    tracing::instrument(skip_all, name = "gen_all_geos_data")
)]
pub async fn gen_all_geos_data(
    manual_refnos: Vec<RefnoEnum>,
    db_option: &DbOptionExt,
    incr_updates: Option<IncrGeoUpdateLog>,
    target_sesno: Option<u32>,
) -> Result<GenModelResult> {
    let time = Instant::now();
    let mut perf = crate::perf_timer::PerfTimer::new("gen_all_geos_data");
    perf.mark("init");

    // cache-first 缺失报告：生成过程中按需补充记录，结束时输出到 output/<project>/cache_miss_report.json
    // cache-first 模式已移除（foyer-cache-cleanup），使用 Direct 模式
    cache_miss_report::init_global_cache_miss_report(db_option, "Direct");
    let mut final_incr_updates = incr_updates;

    // 如果指定了 target_sesno，获取该 sesno 的增量数据
    if let Some(sesno) = target_sesno {
        if !db_option.use_surrealdb {
            return Err(IndexTreeError::Other(anyhow::anyhow!(
                "cache-only 模式下不支持 --target-sesno（需要从 SurrealDB 获取 element_changes）：sesno={}",
                sesno
            )));
        }

        if final_incr_updates.is_none() {
            match get_changes_at_sesno(sesno).await {
                Ok(sesno_changes) => {
                    if sesno_changes.count() > 0 {
                        final_incr_updates = Some(sesno_changes);
                    } else {
                        println!("[gen_model] sesno {} 没有发现变更，跳过增量生成", sesno);
                        return Ok(GenModelResult { success: false });
                    }
                }

                Err(e) => {
                    eprintln!("获取 sesno {} 的变更失败: {}", sesno, e);
                    return Err(IndexTreeError::Other(e));
                }
            }
        }
    }

    let incr_count = final_incr_updates
        .as_ref()
        .map(|log| log.count())
        .unwrap_or(0);
    println!(
        "[gen_model] 启动 gen_all_geos_data: manual_refnos={}, incr_updates={}, target_sesno={:?}",
        manual_refnos.len(),
        incr_count,
        target_sesno,
    );

    // 性能剖析：尽量在最上层启用 tracing，覆盖 precheck -> gen_model -> mesh -> room 计算全链路。

    #[cfg(feature = "profile")]
    let _ = crate::profiling::init_chrome_tracing_for_db_option(db_option, "full_flow_room");
    perf.mark("precheck");

    // ✨ 执行预检查：确保 Tree 文件、pe_transform、db_meta_info 就绪
    if db_option.use_surrealdb {
        use crate::fast_model::gen_model::precheck_coordinator::{PrecheckConfig, run_precheck};
        let precheck_config = PrecheckConfig {
            enabled: true,
            check_tree: true,
            check_pe_transform: true,
            check_db_meta: true,
            tree_output_dir: db_option
                .get_project_output_dir()
                .join("scene_tree")
                .to_string_lossy()
                .to_string(),
        };
        match run_precheck(db_option, Some(precheck_config)).await {
            Ok(stats) => {
                log::info!("[gen_model] 预检查完成: {:?}", stats);
            }

            Err(e) => {
                log::warn!("[gen_model] 预检查部分失败: {}", e);

                // 不阻断流程，继续执行
            }
        }
    } else {
        // cache-only 模式：仅检查 db_meta_info
        let _ = db_meta().ensure_loaded();
    }

    // 调试：打印 IndexTree 模式配置
    println!(
        "[gen_model] IndexTree 默认管线配置: concurrency={}, batch_size={}",
        db_option.get_index_tree_concurrency(),
        db_option.get_index_tree_batch_size()
    );

    // ✅ SurrealDB 写入侧初始化：仅在 use_surrealdb=true 时需要。
    if db_option.use_surrealdb && !db_option.defer_db_write {
        if let Err(e) = aios_core::rs_surreal::inst::init_model_tables().await {
            eprintln!("[gen_model] ❌ 初始化 inst_relate 表结构失败: {}", e);

            // 严重错误，建议直接中断，否则后续写入必挂
            return Err(IndexTreeError::Other(e));
        }
    }

    // =========================
    // LOOP/PRIM 输入缓存初始化（按环境变量启用）

    // =========================
    // geom_input_cache 已移除（foyer-cache-cleanup），跳过缓存初始化
    println!("[gen_model] geom_input_cache: Direct 模式（cache 已移除）");

    // =========================
    // IndexTree 模式：新管线

    // =========================
    // 统一入口：manual/debug/incr/full 全部收敛到 IndexTree 生成管线
    perf.mark("route_decision");
    let debug_roots = db_option.inner.get_all_debug_refnos().await;
    let incr_visible_roots: Vec<RefnoEnum> = final_incr_updates
        .as_ref()
        .map(|log| log.get_all_visible_refnos().into_iter().collect())
        .unwrap_or_default();
    let has_incr_log = final_incr_updates
        .as_ref()
        .map(|log| log.count() > 0)
        .unwrap_or(false);
    let has_incr_visible_roots = !incr_visible_roots.is_empty();
    let scope = decide_generation_scope(
        &manual_refnos,
        &debug_roots,
        has_incr_log,
        &incr_visible_roots,
        final_incr_updates.as_ref(),
    );
    if matches!(scope, GenerationScope::Incremental { .. }) && !has_incr_visible_roots {
        println!(
            "[gen_model] 增量日志存在但未解析到可见 roots，将按 Incremental 空 roots 路径执行（不会回退 Full）"
        );
    }

    let input_source_cnt =
        (!manual_refnos.is_empty() as u8) + (!debug_roots.is_empty() as u8) + (has_incr_log as u8);
    if input_source_cnt >= 2 {
        if let GenerationScope::Manual { roots } = &scope {
            println!(
                "[gen_model] 检测到混合输入(manual/debug/incr)，按 roots 并集执行：{} 个",
                roots.len()
            );
        }
    }

    perf.mark("index_tree_generation");
    let result = process_index_tree_generation(scope, db_option, target_sesno, time).await;
    perf.print_summary();

    // 输出 cache miss 报告（覆盖写）。
    if let Some(report) = cache_miss_report::snapshot_global_report() {
        match report.write_to_default_path(db_option) {
            Ok(path) => {
                println!(
                    "[gen_model] cache_miss_report 已写入: {} (mode={})",
                    path.display(),
                    report.mode
                );
            }
            Err(e) => {
                eprintln!("[gen_model] 写入 cache_miss_report 失败: {}", e);
            }
        }
    } else {
        eprintln!("[gen_model] cache_miss_report 未初始化，跳过写入");
    }

    result
}

async fn filter_bran_hang_refnos(refnos: &[RefnoEnum]) -> Vec<RefnoEnum> {
    let bran_hash = db1_hash("BRAN");
    let hang_hash = db1_hash("HANG");
    let mut out = Vec::new();
    for &r in refnos {
        if !r.is_valid() {
            continue;
        }

        let dbnum = match TreeIndexManager::resolve_dbnum_for_refno(r) {
            Ok(v) => v,
            Err(_) => continue,
        };
        let manager = TreeIndexManager::with_default_dir(vec![dbnum]);
        let Ok(index) = manager.load_index(dbnum) else {
            continue;
        };
        let Some(meta) = index.node_meta(r.refno()) else {
            continue;
        };
        if meta.noun == bran_hash || meta.noun == hang_hash {
            out.push(r);
        }
    }

    out
}

/// 处理 IndexTree 模式的生成流程
async fn process_index_tree_generation(
    scope: GenerationScope,
    db_option: &DbOptionExt,
    _target_sesno: Option<u32>,
    time: Instant,
) -> Result<GenModelResult> {
    let mut perf = crate::perf_timer::PerfTimer::new("index_tree_generation");
    perf.mark("init");
    println!("[gen_model] 进入 IndexTree 生成模式（统一管线）");
    if db_option.manual_db_nums.is_some() || db_option.exclude_db_nums.is_some() {
        println!(
            "[gen_model] 提示: IndexTree 新管线已支持 manual_db_nums / exclude_db_nums 过滤，当前仍按配置执行"
        );
    }

    let seed_roots = match &scope {
        GenerationScope::Full => {
            println!("[gen_model] 当前 scope: Full（按 target_type 入口查询 roots）");
            None
        }
        GenerationScope::Manual { roots } => {
            println!("[gen_model] 当前 scope: Manual roots={}", roots.len());
            Some(roots.clone())
        }
        GenerationScope::Debug { roots } => {
            println!("[gen_model] 当前 scope: Debug roots={}", roots.len());
            Some(roots.clone())
        }
        GenerationScope::Incremental { log } => {
            let roots: Vec<RefnoEnum> = log.get_all_visible_refnos().into_iter().collect();
            println!("[gen_model] 当前 scope: Incremental roots={}", roots.len());
            Some(roots)
        }
    };
    let full_start = Instant::now();
    perf.mark("categorize_and_inst_relate");

    // 1️⃣ 生成/更新 inst_relate，并获取分类后的根 refno
    let config = IndexTreeConfig::from_db_option_ext(db_option)
        .map_err(|e| anyhow::anyhow!("配置错误: {}", e))?;
    let (sender, receiver) = flume::bounded::<aios_core::geometry::ShapeInstancesData>(
        db_option.get_batch_channel_capacity(),
    );
    let replace_exist = db_option.inner.is_replace_mesh();
    let use_surrealdb = db_option.use_surrealdb;
    let defer_db_write = false;

    // Mesh 生成：生产端只产出 inst batch，mesh/持久化在下游并行阶段完成
    let gen_mesh = db_option.inner.gen_mesh;

    // 初始化 Parquet 写入器（默认关闭，通过环境变量显式开启）。
    //
    // 开关：AIOS_ENABLE_PARQUET_STREAM_WRITER=1|true|yes|on
    // 说明：此前该路径固定为 None，容易造成“看似支持但实际未启用”的误解；
    // 这里改为显式开关，默认行为保持不变（关闭）。
    let enable_parquet_stream_writer = std::env::var("AIOS_ENABLE_PARQUET_STREAM_WRITER")
        .ok()
        .map(|v| {
            matches!(
                v.trim().to_ascii_lowercase().as_str(),
                "1" | "true" | "yes" | "on"
            )
        })
        .unwrap_or(false);

    // ParquetStreamWriter 需要 parquet-export feature

    #[cfg(feature = "parquet-export")]
    let parquet_writer = if enable_parquet_stream_writer {
        let output_dir = db_option
            .inner
            .meshes_path
            .as_deref()
            .unwrap_or("assets/meshes");
        let parquet_dir = std::path::Path::new(output_dir)
            .parent()
            .unwrap_or(std::path::Path::new("output"));
        match ParquetStreamWriter::new(parquet_dir) {
            Ok(writer) => {
                println!(
                    "[Parquet] 已启用流式写入（AIOS_ENABLE_PARQUET_STREAM_WRITER=1），输出目录: {}",
                    parquet_dir.display()
                );
                Some(std::sync::Arc::new(writer))
            }
            Err(e) => {
                eprintln!("[Parquet] 初始化写入器失败: {}, 回退为禁用", e);
                None
            }
        }
    } else {
        println!("[Parquet] 流式写入已禁用（可设置 AIOS_ENABLE_PARQUET_STREAM_WRITER=1 显式开启）");
        None
    };

    #[cfg(not(feature = "parquet-export"))]
    let parquet_writer: Option<std::sync::Arc<()>> = None;

    //
    #[allow(unused_variables)]
    let parquet_writer_clone = parquet_writer.clone();

    // model cache-only 已移除（foyer-cache-cleanup）
    let model_cache_ctx: Option<()> = None;
    #[allow(unused_variables)]
    let cache_manager_for_insert: Option<()> = None;

    let touched_dbnums: Arc<std::sync::Mutex<BTreeSet<u32>>> =
        Arc::new(std::sync::Mutex::new(BTreeSet::new()));

    // IndexTree 下用于 inst_relate_aabb 写入的 refno 集合：只收集“本次生成触达”的实例，
    // 避免通过 pe_transform 全库扫描导致卡死/耗时失真。
    let touched_refnos: Arc<std::sync::Mutex<std::collections::HashSet<RefnoEnum>>> =
        Arc::new(std::sync::Mutex::new(std::collections::HashSet::new()));
    let touched_refnos_for_insert = touched_refnos.clone();

    // 当 manual_db_nums 只有一个值时，直接使用该 dbnum，无需从 refno 反推
    let known_dbnum: Option<u32> = db_option
        .inner
        .manual_db_nums
        .as_ref()
        .filter(|nums| nums.len() == 1)
        .and_then(|nums| nums.first().copied());
    let mesh_aabb_map: Arc<DashMap<String, parry3d::bounding_volume::Aabb>> =
        Arc::new(DashMap::new());
    let mesh_pts_map: Arc<DashMap<u64, String>> = Arc::new(DashMap::new());
    let missing_neg_carriers_for_reconcile: Arc<std::sync::Mutex<HashSet<RefnoEnum>>> =
        Arc::new(std::sync::Mutex::new(HashSet::new()));
    let base_write_semaphore = Arc::new(Semaphore::new(db_option.get_base_write_concurrency()));
    let mesh_compute_semaphore = Arc::new(Semaphore::new(db_option.get_mesh_compute_concurrency()));
    let inst_aabb_semaphore = Arc::new(Semaphore::new(db_option.get_inst_aabb_write_concurrency()));
    let (base_writer_sender, base_writer_receiver) =
        flume::bounded::<PipelineBatch>(db_option.get_batch_channel_capacity());
    let (base_result_sender, base_result_receiver) =
        flume::bounded::<(u64, u128, u128)>(db_option.get_batch_channel_capacity());
    let (mesh_stage_sender, mesh_stage_receiver) =
        flume::bounded::<PipelineBatch>(db_option.get_batch_channel_capacity());
    let (mesh_output_sender, mesh_output_receiver) =
        flume::bounded::<BatchMeshOutput>(db_option.get_batch_channel_capacity());
    // completion 仅用于主线程汇总 batch 统计，不参与生产侧背压。
    // 若这里使用有界通道，run_inst_aabb_writer 内部任务会在 send_async(completion)
    // 上卡住，而主线程又要等 inst_aabb_handle 退出后才开始 recv，形成尾部自锁。
    let (completion_sender, completion_receiver) = flume::unbounded::<BatchCompletion>();

    let sink_handle = tokio::spawn(run_batch_sink(
        receiver,
        base_writer_sender,
        mesh_stage_sender,
        touched_refnos_for_insert,
    ));
    let base_writer_handle = tokio::spawn(run_base_writer(
        base_writer_receiver,
        base_result_sender,
        replace_exist,
        base_write_semaphore.clone(),
        mesh_aabb_map.clone(),
        missing_neg_carriers_for_reconcile.clone(),
    ));
    let mesh_stage_handle = tokio::spawn(run_mesh_stage(
        mesh_stage_receiver,
        mesh_output_sender,
        mesh_compute_semaphore,
        db_option.clone(),
        replace_exist,
        gen_mesh,
        mesh_aabb_map.clone(),
        mesh_pts_map.clone(),
    ));
    let inst_aabb_handle = tokio::spawn(run_inst_aabb_writer(
        mesh_output_receiver,
        base_result_receiver,
        completion_sender,
        inst_aabb_semaphore,
        mesh_aabb_map.clone(),
        mesh_pts_map.clone(),
    ));
    println!("⏳ [1/5] 几何体生成 (BRAN/HANG + LOOP/CATE/PRIM)...");
    let categorized = gen_index_tree_geos_optimized(
        Arc::new(db_option.clone()),
        &config,
        sender.clone(),
        seed_roots,
    )
    .await
    .map_err(|e| anyhow::anyhow!("IndexTree 生成失败: {}", e))?;
    println!(
        "✅ [1/5] 几何体生成完成, 用时 {}ms",
        full_start.elapsed().as_millis()
    );

    // 🔥 显式 drop sender，让 receiver 的循环能够正常结束
    // 否则 insert_handle.await 会永久阻塞
    println!("⏳ [2/5] 实例数据入库...");
    drop(sender);
    let insert_report = sink_handle
        .await
        .map_err(|e| anyhow::anyhow!("batch sink 任务异常退出: {}", e))?
        .map_err(IndexTreeError::Other)?;
    base_writer_handle
        .await
        .map_err(|e| anyhow::anyhow!("base writer 任务异常退出: {}", e))?
        .map_err(IndexTreeError::Other)?;
    mesh_stage_handle
        .await
        .map_err(|e| anyhow::anyhow!("mesh stage 任务异常退出: {}", e))?
        .map_err(IndexTreeError::Other)?;
    let barrier_wait_start = Instant::now();
    inst_aabb_handle
        .await
        .map_err(|e| anyhow::anyhow!("inst aabb writer 任务异常退出: {}", e))?
        .map_err(IndexTreeError::Other)?;
    let mut completed_batches = 0usize;
    let mut total_mesh_cache_hits = 0usize;
    let mut total_mesh_new_generated = 0usize;
    while let Ok(completion) = completion_receiver.recv_async().await {
        completed_batches += 1;
        total_mesh_cache_hits += completion.mesh_cache_hits;
        total_mesh_new_generated += completion.mesh_new_generated;
    }
    let missing_neg_carriers = {
        let guard = missing_neg_carriers_for_reconcile.lock().unwrap();
        let mut carriers = guard.iter().copied().collect::<Vec<_>>();
        carriers.sort_unstable();
        carriers
    };
    let barrier_wait_ms = barrier_wait_start.elapsed().as_millis();
    println!(
        "[gen_model] batch barrier complete: batches={} barrier_wait_ms={} mesh_cache_hit={} mesh_new_generated={} missing_neg_candidates={}",
        completed_batches,
        barrier_wait_ms,
        total_mesh_cache_hits,
        total_mesh_new_generated,
        missing_neg_carriers.len()
    );
    let mut bool_tasks = insert_report.bool_tasks;
    println!(
        "✅ [2/5] 实例数据入库完成, 用时 {}ms",
        full_start.elapsed().as_millis()
    );
    perf.mark("mesh_generation");

    // 2️⃣ 可选执行 mesh 生成（已由并行 mesh stage 完成，此处仅汇总结果）
    if db_option.inner.gen_mesh {
        let mesh_start = Instant::now();

        // 收集所有 refnos（后续 web bundle / aabb 等步骤仍需使用）
        let cate = categorized.get_by_category(NounCategory::Cate);
        let loops = categorized.get_by_category(NounCategory::LoopOwner);
        let prims = categorized.get_by_category(NounCategory::Prim);
        let mut all_refnos = Vec::with_capacity(cate.len() + loops.len() + prims.len());
        all_refnos.extend(cate);
        all_refnos.extend(loops);
        all_refnos.extend(prims);
        let mut ran_primary = false;

        ran_primary = gen_mesh;
        if gen_mesh {
            println!(
                "[gen_model] IndexTree 模式 mesh 并行阶段完成，用时 {} ms",
                mesh_start.elapsed().as_millis()
            );
        }

        perf.mark("aabb_write");
        println!("⏳ [3/5] AABB 写入...");

        // 3️⃣ batch barrier 之后，inst_relate_aabb 已按 batch 写入完成
        if use_surrealdb {
            let skip_aabb_write = std::env::var_os("AIOS_SKIP_INST_RELATE_AABB").is_some();
            if skip_aabb_write {
                println!(
                    "[gen_model] IndexTree 模式已跳过 batch inst_relate_aabb 写入（AIOS_SKIP_INST_RELATE_AABB=1）"
                );
            } else {
                println!("[gen_model] IndexTree 模式 batch inst_relate_aabb 写入已完成");
            }
        }

        perf.mark("boolean_operation");
        println!("⏳ [4/5] 布尔运算...");

        // 3.5️⃣ barrier 后补建跨阶段缺失的 neg_relate（LOOP 阶段发现负实体但 PRIM 阶段才创建 geo_relate）
        if use_surrealdb {
            if let Err(e) = reconcile_missing_neg_relate(&all_refnos, &missing_neg_carriers).await {
                eprintln!("[gen_model] reconcile_missing_neg_relate 失败: {}", e);
            }
        }

        // 4️⃣ 可选执行布尔运算
        if db_option.inner.apply_boolean_operation {
            let bool_start = Instant::now();
            println!("[gen_model] IndexTree 模式开始布尔运算（boolean worker）");
            println!(
                "[gen_model] boolean_pipeline_mode={:?}, defer_db_write={}, use_surrealdb={}, enable_db_backfill={}",
                db_option.boolean_pipeline_mode,
                defer_db_write,
                use_surrealdb,
                db_option.enable_db_backfill
            );
            println!(
                "[gen_model] 布尔任务统计: total={} (insert_batch_cnt={})",
                bool_tasks.len(),
                insert_report.batch_cnt
            );

            // model_cache boolean worker 已移除（foyer-cache-cleanup）
            match db_option.boolean_pipeline_mode {
                BooleanPipelineMode::DbLegacy => {
                    if use_surrealdb && !defer_db_write {
                        if let Err(e) =
                            run_boolean_worker(Arc::new(db_option.inner.clone()), 100).await
                        {
                            eprintln!("[gen_model] IndexTree 布尔运算失败（db_legacy）: {}", e);
                        }
                    } else {
                        println!(
                            "[gen_model] boolean_pipeline_mode=db_legacy，当前模式不满足执行条件（use_surrealdb={} defer_db_write={}）",
                            use_surrealdb, defer_db_write
                        );
                    }
                }
                BooleanPipelineMode::MemoryTasks => {
                    // 模式组合合法性守卫：MemoryTasks 至少需要一种写入通道
                    if !use_surrealdb {
                        eprintln!(
                            "[gen_model] boolean_pipeline_mode=memory_tasks 非法：use_surrealdb=false，无写入通道，跳过布尔"
                        );
                    } else if bool_tasks.is_empty() {
                        println!(
                            "[gen_model] boolean_pipeline_mode=memory_tasks，但没有可执行布尔任务"
                        );
                    } else {
                        // T7: DB backfill — 补齐内存中缺失的 cata 任务
                        if db_option.enable_db_backfill {
                            match super::boolean_backfill::backfill_cata_tasks_from_db(
                                &mut bool_tasks,
                                use_surrealdb,
                            )
                            .await
                            {
                                Ok(count) if count > 0 => {
                                    println!(
                                        "[gen_model] DB backfill 补齐了 {} 个 cata 布尔任务，当前总数 {}",
                                        count,
                                        bool_tasks.len()
                                    );
                                }
                                Err(e) => {
                                    eprintln!(
                                        "[gen_model] DB backfill 失败（非致命，继续执行）: {}",
                                        e
                                    );
                                }
                                _ => {}
                            }
                        }

                        match run_bool_worker_from_tasks(
                            std::mem::take(&mut bool_tasks),
                            Arc::new(db_option.inner.clone()),
                            None,
                        )
                        .await
                        {
                            Ok(report) => {
                                println!(
                                    "[gen_model] memory bool worker 完成: total={} cata={} inst={} success={} failed={} skipped={} defer={}",
                                    report.total,
                                    report.cata_cnt,
                                    report.inst_cnt,
                                    report.success,
                                    report.failed,
                                    report.skipped,
                                    report.deferred_mode
                                );
                            }
                            Err(e) => {
                                eprintln!(
                                    "[gen_model] IndexTree 布尔运算失败（memory_tasks）: {}",
                                    e
                                );
                            }
                        }
                    }
                }
            }

            println!(
                "[gen_model] IndexTree 模式布尔运算完成，用时 {} ms",
                bool_start.elapsed().as_millis()
            );
        }
        perf.mark("web_bundle_export");
        println!("⏳ [5/5] 导出...");

        // 5️⃣ 生成 Web Bundle (GLB + JSON 数据包)
        if db_option.mesh_formats.contains(&MeshFormat::Glb) {
            let web_bundle_start = Instant::now();
            println!("[gen_model] 开始生成 Web Bundle (GLB + JSON 数据包)...");
            let mesh_dir = Path::new(
                db_option
                    .inner
                    .meshes_path
                    .as_deref()
                    .unwrap_or("assets/meshes"),
            );

            // 输出到与 meshes 同级的 web_bundle 目录
            let output_dir = mesh_dir.parent().unwrap_or(mesh_dir).join("web_bundle");
            if let Err(e) = export_prepack_lod_for_refnos(
                &all_refnos,
                &mesh_dir,
                &output_dir,
                Arc::new(db_option.inner.clone()),
                true,  // include_descendants
                None,  // filter_nouns
                true,  // verbose
                None,  // name_config
                false, // export_all_lods: 改为 false，遵循 DbOption 中的默认设置
                LengthUnit::Millimeter,
                LengthUnit::Millimeter,
            )
            .await
            {
                eprintln!("[gen_model] 生成 Web Bundle 失败: {}", e);
            } else {
                println!(
                    "[gen_model] Web Bundle 生成完成，输出目录: {}, 用时 {} ms",
                    output_dir.display(),
                    web_bundle_start.elapsed().as_millis()
                );
            }
        }
    }

    perf.mark("sqlite_spatial_index");
    println!(
        "[gen_model] IndexTree 模式全部完成，总用时 {} ms",
        full_start.elapsed().as_millis()
    );
    println!(
        "[gen_model] gen_all_geos_data 总耗时: {} ms",
        time.elapsed().as_millis()
    );
    let touched_dbnums_vec: Vec<u32> = touched_dbnums
        .lock()
        .map(|s| s.iter().copied().collect())
        .unwrap_or_default();
    perf.mark("instances_export");

    // ✅ 模型生成完毕后导出 instances.json（按 dbno）
    if db_option.export_instances {
        let (dbno_source, mut dbnos): (&str, Vec<u32>) =
            if let Some(nums) = db_option.inner.manual_db_nums.clone() {
                ("manual_db_nums", nums)
            } else if !touched_dbnums_vec.is_empty() {
                // 优先导出本次生成实际触达的 dbnum，避免扫描全 MDB 触发无关库的 tree 缺失报错。
                ("touched_dbnums", touched_dbnums_vec.clone())
            } else {
                (
                    "query_mdb_db_nums",
                    aios_core::query_mdb_db_nums(None, aios_core::DBType::DESI).await?,
                )
            };
        if let Some(exclude_nums) = &db_option.inner.exclude_db_nums {
            use std::collections::HashSet;
            let exclude: HashSet<u32> = exclude_nums.iter().copied().collect();
            dbnos.retain(|dbnum| !exclude.contains(dbnum));
        }

        dbnos.sort_unstable();
        dbnos.dedup();
        if dbnos.is_empty() {
            println!(
                "[instances] 跳过导出：未解析到可用 dbnum（source={})",
                dbno_source
            );
        } else {
            println!(
                "[instances] 开始导出 instances.json: source={}, dbnums={:?}",
                dbno_source, dbnos
            );
        }

        let mesh_dir = Path::new(
            db_option
                .inner
                .meshes_path
                .as_deref()
                .unwrap_or("assets/meshes"),
        );
        if !dbnos.is_empty() {
            if let Err(e) = export_instances_json_for_dbnos(
                &dbnos,
                mesh_dir,
                &db_option.get_project_output_dir(),
                Arc::new(db_option.inner.clone()),
                true,
            )
            .await
            {
                eprintln!("[instances] IndexTree 导出失败: {}", e);
            }
        }
    }

    // model_cache close 已移除（foyer-cache-cleanup）
    perf.end_current();

    // 输出性能摘要到控制台
    perf.print_summary();

    // 保存性能报告为 JSON 和 CSV
    let project_name = if !db_option.inner.project_name.is_empty() {
        db_option.inner.project_name.clone()
    } else {
        "default".to_string()
    };
    let timestamp = chrono::Local::now().format("%Y%m%d_%H%M%S");
    let profile_dir = std::path::PathBuf::from("output")
        .join(&project_name)
        .join("profile");

    // 收集配置元数据
    let dbnum_tag = db_option
        .inner
        .manual_db_nums
        .as_ref()
        .and_then(|nums| nums.first().copied())
        .map(|n| n.to_string())
        .unwrap_or_else(|| "all".to_string());
    let enabled_nouns = db_option.index_tree_enabled_target_types.clone();
    let metadata = serde_json::json!({
        "mode": "index_tree",
        "project_name": project_name,
        "dbnum": dbnum_tag,
        "enabled_nouns": enabled_nouns,
        "use_surrealdb": db_option.use_surrealdb,
        "model_cache_write": true,
        "apply_boolean": db_option.inner.apply_boolean_operation,
        "gen_mesh": db_option.inner.gen_mesh,
        "concurrency": db_option.get_index_tree_concurrency(),
        "batch_size": db_option.get_index_tree_batch_size(),
    });
    let json_path = profile_dir.join(format!(
        "perf_gen_model_index_tree_dbnum_{}_{}.json",
        dbnum_tag, timestamp
    ));
    let csv_path = profile_dir.join(format!(
        "perf_gen_model_index_tree_dbnum_{}_{}.csv",
        dbnum_tag, timestamp
    ));
    if let Err(e) = perf.save_json(&json_path, metadata.clone()) {
        eprintln!("[perf] 保存 JSON 报告失败: {}", e);
    }

    if let Err(e) = perf.save_csv(&csv_path, metadata) {
        eprintln!("[perf] 保存 CSV 报告失败: {}", e);
    }

    Ok(GenModelResult { success: true })
}

// ============================================================================
// SQLite 空间索引：从 model cache 生成/增量更新 output/spatial_index.sqlite
//
// 目标：模型生成（写 cache）后，将 AABB 批量落库到 SQLite RTree，供房间计算等流程做粗筛。

// ============================================================================

#[cfg(feature = "sqlite-index")]
pub async fn update_sqlite_spatial_index_from_cache(
    db_option: &DbOptionExt,
    dbnums: &[u32],
) -> Result<()> {
    use crate::spatial_index::SqliteSpatialIndex;
    use crate::sqlite_index::{ImportConfig, SqliteAabbIndex};
    use std::fs;
    if dbnums.is_empty() {
        return Ok(());
    }

    if !db_option.inner.enable_sqlite_rtree {
        // 常见误区：已切换到 cache 生成，但忘了开 enable_sqlite_rtree，导致 spatial_index.sqlite 不会更新，
        // 房间计算（SQLite RTree 粗筛）会退化/失效。
        let idx_path = SqliteSpatialIndex::default_path();
        if !idx_path.exists() {
            eprintln!(
                "[gen_model] 警告：enable_sqlite_rtree=false，且未发现 {:?}；模型 AABB 不会落库到 SQLite。\
                 若需房间计算粗筛/诊断，请在 DbOption.toml 开启 enable_sqlite_rtree=true 或使用 CLI 导入 instances.json。",
                idx_path
            );
        }

        return Ok(());
    }

    // 打开/初始化索引（幂等）
    let idx_path = SqliteSpatialIndex::default_path();
    if let Some(parent) = idx_path.parent() {
        fs::create_dir_all(parent).map_err(|e| anyhow::anyhow!(e))?;
    }

    let idx = SqliteAabbIndex::open(&idx_path).map_err(|e| anyhow::anyhow!(e))?;
    idx.init_schema().map_err(|e| anyhow::anyhow!(e))?;

    // 为避免 aabb.json/trans.json（固定文件名）互相覆盖，每个 dbnum 独立输出目录。
    let base_out = db_option
        .get_project_output_dir()
        .join("instances_cache_for_index");
    fs::create_dir_all(&base_out).map_err(|e| anyhow::anyhow!(e))?;
    let project_output_dir = db_option.get_project_output_dir();
    let project_instances_dir = project_output_dir.join("instances");
    let nested_project_instances_dir = project_output_dir
        .join(&db_option.inner.project_name)
        .join("instances");

    // mesh_lod_tag 仅用于导出侧选择 mesh（用于补齐/计算 AABB）
    let cache_dir = db_option.get_model_cache_dir();
    let mesh_dir = db_option.inner.get_meshes_path();
    let mesh_lod_tag = format!("{:?}", db_option.inner.mesh_precision.default_lod);

    // 去重并保证顺序稳定（便于日志与排查）
    let mut uniq: BTreeSet<u32> = BTreeSet::new();
    uniq.extend(dbnums.iter().copied());
    for dbnum in uniq {
        // 优先复用本轮生成已经落盘的 instances 输出，避免继续依赖已移除的旧 cache contract。
        let direct_instances_path = project_instances_dir.join(format!("instances_{}.json", dbnum));
        let nested_instances_path =
            nested_project_instances_dir.join(format!("instances_{}.json", dbnum));
        let instances_path = if direct_instances_path.exists() {
            direct_instances_path
        } else if nested_instances_path.exists() {
            nested_instances_path
        } else {
            let out_dir = base_out.join(format!("{}", dbnum));
            fs::create_dir_all(&out_dir).map_err(|e| anyhow::anyhow!(e))?;

            let _ = crate::fast_model::export_model::export_prepack_lod::export_dbnum_instances_json_from_cache(
                dbnum,
                &out_dir,
                &cache_dir,
                Some(&mesh_dir),
                Some(mesh_lod_tag.as_str()),
                false,
                None,
                false,
            )
            .await?;

            out_dir.join(format!("instances_{}.json", dbnum))
        };

        if instances_path.exists() {
            let _ = idx.import_from_instances_json(&instances_path, &ImportConfig::default())?;
        }
    }

    Ok(())
}

#[cfg(not(feature = "sqlite-index"))]
pub async fn update_sqlite_spatial_index_from_cache(
    _db_option: &DbOptionExt,
    _dbnums: &[u32],
) -> Result<()> {
    Ok(())
}

fn initialize_spatial_index() {
    // No-op placeholder
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    #[test]
    fn test_route_incremental_when_visible_roots_empty() {
        let manual_refnos: Vec<RefnoEnum> = Vec::new();
        let debug_roots: Vec<RefnoEnum> = Vec::new();
        let mut incr_log = IncrGeoUpdateLog::default();
        incr_log.prim_refnos.insert("17496_171666".into());
        let scope =
            decide_generation_scope(&manual_refnos, &debug_roots, true, &[], Some(&incr_log));
        assert!(matches!(scope, GenerationScope::Incremental { .. }));
    }

    #[tokio::test]
    async fn test_db_write_failures_are_not_silenced() {
        let handles = vec![tokio::spawn(async { true }), tokio::spawn(async { false })];
        let failures = collect_db_write_failures(handles).await;
        let result = ensure_no_db_write_failures(failures);
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("SurrealDB 批量写入存在失败任务")
        );
    }

    fn sample_mesh_output(batch_id: u64) -> BatchMeshOutput {
        BatchMeshOutput {
            batch_id,
            shape_insts: Arc::new(aios_core::geometry::ShapeInstancesData::default()),
            mesh_results: HashMap::new(),
            mesh_task_count: 0,
            mesh_cache_hits: 0,
            mesh_new_generated: 0,
            mesh_ms: 17,
            mesh_wait_ms: 5,
            batch_started_at: Instant::now(),
        }
    }

    #[test]
    fn test_batch_stage_joiner_waits_for_base_after_mesh() {
        let mut joiner = BatchStageJoiner::default();
        assert!(joiner.push_mesh_output(sample_mesh_output(7)).is_none());

        let ready = joiner
            .push_base_metrics(7, 11, 13)
            .expect("base 到达后应完成汇合");

        assert_eq!(ready.batch_id, 7);
        assert_eq!(ready.base_wait_ms, 11);
        assert_eq!(ready.base_write_ms, 13);
        assert_eq!(ready.mesh_ms, 17);
        assert_eq!(ready.mesh_wait_ms, 5);
    }

    #[test]
    fn test_batch_stage_joiner_waits_for_mesh_after_base() {
        let mut joiner = BatchStageJoiner::default();
        assert!(joiner.push_base_metrics(9, 3, 4).is_none());

        let ready = joiner
            .push_mesh_output(sample_mesh_output(9))
            .expect("mesh 到达后应完成汇合");

        assert_eq!(ready.batch_id, 9);
        assert_eq!(ready.base_wait_ms, 3);
        assert_eq!(ready.base_write_ms, 4);
        assert_eq!(ready.mesh_ms, 17);
    }
}
