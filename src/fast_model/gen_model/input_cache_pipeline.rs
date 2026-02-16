//! LOOP/PRIM 输入缓存的 per-refno 流水线（prefetch -> write -> refnos -> consume）。
//!
//! 流程：预取输入 -> 逐 refno 写入 `geom_input_cache` -> 发送 refnos 列表 -> consumer 按 refno 读回并消费。

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
    self, GeomInputCacheManager, LoopInput, PrimInput,
};

fn read_bool_env(name: &str) -> bool {
    std::env::var(name)
        .ok()
        .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
        .unwrap_or(false)
}

fn is_stage_timing_enabled() -> bool {
    read_bool_env("AIOS_GEN_INPUT_CACHE_STAGE_TIMING")
}

fn is_opt_cmpf_neg_enabled() -> bool {
    read_bool_env("AIOS_OPT_CMPF_NEG")
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct ReadyBatchKey {
    pub dbnum: u32,
}

#[derive(Clone, Debug)]
struct ReadyBatchTask {
    key: ReadyBatchKey,
    /// 本批次写入的 refnos（消费者按这些 refno 从 cache 逐条读取）。
    refnos: Vec<RefnoEnum>,
}

const DEFAULT_IN_FLIGHT_WRITES: usize = 4;

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
    use super::neg_query;

    if refnos.is_empty() {
        return Ok(HashMap::new());
    }

    let t0 = std::time::Instant::now();
    let tree_dir = db_option.get_scene_tree_dir();

    // 1) attmap：批量拉取
    let t_att = std::time::Instant::now();
    let att_list = query_provider::get_attmaps_batch(refnos).await.unwrap_or_default();
    let mut attmap_map: HashMap<RefnoEnum, aios_core::NamedAttrMap> = HashMap::new();
    for att in att_list {
        let r = att.get_refno_or_default();
        if r.is_valid() {
            attmap_map.insert(r, att);
        }
    }
    let att_ms = t_att.elapsed().as_millis();

    // 2) world_transform：cache-first 批量（foyer hit + pe_transform miss batch）
    let t_world = std::time::Instant::now();
    let world_map = crate::fast_model::transform_cache::get_world_transforms_cache_first_batch(
        Some(db_option),
        refnos,
    )
    .await?;
    let world_ms = t_world.elapsed().as_millis();

    // 3) loops + height：并发 per-refno（过渡方案；后续可做深层批量）
    let t_loops = std::time::Instant::now();
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
    let loops_ms = t_loops.elapsed().as_millis();

    // 4) neg_refnos：TreeIndex 路径（按 dbnum 分组，单次加载 index；返回 root -> Vec）
    let t_neg = std::time::Instant::now();
    let neg_map: HashMap<RefnoEnum, Vec<RefnoEnum>> =
        neg_query::query_descendants_map_by_dbnum(&tree_dir, refnos, &GENRAL_NEG_NOUN_NAMES, false)
            .unwrap_or_default();
    let neg_ms = t_neg.elapsed().as_millis();

    // 5) cmpf_neg_refnos：可选优化（避免 per-refno 二段查询导致的重复固定开销）
    let t_cmpf = std::time::Instant::now();
    let cmpf_neg_map_opt: Option<HashMap<RefnoEnum, Vec<RefnoEnum>>> = if is_opt_cmpf_neg_enabled()
    {
        let roots_need_cmpf: Vec<RefnoEnum> = refnos
            .iter()
            .copied()
            .filter(|r| {
                attmap_map
                    .get(r)
                    .map(|a| !a.is_neg())
                    .unwrap_or(false)
            })
            .collect();

        // root -> CMPF descendants
        let cmpf_map: HashMap<RefnoEnum, Vec<RefnoEnum>> = neg_query::query_descendants_map_by_dbnum(
            &tree_dir,
            &roots_need_cmpf,
            &["CMPF"],
            false,
        )
        .unwrap_or_default();

        // CMPF node -> neg descendants
        let mut all_cmpf: Vec<RefnoEnum> = Vec::new();
        let mut seen_cmpf: HashSet<RefnoEnum> = HashSet::new();
        for rows in cmpf_map.values() {
            for &r in rows {
                if r.is_valid() && seen_cmpf.insert(r) {
                    all_cmpf.push(r);
                }
            }
        }

        let cmpf_neg_node_map: HashMap<RefnoEnum, Vec<RefnoEnum>> =
            if all_cmpf.is_empty() {
                HashMap::new()
            } else {
                neg_query::query_descendants_map_by_dbnum(
                    &tree_dir,
                    &all_cmpf,
                    &GENRAL_NEG_NOUN_NAMES,
                    false,
                )
                .unwrap_or_default()
            };

        // root -> union(neg descendants under each CMPF)
        let mut root_map: HashMap<RefnoEnum, Vec<RefnoEnum>> =
            HashMap::with_capacity(roots_need_cmpf.len());
        for &root in &roots_need_cmpf {
            let mut out: Vec<RefnoEnum> = Vec::new();
            let mut seen: HashSet<RefnoEnum> = HashSet::new();
            if let Some(cmpf_nodes) = cmpf_map.get(&root) {
                for &cmpf in cmpf_nodes {
                    if let Some(negs) = cmpf_neg_node_map.get(&cmpf) {
                        for &n in negs {
                            if n.is_valid() && seen.insert(n) {
                                out.push(n);
                            }
                        }
                    }
                }
            }
            root_map.insert(root, out);
        }

        Some(root_map)
    } else {
        None
    };
    let cmpf_ms = t_cmpf.elapsed().as_millis();

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

        let cmpf_neg_refnos = if attmap.is_neg() {
            vec![]
        } else if let Some(cmpf_neg_map) = cmpf_neg_map_opt.as_ref() {
            cmpf_neg_map.get(&refno).cloned().unwrap_or_default()
        } else {
            // 兼容旧逻辑：逐 refno 两段查询（CMPF descendants -> neg descendants under CMPF）
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

    if is_stage_timing_enabled() {
        println!(
            "[input_cache_pipeline] fetch_loop_inputs_map_batch timings(ms): att={}, world={}, loops={}, neg={}, cmpf_neg={}, total={}, refnos={}",
            att_ms,
            world_ms,
            loops_ms,
            neg_ms,
            cmpf_ms,
            t0.elapsed().as_millis(),
            refnos.len()
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
    use super::neg_query;

    if refnos.is_empty() {
        return Ok(HashMap::new());
    }

    let t0 = std::time::Instant::now();
    let tree_dir = db_option.get_scene_tree_dir();

    // 1) attmap：批量拉取
    let t_att = std::time::Instant::now();
    let att_list = query_provider::get_attmaps_batch(refnos).await.unwrap_or_default();
    let mut attmap_map: HashMap<RefnoEnum, aios_core::NamedAttrMap> = HashMap::new();
    for att in att_list {
        let r = att.get_refno_or_default();
        if r.is_valid() {
            attmap_map.insert(r, att);
        }
    }
    let att_ms = t_att.elapsed().as_millis();

    // 2) world_transform：cache-first 批量（foyer hit + pe_transform miss batch）
    let t_world = std::time::Instant::now();
    let world_map = crate::fast_model::transform_cache::get_world_transforms_cache_first_batch(
        Some(db_option),
        refnos,
    )
    .await?;
    let world_ms = t_world.elapsed().as_millis();

    // 3) neg_refnos：TreeIndex 路径（按 dbnum 分组，单次加载 index；返回 root -> Vec）
    let t_neg = std::time::Instant::now();
    let neg_map: HashMap<RefnoEnum, Vec<RefnoEnum>> =
        neg_query::query_descendants_map_by_dbnum(&tree_dir, refnos, &GENRAL_NEG_NOUN_NAMES, false)
            .unwrap_or_default();
    let neg_ms = t_neg.elapsed().as_millis();

    // 4) poly_extra：仅 POHE/POLYHE，per-refno 并发（过渡方案）
    let t_poly = std::time::Instant::now();
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
    let poly_ms = t_poly.elapsed().as_millis();

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

    if is_stage_timing_enabled() {
        println!(
            "[input_cache_pipeline] fetch_prim_inputs_map_batch timings(ms): att={}, world={}, neg={}, poly={}, total={}, refnos={}",
            att_ms,
            world_ms,
            neg_ms,
            poly_ms,
            t0.elapsed().as_millis(),
            refnos.len()
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
        let dbnum = task.key.dbnum;
        let mut loop_inputs: HashMap<RefnoEnum, LoopInput> = HashMap::new();
        let mut miss_count = 0u64;
        for &refno in &task.refnos {
            if let Some(input) = cache.get_loop_input(dbnum, refno) {
                loop_inputs.insert(refno, input);
            } else {
                miss_count += 1;
            }
        }
        if loop_inputs.is_empty() && !task.refnos.is_empty() {
            stats
                .cache_miss_hard_fail
                .fetch_add(task.refnos.len() as u64, Ordering::Relaxed);
            anyhow::bail!(
                "[input_cache_pipeline] loop inputs all missing (strict cache mode): dbnum={}, refnos={}",
                dbnum,
                task.refnos.len()
            );
        }
        if miss_count > 0 {
            stats.cache_miss_hard_fail.fetch_add(miss_count, Ordering::Relaxed);
        }
        stats
            .cache_hit_count
            .fetch_add(loop_inputs.len() as u64, Ordering::Relaxed);
        if !crate::fast_model::loop_model::gen_loop_geos_from_inputs(
            ctx.db_option.clone(),
            loop_inputs,
            loop_sjus_map_arc.clone(),
            sender.clone(),
        )
        .await?
        {
            anyhow::bail!("gen_loop_geos_from_inputs failed");
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
        let dbnum = task.key.dbnum;
        let mut prim_inputs: HashMap<RefnoEnum, PrimInput> = HashMap::new();
        let mut miss_count = 0u64;
        for &refno in &task.refnos {
            if let Some(input) = cache.get_prim_input(dbnum, refno) {
                prim_inputs.insert(refno, input);
            } else {
                miss_count += 1;
            }
        }
        if prim_inputs.is_empty() && !task.refnos.is_empty() {
            stats
                .cache_miss_hard_fail
                .fetch_add(task.refnos.len() as u64, Ordering::Relaxed);
            anyhow::bail!(
                "[input_cache_pipeline] prim inputs all missing (strict cache mode): dbnum={}, refnos={}",
                dbnum,
                task.refnos.len()
            );
        }
        if miss_count > 0 {
            stats.cache_miss_hard_fail.fetch_add(miss_count, Ordering::Relaxed);
        }
        stats
            .cache_hit_count
            .fetch_add(prim_inputs.len() as u64, Ordering::Relaxed);
        if !crate::fast_model::prim_model::gen_prim_geos_from_inputs(
            ctx.db_option.clone(),
            prim_inputs,
            sender.clone(),
        )
        .await?
        {
            anyhow::bail!("gen_prim_geos_from_inputs failed");
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
    geom_input_cache::init_global_geom_input_cache();
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
                // per-refno 写入
                for (refno, input) in &inputs {
                    cache.insert_loop_input(dbnum, *refno, input);
                }

                tx.send(ReadyBatchTask {
                    key: ReadyBatchKey { dbnum },
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

    geom_input_cache::init_global_geom_input_cache();
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
                // per-refno 写入
                for (refno, input) in &inputs {
                    cache.insert_prim_input(dbnum, *refno, input);
                }

                tx.send(ReadyBatchTask {
                    key: ReadyBatchKey { dbnum },
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
    use geom_input_cache::CateInput;

    #[tokio::test]
    async fn test_pipeline_per_refno_roundtrip_smoke() {
        // 验证 per-refno 写入 -> ReadyBatchTask 传递 refnos -> per-refno 读回 的链路。
        use tempfile::tempdir;

        let dir = tempdir().unwrap();
        let cache = GeomInputCacheManager::new(dir.path()).await.unwrap();

        let r1: RefnoEnum = "100/200".into();
        let r2: RefnoEnum = "100/201".into();
        let dbnum = 1u32;

        // 写入两个 cate input（最轻量的类型，用于 smoke test）
        let cate1 = CateInput {
            refno: r1,
            attmap: aios_core::NamedAttrMap::default(),
            world_transform: Default::default(),
            owner_refno: None,
            owner_type: String::new(),
            visible: true,
            neg_refnos: vec![],
            cmpf_neg_refnos: vec![],
        };
        let cate2 = CateInput {
            refno: r2,
            attmap: aios_core::NamedAttrMap::default(),
            world_transform: Default::default(),
            owner_refno: None,
            owner_type: String::new(),
            visible: true,
            neg_refnos: vec![],
            cmpf_neg_refnos: vec![],
        };
        cache.insert_cate_input(dbnum, r1, &cate1);
        cache.insert_cate_input(dbnum, r2, &cate2);

        // 模拟 consumer 侧 per-refno 读回
        let got1 = cache.get_cate_input(dbnum, r1).await;
        let got2 = cache.get_cate_input(dbnum, r2).await;
        assert!(got1.is_some(), "r1 must be present");
        assert!(got2.is_some(), "r2 must be present");
        assert_eq!(got1.unwrap().refno, r1);
        assert_eq!(got2.unwrap().refno, r2);

        cache.close().await.unwrap();
    }
}
