//! Internal model write pipeline.
//!
//! Owns channel topology, workers, stage joins, the batch barrier, final mesh
//! sweep and relation reconciliation. The generation orchestrator only starts
//! the pipeline, produces geometry, and consumes the unified report.

use aios_core::RefnoEnum;
use aios_core::geometry::ShapeInstancesData;
use dashmap::DashMap;
use flume::{Receiver, Sender};
use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Instant;
use tokio::sync::{OwnedSemaphorePermit, Semaphore};

use super::boolean_task::BooleanTask;
use super::model_writer::{GenerationArtifacts, ModelWriterBackend, ModelWriterFinishReport};
use crate::fast_model::mesh_generate::{MeshResult, query_existing_meshed_inst_geo_ids};
use crate::options::DbOptionExt;

#[derive(Debug, Default)]
struct InsertHandleReport {
    batch_cnt: u64,
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
    artifacts: Arc<GenerationArtifacts>,
) -> anyhow::Result<InsertHandleReport> {
    let mut batch_cnt: u64 = 0;

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

        artifacts.record_base_batch(batch.batch_id, Arc::clone(&batch.shape_insts))?;
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

    Ok(InsertHandleReport { batch_cnt })
}

async fn run_drain_only_sink(
    receiver: Receiver<ShapeInstancesData>,
    model_writer: Arc<dyn ModelWriterBackend>,
    artifacts: Arc<GenerationArtifacts>,
) -> anyhow::Result<ModelWriterFinishReport> {
    model_writer.cleanup().await?;
    model_writer.init().await?;
    let mut batch_id = 0u64;
    while let Ok(batch) = receiver.recv_async().await {
        batch_id += 1;
        let batch = Arc::new(batch);
        artifacts.record_base_batch(batch_id, Arc::clone(&batch))?;
        model_writer.write_base_batch(&batch).await?;
    }
    model_writer.finalize().await
}

async fn run_base_writer(
    receiver: Receiver<PipelineBatch>,
    result_sender: Sender<(u64, u128, u128)>,
    base_write_semaphore: Arc<Semaphore>,
    worker_count: usize,
    model_writer: Arc<dyn ModelWriterBackend>,
    artifacts: Arc<GenerationArtifacts>,
) -> anyhow::Result<ModelWriterFinishReport> {
    let mut handles = Vec::new();
    let worker_count = worker_count.max(1);
    println!("[batch_stage] stage=base worker_pool={}", worker_count);
    let cleanup_report = model_writer.cleanup().await?;
    println!(
        "[model-writer:{}] stage={} status={:?} skipped_reason={:?}",
        model_writer.name(),
        cleanup_report.stage,
        cleanup_report.status,
        cleanup_report.skipped_reason
    );
    let init_report = model_writer.init().await?;
    println!(
        "[model-writer:{}] stage={} status={:?}",
        model_writer.name(),
        init_report.stage,
        init_report.status
    );
    // fail-fast：单 worker 失败后置 abort 标志，其余 worker 停止消费新批次并退出，
    // receiver 随之关闭 → sink 发送失败 → 生成侧尽早停止。此前错误只在全部批次
    // 消费完、join 时才暴露，大批量下会白跑很久。
    let abort = Arc::new(AtomicBool::new(false));
    for worker_id in 0..worker_count {
        let receiver = receiver.clone();
        let semaphore = base_write_semaphore.clone();
        let result_sender = result_sender.clone();
        let model_writer = model_writer.clone();
        let artifacts = Arc::clone(&artifacts);
        let abort = Arc::clone(&abort);
        handles.push(tokio::spawn(async move {
            while let Ok(batch) = receiver.recv_async().await {
                if abort.load(Ordering::Relaxed) {
                    break;
                }
                let step: anyhow::Result<()> = async {
                    let (permit, wait_ms) = acquire_with_wait(semaphore.clone()).await?;
                    let base_start = Instant::now();
                    let write_report = model_writer.write_base_batch(&batch.shape_insts).await?;
                    // 专门的 pe_transform 落库阶段：用生成阶段已算出的 world_transform
                    // 就地写库，按需生成、无整库 BFS。
                    let pe_transform_report =
                        model_writer.persist_pe_transform(&batch.shape_insts).await?;
                    artifacts.record_missing_neg_carriers(
                        write_report.missing_neg_carriers.iter().copied(),
                    )?;
                    let base_ms = base_start.elapsed().as_millis();
                    drop(permit);
                    println!(
                        "[batch_stage] batch={} stage=base worker={} wait_ms={} base_write_ms={} pe_transform={:?}/{} missing_neg_candidates={}",
                        batch.batch_id,
                        worker_id,
                        wait_ms,
                        base_ms,
                        pe_transform_report.status,
                        pe_transform_report.item_count,
                        write_report.missing_neg_carriers.len()
                    );
                    result_sender
                        .send_async((batch.batch_id, wait_ms, base_ms))
                        .await?;
                    Ok(())
                }
                .await;
                if let Err(error) = step {
                    abort.store(true, Ordering::Relaxed);
                    return Err(error);
                }
            }
            Ok::<(), anyhow::Error>(())
        }));
    }

    for handle in handles {
        handle.await.map_err(|e| anyhow::anyhow!(e))??;
    }
    let finish_report = model_writer.finalize().await?;
    drop(result_sender);
    Ok(finish_report)
}

async fn run_mesh_stage(
    receiver: Receiver<PipelineBatch>,
    output_sender: Sender<BatchMeshOutput>,
    mesh_compute_semaphore: Arc<Semaphore>,
    worker_count: usize,
    db_option: DbOptionExt,
    gen_mesh: bool,
    mesh_aabb_map: Arc<DashMap<String, parry3d::bounding_volume::Aabb>>,
    mesh_pts_map: Arc<DashMap<u64, String>>,
    artifacts: Arc<GenerationArtifacts>,
) -> anyhow::Result<()> {
    let deduper = Arc::new(crate::fast_model::mesh_generate::RecentGeoDeduper::new(
        200_000,
    ));
    if gen_mesh {
        crate::fast_model::preload_mesh_cache();
        let ids = query_existing_meshed_inst_geo_ids();
        let count = ids.len();
        deduper.preload(ids);
        println!(
            "[mesh_pipeline] 预加载 {} 个已 meshed inst_geo ID 到去重器 (size={})",
            count,
            deduper.len()
        );
    } else if !gen_mesh {
        println!("[mesh_pipeline] gen_mesh 未开启，跳过 mesh 阶段");
    }

    let mut handles = Vec::new();
    let worker_count = worker_count.max(1);
    println!("[batch_stage] stage=mesh worker_pool={}", worker_count);
    // 与 base writer 相同的 fail-fast 策略。
    let abort = Arc::new(AtomicBool::new(false));
    for worker_id in 0..worker_count {
        let receiver = receiver.clone();
        let semaphore = mesh_compute_semaphore.clone();
        let deduper = deduper.clone();
        let mesh_aabb_map = mesh_aabb_map.clone();
        let mesh_pts_map = mesh_pts_map.clone();
        let output_sender = output_sender.clone();
        let artifacts = Arc::clone(&artifacts);
        let db_option_inner = db_option.inner.clone();
        let abort = Arc::clone(&abort);
        handles.push(tokio::spawn(async move {
            while let Ok(batch) = receiver.recv_async().await {
                if abort.load(Ordering::Relaxed) {
                    break;
                }
                let step: anyhow::Result<()> = async {
                    let (permit, wait_ms) = acquire_with_wait(semaphore.clone()).await?;
                    let mesh_start = Instant::now();
                    let tasks =
                        crate::fast_model::mesh_generate::extract_mesh_tasks(&batch.shape_insts);
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
                    artifacts.record_mesh_results(batch.batch_id, &mesh_results)?;

                    let mesh_ms = mesh_start.elapsed().as_millis();
                    drop(permit);
                    println!(
                        "[batch_stage] batch={} stage=mesh worker={} wait_ms={} mesh_ms={} mesh_tasks={} mesh_cache_hit={} mesh_new_generated={}",
                        batch.batch_id, worker_id, wait_ms, mesh_ms, mesh_task_count, mesh_cache_hits, mesh_new_generated
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
                            "[batch_stage] batch={} stage=mesh_output worker={} send_wait_ms={}",
                            batch.batch_id, worker_id, output_send_wait_ms
                        );
                    }
                    Ok(())
                }
                .await;
                if let Err(error) = step {
                    abort.store(true, Ordering::Relaxed);
                    return Err(error);
                }
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

async fn process_inst_aabb_batch(
    batch: JoinedBatchOutput,
    inst_aabb_semaphore: Arc<Semaphore>,
    mesh_aabb_map: Arc<DashMap<String, parry3d::bounding_volume::Aabb>>,
    mesh_pts_map: Arc<DashMap<u64, String>>,
    completion_sender: Sender<BatchCompletion>,
    model_writer: Arc<dyn ModelWriterBackend>,
    skip_inst_relate_aabb: bool,
    worker_id: usize,
) -> anyhow::Result<()> {
    let (aabb_permit, inst_aabb_wait_ms) = acquire_with_wait(inst_aabb_semaphore).await?;
    let inst_aabb_start = Instant::now();
    let mesh_report = model_writer
        .persist_mesh_results(&batch.mesh_results, &mesh_aabb_map, &mesh_pts_map)
        .await?;
    let inst_report = model_writer
        .persist_inst_relate_aabb(
            &batch.shape_insts,
            &batch.mesh_results,
            &mesh_aabb_map,
            skip_inst_relate_aabb,
        )
        .await?;
    println!(
        "[batch_stage] batch={} stage=writer_backend worker={} mesh_persist={:?}/{} inst_relate_aabb={:?}/{}",
        batch.batch_id,
        worker_id,
        mesh_report.status,
        mesh_report.item_count,
        inst_report.status,
        inst_report.item_count
    );
    let inst_aabb_ms = inst_aabb_start.elapsed().as_millis();
    drop(aabb_permit);

    let total_ms = batch.batch_started_at.elapsed().as_millis();
    println!(
        "[batch_perf] batch={} worker={} base_wait_ms={} base_write_ms={} mesh_wait_ms={} mesh_ms={} inst_aabb_wait_ms={} inst_aabb_ms={} total_ms={} mesh_cache_hit={} mesh_new_generated={} mesh_tasks={}",
        batch.batch_id,
        worker_id,
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
    Ok(())
}

async fn run_inst_aabb_writer(
    receiver: Receiver<BatchMeshOutput>,
    base_result_receiver: Receiver<(u64, u128, u128)>,
    completion_sender: Sender<BatchCompletion>,
    inst_aabb_semaphore: Arc<Semaphore>,
    worker_count: usize,
    mesh_aabb_map: Arc<DashMap<String, parry3d::bounding_volume::Aabb>>,
    mesh_pts_map: Arc<DashMap<u64, String>>,
    model_writer: Arc<dyn ModelWriterBackend>,
    skip_inst_relate_aabb: bool,
) -> anyhow::Result<()> {
    let worker_count = worker_count.max(1);
    let (joined_sender, joined_receiver) = flume::unbounded::<JoinedBatchOutput>();
    let mut handles = Vec::new();
    println!(
        "[batch_stage] stage=inst_aabb worker_pool={} skip_inst_relate_aabb={}",
        worker_count, skip_inst_relate_aabb
    );
    // 与 base/mesh 阶段相同的 fail-fast 策略。
    let abort = Arc::new(AtomicBool::new(false));
    for worker_id in 0..worker_count {
        let joined_receiver = joined_receiver.clone();
        let inst_aabb_semaphore = inst_aabb_semaphore.clone();
        let mesh_aabb_map = mesh_aabb_map.clone();
        let mesh_pts_map = mesh_pts_map.clone();
        let completion_sender = completion_sender.clone();
        let model_writer = model_writer.clone();
        let abort = Arc::clone(&abort);
        handles.push(tokio::spawn(async move {
            while let Ok(batch) = joined_receiver.recv_async().await {
                if abort.load(Ordering::Relaxed) {
                    break;
                }
                let step = process_inst_aabb_batch(
                    batch,
                    inst_aabb_semaphore.clone(),
                    mesh_aabb_map.clone(),
                    mesh_pts_map.clone(),
                    completion_sender.clone(),
                    model_writer.clone(),
                    skip_inst_relate_aabb,
                    worker_id,
                )
                .await;
                if let Err(error) = step {
                    abort.store(true, Ordering::Relaxed);
                    return Err(error);
                }
            }
            Ok::<(), anyhow::Error>(())
        }));
    }

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
                            joined_sender.send_async(batch).await?;
                        } else {
                            let (pending_mesh, pending_base) = joiner.pending_counts();
                            println!(
                                "[batch_stage] batch={} stage=join waiting=base_result pending_mesh_outputs={} pending_base_metrics={}",
                                batch_id, pending_mesh, pending_base
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
                            joined_sender.send_async(batch).await?;
                        } else {
                            let (pending_mesh, pending_base) = joiner.pending_counts();
                            println!(
                                "[batch_stage] batch={} stage=join waiting=mesh_output pending_mesh_outputs={} pending_base_metrics={}",
                                batch_id, pending_mesh, pending_base
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

    drop(joined_sender);
    for handle in handles {
        handle.await.map_err(|e| anyhow::anyhow!(e))??;
    }
    drop(completion_sender);
    Ok(())
}

pub(crate) struct WritePipelineStart {
    pub db_option: DbOptionExt,
    pub cleanup_hierarchy: Option<Arc<crate::generation_read::HierarchySnapshot>>,
    pub incremental_cleanup_roots: Vec<RefnoEnum>,
    pub model_writer: Arc<dyn ModelWriterBackend>,
    pub artifacts: Arc<GenerationArtifacts>,
    pub mesh_aabb_map: Arc<DashMap<String, parry3d::bounding_volume::Aabb>>,
    pub mesh_pts_map: Arc<DashMap<u64, String>>,
    pub channel_capacity: usize,
    pub base_write_concurrency: usize,
    pub mesh_compute_concurrency: usize,
    pub inst_aabb_write_concurrency: usize,
    pub skip_inst_relate_aabb: bool,
    pub skip_final_aabb_sweep: bool,
    pub use_surrealdb: bool,
}

pub(crate) struct WritePipelineReport {
    pub writer_finish: ModelWriterFinishReport,
    pub batch_count: u64,
    pub completed_batches: usize,
    pub mesh_cache_hits: usize,
    pub mesh_new_generated: usize,
    pub barrier_wait_ms: u128,
    pub missing_neg_carrier_count: usize,
    pub bool_tasks: Vec<BooleanTask>,
}

impl WritePipelineReport {
    pub(crate) fn is_drain_only(&self) -> bool {
        self.writer_finish.drain_only_stats.is_some()
    }
}

pub(crate) enum ModelWritePipeline {
    DrainOnly {
        handle: tokio::task::JoinHandle<anyhow::Result<ModelWriterFinishReport>>,
    },
    Full {
        sink_handle: tokio::task::JoinHandle<anyhow::Result<InsertHandleReport>>,
        base_writer_handle: tokio::task::JoinHandle<anyhow::Result<ModelWriterFinishReport>>,
        mesh_stage_handle: tokio::task::JoinHandle<anyhow::Result<()>>,
        inst_aabb_handle: tokio::task::JoinHandle<anyhow::Result<()>>,
        completion_receiver: Receiver<BatchCompletion>,
        model_writer: Arc<dyn ModelWriterBackend>,
        artifacts: Arc<GenerationArtifacts>,
        mesh_aabb_map: Arc<DashMap<String, parry3d::bounding_volume::Aabb>>,
        mesh_pts_map: Arc<DashMap<u64, String>>,
        skip_final_aabb_sweep: bool,
        use_surrealdb: bool,
    },
}

impl ModelWritePipeline {
    pub(crate) async fn start(
        request: WritePipelineStart,
    ) -> anyhow::Result<(Sender<ShapeInstancesData>, Self)> {
        if request.model_writer.writes_to_surreal() && !request.incremental_cleanup_roots.is_empty()
        {
            // cleanup 必须使用"已发布模型对应的旧层级切面"。无旧切面（尚无 model_gen
            // 锚点、model watermark=0）时没有可清理的已发布产物，跳过而不是回退到目标
            // 生成切面——回退会把删除根解析到已无删除 PE 的目标切面上，触发
            // MissingRequiredData（plan §4.3；delete-only cleanup 阻断项）。
            if let Some(cleanup_hierarchy) = request.cleanup_hierarchy.as_deref() {
                println!(
                    "[write-pipeline] incremental cleanup start: roots={}",
                    request.incremental_cleanup_roots.len()
                );
                crate::fast_model::gen_model::pdms_inst::pre_cleanup_for_regen_versioned(
                    &request.incremental_cleanup_roots,
                    cleanup_hierarchy,
                )
                .await?;
                println!("[write-pipeline] incremental cleanup complete");
            } else {
                println!(
                    "[write-pipeline] incremental cleanup skipped: 无已发布模型旧切面(model watermark=0), roots={}",
                    request.incremental_cleanup_roots.len()
                );
            }
        } else if !request.incremental_cleanup_roots.is_empty() {
            println!(
                "[write-pipeline] incremental cleanup skipped: writer={} roots={}",
                request.model_writer.name(),
                request.incremental_cleanup_roots.len()
            );
        }

        let (sender, receiver) = flume::bounded(request.channel_capacity);
        if !request.model_writer.runs_downstream_pipeline() {
            let handle = tokio::spawn(run_drain_only_sink(
                receiver,
                request.model_writer,
                request.artifacts,
            ));
            return Ok((sender, Self::DrainOnly { handle }));
        }

        let base_write_semaphore = Arc::new(Semaphore::new(request.base_write_concurrency.max(1)));
        let mesh_compute_semaphore =
            Arc::new(Semaphore::new(request.mesh_compute_concurrency.max(1)));
        let inst_aabb_semaphore =
            Arc::new(Semaphore::new(request.inst_aabb_write_concurrency.max(1)));
        let (base_writer_sender, base_writer_receiver) = flume::bounded(request.channel_capacity);
        let (base_result_sender, base_result_receiver) = flume::bounded(request.channel_capacity);
        let (mesh_stage_sender, mesh_stage_receiver) = flume::bounded(request.channel_capacity);
        let (mesh_output_sender, mesh_output_receiver) = flume::bounded(request.channel_capacity);
        let (completion_sender, completion_receiver) = flume::unbounded();
        let touched_refnos = Arc::new(std::sync::Mutex::new(HashSet::new()));

        let sink_handle = tokio::spawn(run_batch_sink(
            receiver,
            base_writer_sender,
            mesh_stage_sender,
            touched_refnos,
            Arc::clone(&request.artifacts),
        ));
        let base_writer_handle = tokio::spawn(run_base_writer(
            base_writer_receiver,
            base_result_sender,
            base_write_semaphore,
            request.base_write_concurrency,
            Arc::clone(&request.model_writer),
            Arc::clone(&request.artifacts),
        ));
        let mesh_stage_handle = tokio::spawn(run_mesh_stage(
            mesh_stage_receiver,
            mesh_output_sender,
            mesh_compute_semaphore,
            request.mesh_compute_concurrency,
            request.db_option.clone(),
            request.db_option.inner.gen_mesh,
            Arc::clone(&request.mesh_aabb_map),
            Arc::clone(&request.mesh_pts_map),
            Arc::clone(&request.artifacts),
        ));
        let inst_aabb_handle = tokio::spawn(run_inst_aabb_writer(
            mesh_output_receiver,
            base_result_receiver,
            completion_sender,
            inst_aabb_semaphore,
            request.inst_aabb_write_concurrency,
            Arc::clone(&request.mesh_aabb_map),
            Arc::clone(&request.mesh_pts_map),
            Arc::clone(&request.model_writer),
            request.skip_inst_relate_aabb,
        ));

        Ok((
            sender,
            Self::Full {
                sink_handle,
                base_writer_handle,
                mesh_stage_handle,
                inst_aabb_handle,
                completion_receiver,
                model_writer: request.model_writer,
                artifacts: request.artifacts,
                mesh_aabb_map: request.mesh_aabb_map,
                mesh_pts_map: request.mesh_pts_map,
                skip_final_aabb_sweep: request.skip_final_aabb_sweep,
                use_surrealdb: request.use_surrealdb,
            },
        ))
    }

    pub(crate) fn is_drain_only(&self) -> bool {
        matches!(self, Self::DrainOnly { .. })
    }

    pub(crate) async fn finish(self) -> anyhow::Result<WritePipelineReport> {
        match self {
            Self::DrainOnly { handle } => {
                let writer_finish = handle
                    .await
                    .map_err(|error| anyhow::anyhow!("drain-only sink 任务异常退出: {error}"))??;
                let batch_count = writer_finish
                    .drain_only_stats
                    .as_ref()
                    .map(|stats| stats.batches as u64)
                    .unwrap_or(0);
                Ok(WritePipelineReport {
                    writer_finish,
                    batch_count,
                    completed_batches: 0,
                    mesh_cache_hits: 0,
                    mesh_new_generated: 0,
                    barrier_wait_ms: 0,
                    missing_neg_carrier_count: 0,
                    bool_tasks: Vec::new(),
                })
            }
            Self::Full {
                sink_handle,
                base_writer_handle,
                mesh_stage_handle,
                inst_aabb_handle,
                completion_receiver,
                model_writer,
                artifacts,
                mesh_aabb_map,
                mesh_pts_map,
                skip_final_aabb_sweep,
                use_surrealdb,
            } => {
                let insert_result = sink_handle
                    .await
                    .map_err(|error| anyhow::anyhow!("batch sink 任务异常退出: {error}"))
                    .and_then(|result| result);
                let writer_result = base_writer_handle
                    .await
                    .map_err(|error| anyhow::anyhow!("base writer 任务异常退出: {error}"))
                    .and_then(|result| result);
                let mesh_result = mesh_stage_handle
                    .await
                    .map_err(|error| anyhow::anyhow!("mesh stage 任务异常退出: {error}"))
                    .and_then(|result| result);

                let barrier_wait_start = Instant::now();
                let inst_aabb_result = inst_aabb_handle
                    .await
                    .map_err(|error| anyhow::anyhow!("inst aabb writer 任务异常退出: {error}"))
                    .and_then(|result| result);
                let writer_finish = writer_result?;
                mesh_result?;
                inst_aabb_result?;
                let insert_report = insert_result?;
                let mut completed_batches = 0usize;
                let mut mesh_cache_hits = 0usize;
                let mut mesh_new_generated = 0usize;
                while let Ok(completion) = completion_receiver.recv_async().await {
                    completed_batches += 1;
                    mesh_cache_hits += completion.mesh_cache_hits;
                    mesh_new_generated += completion.mesh_new_generated;
                }
                let barrier_wait_ms = barrier_wait_start.elapsed().as_millis();
                crate::perf_metrics::add_generate_counters(mesh_new_generated, mesh_cache_hits);

                if skip_final_aabb_sweep {
                    println!("[write-pipeline] final_sweep skipped by generation contract");
                } else {
                    let sweep_start = Instant::now();
                    let report = model_writer
                        .finalize_mesh_entities(&mesh_aabb_map, &mesh_pts_map)
                        .await?;
                    println!(
                        "[write-pipeline] final_sweep aabb={} pts={} status={:?} elapsed_ms={}",
                        mesh_aabb_map.len(),
                        mesh_pts_map.len(),
                        report.status,
                        sweep_start.elapsed().as_millis()
                    );
                }

                let missing_neg_carrier_count = artifacts.missing_neg_carriers()?.len();
                let (bool_tasks, relation_artifacts) = artifacts.take_run_outputs()?;
                if use_surrealdb {
                    let report = model_writer
                        .reconcile_missing_neg_relations(&relation_artifacts, artifacts.tubi_info())
                        .await?;
                    println!(
                        "[write-pipeline] relation reconcile status={:?} item_count={} skipped_reason={:?}",
                        report.status, report.item_count, report.skipped_reason
                    );
                }

                Ok(WritePipelineReport {
                    writer_finish,
                    batch_count: insert_report.batch_cnt,
                    completed_batches,
                    mesh_cache_hits,
                    mesh_new_generated,
                    barrier_wait_ms,
                    missing_neg_carrier_count,
                    bool_tasks,
                })
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fast_model::gen_model::model_writer::{
        BooleanBridgeReport, BooleanBridgeRequest, ModelWriteBatchReport, ModelWriterStageReport,
        create_model_writer,
    };
    use crate::options::ModelWriterMode;
    use aios_core::parsed_data::TubiInfoData;
    use parry3d::bounding_volume::Aabb;

    struct FailingWriter {
        cleanup_fails: bool,
    }

    #[async_trait::async_trait]
    impl ModelWriterBackend for FailingWriter {
        fn name(&self) -> &'static str {
            "failing-test-writer"
        }

        fn writes_to_surreal(&self) -> bool {
            false
        }

        fn runs_downstream_pipeline(&self) -> bool {
            false
        }

        async fn cleanup(&self) -> anyhow::Result<ModelWriterStageReport> {
            if self.cleanup_fails {
                anyhow::bail!("injected cleanup failure");
            }
            Ok(ModelWriterStageReport::executed("cleanup", 0))
        }

        async fn write_base_batch(
            &self,
            _batch: &ShapeInstancesData,
        ) -> anyhow::Result<ModelWriteBatchReport> {
            anyhow::bail!("injected worker failure")
        }

        async fn persist_mesh_results(
            &self,
            _mesh_results: &HashMap<u64, MeshResult>,
            _mesh_aabb_map: &DashMap<String, Aabb>,
            _mesh_pts_map: &DashMap<u64, String>,
        ) -> anyhow::Result<ModelWriterStageReport> {
            unreachable!("cleanup/worker propagation tests do not run mesh persistence")
        }

        async fn persist_inst_relate_aabb(
            &self,
            _shape_insts: &ShapeInstancesData,
            _mesh_results: &HashMap<u64, MeshResult>,
            _mesh_aabb_map: &DashMap<String, Aabb>,
            _skip_inst_relate_aabb: bool,
        ) -> anyhow::Result<ModelWriterStageReport> {
            unreachable!("cleanup/worker propagation tests do not run AABB persistence")
        }

        async fn reconcile_missing_neg_relations(
            &self,
            _artifacts: &ShapeInstancesData,
            _tubi_info: &DashMap<String, TubiInfoData>,
        ) -> anyhow::Result<ModelWriterStageReport> {
            unreachable!("cleanup/worker propagation tests do not reconcile relations")
        }

        async fn run_boolean_bridge(
            &self,
            _request: BooleanBridgeRequest,
        ) -> anyhow::Result<BooleanBridgeReport> {
            unreachable!("cleanup/worker propagation tests do not run boolean bridge")
        }
    }

    fn sample_mesh_output(batch_id: u64) -> BatchMeshOutput {
        BatchMeshOutput {
            batch_id,
            shape_insts: Arc::new(ShapeInstancesData::default()),
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
    fn stage_joiner_handles_mesh_then_base() {
        let mut joiner = BatchStageJoiner::default();
        assert!(joiner.push_mesh_output(sample_mesh_output(7)).is_none());
        let ready = joiner.push_base_metrics(7, 11, 13).expect("joined");
        assert_eq!(ready.batch_id, 7);
        assert_eq!(ready.base_wait_ms, 11);
        assert_eq!(ready.base_write_ms, 13);
    }

    #[test]
    fn stage_joiner_handles_base_then_mesh() {
        let mut joiner = BatchStageJoiner::default();
        assert!(joiner.push_base_metrics(9, 3, 4).is_none());
        let ready = joiner
            .push_mesh_output(sample_mesh_output(9))
            .expect("joined");
        assert_eq!(ready.batch_id, 9);
        assert_eq!(ready.base_wait_ms, 3);
        assert_eq!(ready.base_write_ms, 4);
    }

    #[tokio::test]
    async fn closed_geometry_channel_completes_sink() {
        let (sender, receiver) = flume::unbounded();
        let (base_sender, _base_receiver) = flume::unbounded();
        let (mesh_sender, _mesh_receiver) = flume::unbounded();
        drop(sender);

        let report = run_batch_sink(
            receiver,
            base_sender,
            mesh_sender,
            Arc::new(std::sync::Mutex::new(HashSet::new())),
            Arc::new(GenerationArtifacts::new(1)),
        )
        .await
        .expect("closed channel should complete");
        assert_eq!(report.batch_cnt, 0);
    }

    #[tokio::test]
    async fn full_and_drain_only_sinks_record_same_geometry_artifact() {
        let drain_artifacts = Arc::new(GenerationArtifacts::new(42));
        let (drain_sender, drain_receiver) = flume::unbounded();
        drain_sender
            .send(ShapeInstancesData::default())
            .expect("send drain batch");
        drop(drain_sender);
        run_drain_only_sink(
            drain_receiver,
            create_model_writer(
                ModelWriterMode::DrainOnly,
                Arc::new(DashMap::new()),
                Arc::new(std::sync::Mutex::new(HashSet::new())),
                None,
            ),
            Arc::clone(&drain_artifacts),
        )
        .await
        .expect("drain-only sink");

        let full_artifacts = Arc::new(GenerationArtifacts::new(42));
        let (full_sender, full_receiver) = flume::unbounded();
        let (base_sender, _base_receiver) = flume::unbounded();
        let (mesh_sender, _mesh_receiver) = flume::unbounded();
        full_sender
            .send(ShapeInstancesData::default())
            .expect("send full batch");
        drop(full_sender);
        run_batch_sink(
            full_receiver,
            base_sender,
            mesh_sender,
            Arc::new(std::sync::Mutex::new(HashSet::new())),
            Arc::clone(&full_artifacts),
        )
        .await
        .expect("full sink");

        assert_eq!(
            drain_artifacts
                .summary()
                .expect("drain summary")
                .geometry_artifact_hash,
            full_artifacts
                .summary()
                .expect("full summary")
                .geometry_artifact_hash
        );
    }

    #[tokio::test]
    async fn writer_cleanup_failure_is_returned() {
        let (sender, receiver) = flume::unbounded();
        drop(sender);
        let result = run_drain_only_sink(
            receiver,
            Arc::new(FailingWriter {
                cleanup_fails: true,
            }),
            Arc::new(GenerationArtifacts::new(1)),
        )
        .await;
        assert!(
            result
                .expect_err("cleanup failure must propagate")
                .to_string()
                .contains("injected cleanup failure")
        );
    }

    #[tokio::test]
    async fn writer_worker_failure_is_returned() {
        let (sender, receiver) = flume::unbounded();
        sender
            .send(PipelineBatch {
                batch_id: 1,
                shape_insts: Arc::new(ShapeInstancesData::default()),
                batch_started_at: Instant::now(),
            })
            .expect("send batch");
        drop(sender);
        let (result_sender, _result_receiver) = flume::unbounded();
        let result = run_base_writer(
            receiver,
            result_sender,
            Arc::new(Semaphore::new(1)),
            1,
            Arc::new(FailingWriter {
                cleanup_fails: false,
            }),
            Arc::new(GenerationArtifacts::new(1)),
        )
        .await;
        assert!(
            result
                .expect_err("worker failure must propagate")
                .to_string()
                .contains("injected worker failure")
        );
    }

    #[tokio::test]
    async fn unmatched_stage_output_is_reported() {
        let (mesh_sender, mesh_receiver) = flume::unbounded();
        let (base_sender, base_receiver) = flume::unbounded();
        let (completion_sender, _completion_receiver) = flume::unbounded();
        mesh_sender.send(sample_mesh_output(3)).expect("send mesh");
        drop(mesh_sender);
        drop(base_sender);

        let writer = create_model_writer(
            ModelWriterMode::DrainOnly,
            Arc::new(DashMap::new()),
            Arc::new(std::sync::Mutex::new(HashSet::new())),
            None,
        );
        let result = run_inst_aabb_writer(
            mesh_receiver,
            base_receiver,
            completion_sender,
            Arc::new(Semaphore::new(1)),
            1,
            Arc::new(DashMap::new()),
            Arc::new(DashMap::new()),
            writer,
            false,
        )
        .await;
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("batch stage join 未收敛")
        );
    }
}
