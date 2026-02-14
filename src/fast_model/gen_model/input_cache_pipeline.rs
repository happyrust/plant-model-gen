//! LOOP/PRIM 输入缓存的 key-driven 流水线（prefetch -> write -> key -> consume）。
//!
//! 目标（M1）：
//! - 以 batch 为单位：预取输入 -> 写入 `geom_input_cache` -> 发送 key -> consumer 按 key 取回并消费。
//! - Smoke test 不依赖 SurrealDB：只验证“写入 -> 发 key -> 按 key 读回”的链路。

use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use aios_core::geometry::ShapeInstancesData;
use aios_core::RefnoEnum;
use dashmap::DashMap;
use futures::stream::{self, StreamExt};
use glam::Vec3;
use tokio::sync::Semaphore;

use super::context::NounProcessContext;
use crate::fast_model::foyer_cache::geom_input_cache::{
    self, GeomInputBatch, GeomInputCacheManager, LoopInput, PrimInput,
};

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct ReadyBatchKey {
    pub dbnum: u32,
    pub batch_id: String,
}

#[derive(Clone, Debug)]
struct ReadyBatchTask {
    key: ReadyBatchKey,
    // 用于指标统计与 missing 报错上下文。
    refnos: Vec<RefnoEnum>,
}

const DEFAULT_IN_FLIGHT_WRITES: usize = 4;

static BATCH_ID_SEQ: AtomicU64 = AtomicU64::new(0);

#[derive(Default)]
struct PipelineStats {
    prefetch_count: AtomicU64,
    cache_hit_count: AtomicU64,
    cache_miss_hard_fail: AtomicU64,
    duplicate_prefetch_count: AtomicU64,
}

fn log_pipeline_stats(kind: &str, stats: &PipelineStats) {
    let prefetch_count = stats.prefetch_count.load(Ordering::Relaxed);
    let cache_hit_count = stats.cache_hit_count.load(Ordering::Relaxed);
    let cache_miss_hard_fail = stats.cache_miss_hard_fail.load(Ordering::Relaxed);
    let duplicate_prefetch_count = stats.duplicate_prefetch_count.load(Ordering::Relaxed);
    let total = cache_hit_count + cache_miss_hard_fail;
    let cache_hit_rate = if total == 0 {
        0.0
    } else {
        (cache_hit_count as f64) * 100.0 / (total as f64)
    };

    println!(
        "[input_cache_pipeline][{}] prefetch_count={}, cache_hit_rate={:.2}%, cache_miss_hard_fail={}, duplicate_prefetch_count={}",
        kind, prefetch_count, cache_hit_rate, cache_miss_hard_fail, duplicate_prefetch_count
    );
}

fn make_batch_id(dbnum: u32) -> String {
    // 约定：batch_id 末尾为纯数字，便于 geom_input_cache 的 index counter 解析 max_seq。
    let t_ms = chrono::Utc::now().timestamp_millis().max(0) as u64;
    let seq = BATCH_ID_SEQ.fetch_add(1, Ordering::Relaxed) % 1000;
    format!("gi_{}_{}", dbnum, t_ms.saturating_mul(1000).saturating_add(seq))
}

fn now_ms() -> i64 {
    chrono::Utc::now().timestamp_millis()
}

fn group_refnos_by_dbnum(refnos: &[RefnoEnum]) -> anyhow::Result<HashMap<u32, Vec<RefnoEnum>>> {
    geom_input_cache::group_refnos_by_dbnum_strict(refnos)
}

async fn fetch_loop_inputs_map(
    db_option: &crate::options::DbOptionExt,
    refnos: &[RefnoEnum],
) -> anyhow::Result<HashMap<RefnoEnum, LoopInput>> {
    use aios_core::pdms_types::GENRAL_NEG_NOUN_NAMES;
    use crate::fast_model::query_provider;
    use crate::fast_model::shared;

    if refnos.is_empty() {
        return Ok(HashMap::new());
    }

    let disable_batch = std::env::var("AIOS_GEN_INPUT_CACHE_NO_BATCH")
        .ok()
        .map(|v| v == "1")
        .unwrap_or(false);
    if !disable_batch {
        match fetch_loop_inputs_map_batch(db_option, refnos).await {
            Ok(v) => return Ok(v),
            Err(e) => eprintln!(
                "[input_cache_pipeline] fetch_loop_inputs_map_batch failed, fallback m1: err={}",
                e
            ),
        }
    }

    let mut inputs: HashMap<RefnoEnum, LoopInput> = HashMap::new();
    let mut skipped = 0usize;

    for &refno in refnos {
        // 1) attmap
        let attmap = match aios_core::get_named_attmap(refno).await {
            Ok(a) => a,
            Err(_) => {
                skipped += 1;
                continue;
            }
        };

        // 2) world_transform
        let world_transform = match crate::fast_model::transform_cache::get_world_transform_cache_first(
            Some(db_option),
            refno,
        )
        .await
        {
            Ok(Some(t)) => t,
            _ => {
                skipped += 1;
                continue;
            }
        };

        // 3) loops + height
        let loop_res = match aios_core::fetch_loops_and_height(refno).await {
            Ok(r) => r,
            Err(_) => {
                skipped += 1;
                continue;
            }
        };

        // 4) owner_refno + owner_type
        let (owner_refno, owner_type) = shared::get_owner_info_from_attr(&attmap).await;

        // 5) visible
        let visible = attmap.is_visible_by_level(None).unwrap_or(true);

        // 6) neg_refnos
        let neg_refnos = query_provider::query_multi_descendants_with_self(
            &[refno],
            &GENRAL_NEG_NOUN_NAMES,
            false,
        )
        .await
        .unwrap_or_default();

        // 7) cmpf_neg_refnos
        let cmpf_neg_refnos = if !attmap.is_neg() {
            let cmpf_refnos =
                query_provider::get_descendants_by_types(refno, &["CMPF"], None)
                    .await
                    .unwrap_or_default();
            if !cmpf_refnos.is_empty() {
                query_provider::query_multi_descendants(&cmpf_refnos, &GENRAL_NEG_NOUN_NAMES)
                    .await
                    .unwrap_or_default()
            } else {
                vec![]
            }
        } else {
            vec![]
        };

        inputs.insert(
            refno,
            LoopInput {
                refno,
                attmap,
                world_transform,
                loops: loop_res.loops,
                height: loop_res.height,
                owner_refno,
                owner_type,
                visible,
                neg_refnos,
                cmpf_neg_refnos,
            },
        );
    }

    if skipped > 0 {
        // 仅用于调试：M1 阶段不要让单 refno 的失败阻断 pipeline。
        // 实际模型生成是否完整，由下游的旧路径处理函数决定（此处只负责尽力写缓存）。
        eprintln!(
            "[input_cache_pipeline] fetch_loop_inputs_map: skipped={}/{}",
            skipped,
            refnos.len()
        );
    }

    Ok(inputs)
}

async fn fetch_loop_inputs_map_batch(
    db_option: &crate::options::DbOptionExt,
    refnos: &[RefnoEnum],
) -> anyhow::Result<HashMap<RefnoEnum, LoopInput>> {
    use aios_core::pdms_types::GENRAL_NEG_NOUN_NAMES;
    use crate::fast_model::query_provider;
    use crate::fast_model::shared;

    if refnos.is_empty() {
        return Ok(HashMap::new());
    }

    let t0 = std::time::Instant::now();

    // 1) attmap：批量拉取
    let att_list = query_provider::get_attmaps_batch(refnos).await.unwrap_or_default();
    let mut attmap_map: HashMap<RefnoEnum, aios_core::NamedAttrMap> = HashMap::new();
    for att in att_list {
        let r = att.get_refno_or_default();
        if r.is_valid() {
            attmap_map.insert(r, att);
        }
    }

    // 2) world_transform：cache-first 批量（foyer hit + pe_transform miss batch）
    let world_map = crate::fast_model::transform_cache::get_world_transforms_cache_first_batch(
        Some(db_option),
        refnos,
    )
    .await?;

    // 3) loops + height：并发 per-refno（过渡方案；后续可做深层批量）
    const LOOP_CONCURRENCY: usize = 16;
    let loop_rows = stream::iter(refnos.iter().copied())
        .map(|r| async move {
            match aios_core::fetch_loops_and_height(r).await {
                Ok(v) => Some((r, (v.loops, v.height))),
                Err(_) => None,
            }
        })
        .buffer_unordered(LOOP_CONCURRENCY)
        .collect::<Vec<_>>()
        .await;
    let mut loop_map: HashMap<RefnoEnum, (Vec<Vec<Vec3>>, f32)> = HashMap::new();
    for row in loop_rows {
        if let Some((r, v)) = row {
            loop_map.insert(r, v);
        }
    }

    // 4) neg_refnos：TreeIndex 路径（小并发）
    const NEG_CONCURRENCY: usize = 32;
    let neg_rows = stream::iter(refnos.iter().copied())
        .map(|r| async move {
            let v = query_provider::query_multi_descendants_with_self(
                &[r],
                &GENRAL_NEG_NOUN_NAMES,
                false,
            )
            .await
            .unwrap_or_default();
            (r, v)
        })
        .buffer_unordered(NEG_CONCURRENCY)
        .collect::<Vec<_>>()
        .await;
    let neg_map: HashMap<RefnoEnum, Vec<RefnoEnum>> = neg_rows.into_iter().collect();

    let mut inputs: HashMap<RefnoEnum, LoopInput> = HashMap::new();
    let mut skipped = 0usize;

    for &refno in refnos {
        let Some(attmap) = attmap_map.get(&refno).cloned() else {
            skipped += 1;
            continue;
        };
        let Some(world_transform) = world_map.get(&refno).cloned() else {
            skipped += 1;
            continue;
        };
        let Some((loops, height)) = loop_map.get(&refno).cloned() else {
            skipped += 1;
            continue;
        };

        let (owner_refno, owner_type) = shared::get_owner_info_from_attr(&attmap).await;
        let visible = attmap.is_visible_by_level(None).unwrap_or(true);

        let neg_refnos = neg_map.get(&refno).cloned().unwrap_or_default();

        // cmpf_neg_refnos：先沿用旧逻辑（TreeIndex 很快；后续可再做批量）
        let cmpf_neg_refnos = if !attmap.is_neg() {
            let cmpf_refnos = query_provider::get_descendants_by_types(refno, &["CMPF"], None)
                .await
                .unwrap_or_default();
            if !cmpf_refnos.is_empty() {
                query_provider::query_multi_descendants(&cmpf_refnos, &GENRAL_NEG_NOUN_NAMES)
                    .await
                    .unwrap_or_default()
            } else {
                vec![]
            }
        } else {
            vec![]
        };

        inputs.insert(
            refno,
            LoopInput {
                refno,
                attmap,
                world_transform,
                loops,
                height,
                owner_refno,
                owner_type,
                visible,
                neg_refnos,
                cmpf_neg_refnos,
            },
        );
    }

    if skipped > 0 {
        eprintln!(
            "[input_cache_pipeline] fetch_loop_inputs_map_batch: skipped={}/{} elapsed={} ms",
            skipped,
            refnos.len(),
            t0.elapsed().as_millis()
        );
    }

    Ok(inputs)
}

async fn fetch_prim_inputs_map(
    db_option: &crate::options::DbOptionExt,
    refnos: &[RefnoEnum],
) -> anyhow::Result<HashMap<RefnoEnum, PrimInput>> {
    use aios_core::pdms_types::GENRAL_NEG_NOUN_NAMES;
    use crate::fast_model::query_provider;
    use crate::fast_model::shared;

    if refnos.is_empty() {
        return Ok(HashMap::new());
    }

    let disable_batch = std::env::var("AIOS_GEN_INPUT_CACHE_NO_BATCH")
        .ok()
        .map(|v| v == "1")
        .unwrap_or(false);
    if !disable_batch {
        match fetch_prim_inputs_map_batch(db_option, refnos).await {
            Ok(v) => return Ok(v),
            Err(e) => eprintln!(
                "[input_cache_pipeline] fetch_prim_inputs_map_batch failed, fallback m1: err={}",
                e
            ),
        }
    }

    let mut inputs: HashMap<RefnoEnum, PrimInput> = HashMap::new();
    let mut skipped = 0usize;

    for &refno in refnos {
        // 1) attmap
        let attmap = match aios_core::get_named_attmap(refno).await {
            Ok(a) => a,
            Err(_) => {
                skipped += 1;
                continue;
            }
        };

        // 2) world_transform
        let world_transform = match crate::fast_model::transform_cache::get_world_transform_cache_first(
            Some(db_option),
            refno,
        )
        .await
        {
            Ok(Some(t)) => t,
            _ => {
                skipped += 1;
                continue;
            }
        };

        // 3) owner_refno + owner_type
        let (owner_refno, owner_type) = shared::get_owner_info_from_attr(&attmap).await;

        // 4) visible
        let visible = attmap.is_visible_by_level(None).unwrap_or(true);

        // 5) neg_refnos
        let neg_refnos = query_provider::query_multi_descendants_with_self(
            &[refno],
            &GENRAL_NEG_NOUN_NAMES,
            false,
        )
        .await
        .unwrap_or_default();

        // 6) poly_extra（仅 POHE/POLYHE）
        let poly_extra = match attmap.get_type_str() {
            "POHE" | "POLYHE" => match geom_input_cache::try_build_prim_poly_extra(refno).await {
                Ok(v) => v,
                Err(e) => {
                    eprintln!(
                        "[input_cache_pipeline] fetch_prim_inputs_map: refno={} 构造 poly_extra 失败: {}",
                        refno, e
                    );
                    None
                }
            },
            _ => None,
        };

        inputs.insert(
            refno,
            PrimInput {
                refno,
                attmap,
                world_transform,
                owner_refno,
                owner_type,
                visible,
                neg_refnos,
                poly_extra,
            },
        );
    }

    if skipped > 0 {
        eprintln!(
            "[input_cache_pipeline] fetch_prim_inputs_map: skipped={}/{}",
            skipped,
            refnos.len()
        );
    }

    Ok(inputs)
}

async fn fetch_prim_inputs_map_batch(
    db_option: &crate::options::DbOptionExt,
    refnos: &[RefnoEnum],
) -> anyhow::Result<HashMap<RefnoEnum, PrimInput>> {
    use aios_core::pdms_types::GENRAL_NEG_NOUN_NAMES;
    use crate::fast_model::query_provider;
    use crate::fast_model::shared;

    if refnos.is_empty() {
        return Ok(HashMap::new());
    }

    let t0 = std::time::Instant::now();

    // 1) attmap：批量拉取
    let att_list = query_provider::get_attmaps_batch(refnos).await.unwrap_or_default();
    let mut attmap_map: HashMap<RefnoEnum, aios_core::NamedAttrMap> = HashMap::new();
    for att in att_list {
        let r = att.get_refno_or_default();
        if r.is_valid() {
            attmap_map.insert(r, att);
        }
    }

    // 2) world_transform：cache-first 批量（foyer hit + pe_transform miss batch）
    let world_map = crate::fast_model::transform_cache::get_world_transforms_cache_first_batch(
        Some(db_option),
        refnos,
    )
    .await?;

    // 3) neg_refnos：TreeIndex 路径（小并发）
    const NEG_CONCURRENCY: usize = 32;
    let neg_rows = stream::iter(refnos.iter().copied())
        .map(|r| async move {
            let v = query_provider::query_multi_descendants_with_self(
                &[r],
                &GENRAL_NEG_NOUN_NAMES,
                false,
            )
            .await
            .unwrap_or_default();
            (r, v)
        })
        .buffer_unordered(NEG_CONCURRENCY)
        .collect::<Vec<_>>()
        .await;
    let neg_map: HashMap<RefnoEnum, Vec<RefnoEnum>> = neg_rows.into_iter().collect();

    // 4) poly_extra：仅 POHE/POLYHE，per-refno 并发（过渡方案）
    let poly_targets: Vec<RefnoEnum> = refnos
        .iter()
        .copied()
        .filter(|r| {
            attmap_map
                .get(r)
                .map(|a| matches!(a.get_type_str(), "POHE" | "POLYHE"))
                .unwrap_or(false)
        })
        .collect();

    const POLY_CONCURRENCY: usize = 8;
    let poly_rows = stream::iter(poly_targets.iter().copied())
        .map(|r| async move {
            let v = geom_input_cache::try_build_prim_poly_extra(r)
                .await
                .ok()
                .flatten();
            (r, v)
        })
        .buffer_unordered(POLY_CONCURRENCY)
        .collect::<Vec<_>>()
        .await;

    let poly_map: HashMap<RefnoEnum, geom_input_cache::PrimPolyExtra> = poly_rows
        .into_iter()
        .filter_map(|(r, v)| v.map(|x| (r, x)))
        .collect();

    let mut inputs: HashMap<RefnoEnum, PrimInput> = HashMap::new();
    let mut skipped = 0usize;

    for &refno in refnos {
        let Some(attmap) = attmap_map.get(&refno).cloned() else {
            skipped += 1;
            continue;
        };
        let Some(world_transform) = world_map.get(&refno).cloned() else {
            skipped += 1;
            continue;
        };

        let (owner_refno, owner_type) = shared::get_owner_info_from_attr(&attmap).await;
        let visible = attmap.is_visible_by_level(None).unwrap_or(true);

        let neg_refnos = neg_map.get(&refno).cloned().unwrap_or_default();
        let poly_extra = poly_map.get(&refno).cloned();

        inputs.insert(
            refno,
            PrimInput {
                refno,
                attmap,
                world_transform,
                owner_refno,
                owner_type,
                visible,
                neg_refnos,
                poly_extra,
            },
        );
    }

    if skipped > 0 {
        eprintln!(
            "[input_cache_pipeline] fetch_prim_inputs_map_batch: skipped={}/{} elapsed={} ms",
            skipped,
            refnos.len(),
            t0.elapsed().as_millis()
        );
    }

    Ok(inputs)
}

async fn consume_batches_loop(
    ctx: NounProcessContext,
    cache: &'static GeomInputCacheManager,
    rx: flume::Receiver<ReadyBatchTask>,
    loop_sjus_map_arc: Arc<DashMap<RefnoEnum, (Vec3, f32)>>,
    sender: flume::Sender<ShapeInstancesData>,
    stats: Arc<PipelineStats>,
) -> anyhow::Result<()> {
    while let Ok(task) = rx.recv_async().await {
        let batch_opt = cache.get(task.key.dbnum, &task.key.batch_id).await;
        if let Some(batch) = batch_opt {
            stats
                .cache_hit_count
                .fetch_add(batch.loop_inputs.len() as u64, Ordering::Relaxed);
            if !crate::fast_model::loop_model::gen_loop_geos_from_inputs(
                ctx.db_option.clone(),
                batch.loop_inputs,
                loop_sjus_map_arc.clone(),
                sender.clone(),
            )
            .await?
            {
                anyhow::bail!("gen_loop_geos_from_inputs failed");
            }
        } else {
            stats
                .cache_miss_hard_fail
                .fetch_add(task.refnos.len() as u64, Ordering::Relaxed);
            anyhow::bail!(
                "[input_cache_pipeline] loop batch missing (strict cache mode): dbnum={}, batch_id={}, refnos={}",
                task.key.dbnum,
                task.key.batch_id,
                task.refnos.len()
            );
        }
    }
    Ok(())
}

async fn consume_batches_prim(
    ctx: NounProcessContext,
    cache: &'static GeomInputCacheManager,
    rx: flume::Receiver<ReadyBatchTask>,
    sender: flume::Sender<ShapeInstancesData>,
    stats: Arc<PipelineStats>,
) -> anyhow::Result<()> {
    while let Ok(task) = rx.recv_async().await {
        let batch_opt = cache.get(task.key.dbnum, &task.key.batch_id).await;
        if let Some(batch) = batch_opt {
            stats
                .cache_hit_count
                .fetch_add(batch.prim_inputs.len() as u64, Ordering::Relaxed);
            if !crate::fast_model::prim_model::gen_prim_geos_from_inputs(
                ctx.db_option.clone(),
                batch.prim_inputs,
                sender.clone(),
            )
            .await?
            {
                anyhow::bail!("gen_prim_geos_from_inputs failed");
            }
        } else {
            stats
                .cache_miss_hard_fail
                .fetch_add(task.refnos.len() as u64, Ordering::Relaxed);
            anyhow::bail!(
                "[input_cache_pipeline] prim batch missing (strict cache mode): dbnum={}, batch_id={}, refnos={}",
                task.key.dbnum,
                task.key.batch_id,
                task.refnos.len()
            );
        }
    }
    Ok(())
}

pub async fn run_loop_pipeline_from_refnos(
    ctx: &NounProcessContext,
    loop_sjus_map_arc: Arc<DashMap<RefnoEnum, (Vec3, f32)>>,
    sender: flume::Sender<ShapeInstancesData>,
    refnos: &[RefnoEnum],
) -> anyhow::Result<()> {
    if refnos.is_empty() {
        return Ok(());
    }

    // 以全局 cache 为写入目的地，避免重复打开/加载 index 文件。
    geom_input_cache::init_global_geom_input_cache(ctx.db_option.as_ref()).await?;
    let cache = geom_input_cache::global_geom_input_cache()
        .ok_or_else(|| anyhow::anyhow!("geom_input_cache 未初始化"))?;

    let (tx, rx) = flume::unbounded::<ReadyBatchTask>();
    let consumer_ctx = ctx.clone();
    let consumer_loop_sjus_map = loop_sjus_map_arc.clone();
    let consumer_sender = sender.clone();
    let stats = Arc::new(PipelineStats::default());
    let consumer_stats = stats.clone();
    let consumer = tokio::spawn(async move {
        consume_batches_loop(
            consumer_ctx,
            cache,
            rx,
            consumer_loop_sjus_map,
            consumer_sender,
            consumer_stats,
        )
        .await
    });

    let sem = Arc::new(Semaphore::new(DEFAULT_IN_FLIGHT_WRITES));
    let mut join_set = tokio::task::JoinSet::<anyhow::Result<()>>::new();

    let chunk_size = ctx.batch_size.max(1);
    let groups = group_refnos_by_dbnum(refnos)?;
    for (dbnum, refs) in groups {
        for chunk in refs.chunks(chunk_size) {
            let permit = sem.clone().acquire_owned().await?;
            let tx = tx.clone();
            let db_option = ctx.db_option.clone();
            let refnos_vec: Vec<RefnoEnum> = chunk.to_vec();
            let stats = stats.clone();

            join_set.spawn(async move {
                let _permit = permit;
                let unique_count = refnos_vec.iter().copied().collect::<HashSet<_>>().len();
                if unique_count < refnos_vec.len() {
                    stats.duplicate_prefetch_count.fetch_add(
                        (refnos_vec.len() - unique_count) as u64,
                        Ordering::Relaxed,
                    );
                }

                let inputs = fetch_loop_inputs_map(db_option.as_ref(), &refnos_vec).await?;
                stats
                    .prefetch_count
                    .fetch_add(inputs.len() as u64, Ordering::Relaxed);
                let batch_id = make_batch_id(dbnum);
                cache.insert_batch(GeomInputBatch {
                    dbnum,
                    batch_id: batch_id.clone(),
                    created_at: now_ms(),
                    loop_inputs: inputs,
                    prim_inputs: HashMap::new(),
                });

                tx.send(ReadyBatchTask {
                    key: ReadyBatchKey { dbnum, batch_id },
                    refnos: refnos_vec,
                })
                .map_err(|e| anyhow::anyhow!("send ReadyBatchTask failed: {}", e))?;

                Ok(())
            });
        }
    }

    drop(tx);
    while let Some(res) = join_set.join_next().await {
        res.map_err(|e| anyhow::anyhow!("prefetch/write task join failed: {}", e))??;
    }

    let consumer_result = consumer
        .await
        .map_err(|e| anyhow::anyhow!("consumer join failed: {}", e))?;
    log_pipeline_stats("loop", &stats);
    consumer_result?;

    Ok(())
}

pub async fn run_prim_pipeline_from_refnos(
    ctx: &NounProcessContext,
    sender: flume::Sender<ShapeInstancesData>,
    refnos: &[RefnoEnum],
) -> anyhow::Result<()> {
    if refnos.is_empty() {
        return Ok(());
    }

    geom_input_cache::init_global_geom_input_cache(ctx.db_option.as_ref()).await?;
    let cache = geom_input_cache::global_geom_input_cache()
        .ok_or_else(|| anyhow::anyhow!("geom_input_cache 未初始化"))?;

    let (tx, rx) = flume::unbounded::<ReadyBatchTask>();
    let consumer_ctx = ctx.clone();
    let consumer_sender = sender.clone();
    let stats = Arc::new(PipelineStats::default());
    let consumer_stats = stats.clone();
    let consumer = tokio::spawn(async move {
        consume_batches_prim(consumer_ctx, cache, rx, consumer_sender, consumer_stats).await
    });

    let sem = Arc::new(Semaphore::new(DEFAULT_IN_FLIGHT_WRITES));
    let mut join_set = tokio::task::JoinSet::<anyhow::Result<()>>::new();

    let chunk_size = ctx.batch_size.max(1);
    let groups = group_refnos_by_dbnum(refnos)?;
    for (dbnum, refs) in groups {
        for chunk in refs.chunks(chunk_size) {
            let permit = sem.clone().acquire_owned().await?;
            let tx = tx.clone();
            let db_option = ctx.db_option.clone();
            let refnos_vec: Vec<RefnoEnum> = chunk.to_vec();
            let stats = stats.clone();

            join_set.spawn(async move {
                let _permit = permit;
                let unique_count = refnos_vec.iter().copied().collect::<HashSet<_>>().len();
                if unique_count < refnos_vec.len() {
                    stats.duplicate_prefetch_count.fetch_add(
                        (refnos_vec.len() - unique_count) as u64,
                        Ordering::Relaxed,
                    );
                }

                let inputs = fetch_prim_inputs_map(db_option.as_ref(), &refnos_vec).await?;
                stats
                    .prefetch_count
                    .fetch_add(inputs.len() as u64, Ordering::Relaxed);
                let batch_id = make_batch_id(dbnum);
                cache.insert_batch(GeomInputBatch {
                    dbnum,
                    batch_id: batch_id.clone(),
                    created_at: now_ms(),
                    loop_inputs: HashMap::new(),
                    prim_inputs: inputs,
                });

                tx.send(ReadyBatchTask {
                    key: ReadyBatchKey { dbnum, batch_id },
                    refnos: refnos_vec,
                })
                .map_err(|e| anyhow::anyhow!("send ReadyBatchTask failed: {}", e))?;

                Ok(())
            });
        }
    }

    drop(tx);
    while let Some(res) = join_set.join_next().await {
        res.map_err(|e| anyhow::anyhow!("prefetch/write task join failed: {}", e))??;
    }

    let consumer_result = consumer
        .await
        .map_err(|e| anyhow::anyhow!("consumer join failed: {}", e))?;
    log_pipeline_stats("prim", &stats);
    consumer_result?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    async fn consume_keys_from_cache(
        cache: &GeomInputCacheManager,
        rx: flume::Receiver<ReadyBatchKey>,
    ) -> anyhow::Result<Vec<GeomInputBatch>> {
        let mut got = Vec::new();
        while let Ok(key) = rx.recv_async().await {
            let batch = cache
                .get(key.dbnum, &key.batch_id)
                .await
                .ok_or_else(|| anyhow::anyhow!("batch missing: dbnum={}, batch_id={}", key.dbnum, key.batch_id))?;
            got.push(batch);
        }
        Ok(got)
    }

    #[tokio::test]
    async fn test_pipeline_key_driven_consume_smoke() {
        // NOTE: 这里不调用真实 fetch_*；只验证“写入->发 key->按 key 读回”的链路。
        // 期望：consumer 收到 2 个 key，并能从 cache get 到对应 batch。
        use tempfile::tempdir;

        let dir = tempdir().unwrap();
        let cache = GeomInputCacheManager::new(dir.path()).await.unwrap();

        let (tx, rx) = flume::unbounded::<ReadyBatchKey>();

        // 两个 batch：只要能 roundtrip 即可，不构造真实输入。
        let b1_id = "gi_1_100".to_string();
        cache.insert_batch(GeomInputBatch {
            dbnum: 1,
            batch_id: b1_id.clone(),
            created_at: 1,
            loop_inputs: HashMap::new(),
            prim_inputs: HashMap::new(),
        });
        tx.send(ReadyBatchKey {
            dbnum: 1,
            batch_id: b1_id.clone(),
        })
        .unwrap();

        let b2_id = "gi_1_101".to_string();
        cache.insert_batch(GeomInputBatch {
            dbnum: 1,
            batch_id: b2_id.clone(),
            created_at: 2,
            loop_inputs: HashMap::new(),
            prim_inputs: HashMap::new(),
        });
        tx.send(ReadyBatchKey {
            dbnum: 1,
            batch_id: b2_id.clone(),
        })
        .unwrap();

        drop(tx);

        let got = consume_keys_from_cache(&cache, rx).await.unwrap();
        cache.close().await.unwrap();

        assert_eq!(got.len(), 2);
        let ids: HashSet<String> = got.into_iter().map(|b| b.batch_id).collect();
        assert!(ids.contains(&b1_id));
        assert!(ids.contains(&b2_id));
    }
}
