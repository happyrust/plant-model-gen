//! 模型生成编排器
//!
//! 负责协调整个模型生成流程：
//! - GenPipeline 单管线路由（Full / Manual / Debug / Incremental）
//! - 几何体生成、Mesh 生成、布尔运算的编排
//! - 增量更新、手动 refno、调试模式的处理
//! - 空间索引和截图捕获的触发
use crate::data_interface::db_meta_manager::db_meta;
use crate::data_interface::increment_record::IncrGeoUpdateLog;
use crate::fast_model::export_model::export_prepack_lod::export_instances_json_for_dbnos;
use crate::fast_model::export_model::export_prepack_lod::export_instances_json_for_refnos_grouped_by_dbno;
use crate::fast_model::export_model::export_prepack_lod::export_prepack_lod_for_refnos;
use crate::fast_model::gen_model::model_writer::{
    BooleanBridgeReport, BooleanBridgeRequest, GenerationArtifacts, GenerationArtifactsSummary,
    create_model_writer,
};
use crate::fast_model::unit_converter::LengthUnit;
use crate::generation_read::{GenerationReadSpec, SessionMetricsSnapshot};
use crate::options::{DbOptionExt, MeshFormat};
use aios_core::RefnoEnum;
use dashmap::DashMap;
use std::collections::{BTreeSet, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Instant;

use super::cache_miss_report;
use super::config::{ExecutionTuning, GenPipelineConfig, GenerationContract};
use super::context::GenerationReadContext;
use super::errors::{GenPipelineError, Result};
use super::gen_pipeline::{
    execute_generation_targets, resolve_full_generation_targets,
    resolve_incremental_generation_targets, resolve_root_generation_targets,
};
use super::models::NounCategory;
use super::write_pipeline::{ModelWritePipeline, WritePipelineStart};
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

fn should_start_write_pipeline(contract: &GenerationContract) -> bool {
    !contract.dry_run()
}

#[derive(Debug, Clone)]
pub struct GenModelResult {
    pub success: bool,
    pub authoritative_snapshot_id: u64,
    pub artifacts: Option<GenerationArtifactsSummary>,
    pub read_metrics: SessionMetricsSnapshot,
    pub provenance: GenerationRunProvenance,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GenerationRunProvenance {
    input_manifest_hash: String,
    contract_hash: String,
    target_hash: String,
}

impl GenerationRunProvenance {
    fn new(input_manifest_hash: String, contract_hash: String, target_hash: String) -> Self {
        Self {
            input_manifest_hash,
            contract_hash,
            target_hash,
        }
    }

    pub fn input_manifest_hash(&self) -> &str {
        &self.input_manifest_hash
    }

    pub fn contract_hash(&self) -> &str {
        &self.contract_hash
    }

    pub fn target_hash(&self) -> &str {
        &self.target_hash
    }
}

fn validated_read_metrics(
    generation_read: &GenerationReadContext,
) -> anyhow::Result<SessionMetricsSnapshot> {
    let metrics = generation_read.session.metrics();
    metrics.assert_batch_first_hot_path()?;
    Ok(metrics)
}

/// 主入口函数：生成所有几何体数据
///
/// 这是主要的公共 API，统一收敛到 GenPipeline 生成管线：
/// - Full：按 `gen_pipeline_enabled_target_types` 从 TreeIndex 提取入口 roots
/// - Manual / Debug / Incremental：构造 roots 并集后以 seed_roots 直入
///
/// # Arguments
/// * `manual_refnos` - 手动指定的 refno 列表
/// * `db_option` - 数据库配置
/// * `incr_updates` - 增量更新日志
pub async fn gen_all_geos_data(
    manual_refnos: Vec<RefnoEnum>,
    db_option: &DbOptionExt,
    incr_updates: Option<IncrGeoUpdateLog>,
) -> Result<GenModelResult> {
    crate::versioned_db::version_commit::ensure_live_generation_allowed(
        db_option,
        "live model generation",
    )
    .await
    .map_err(GenPipelineError::Other)?;
    gen_all_geos_data_with_read_spec(
        manual_refnos,
        db_option,
        incr_updates,
        GenerationReadSpec::live(),
    )
    .await
}

/// Opens one generation session from an explicit, immutable read contract.
///
/// Initialization callers may pass [`GenerationReadSpec::live`]. Incremental,
/// catch-up, and repair callers should pass [`GenerationReadSpec::at`]; until
/// the main-table adapter can honor `VERSION AT`, that mode fails closed in the
/// generation-read factory rather than reading latest state.
pub async fn gen_all_geos_data_with_read_spec(
    manual_refnos: Vec<RefnoEnum>,
    db_option: &DbOptionExt,
    incr_updates: Option<IncrGeoUpdateLog>,
    read_spec: GenerationReadSpec,
) -> Result<GenModelResult> {
    gen_all_geos_data_with_read_specs(manual_refnos, db_option, incr_updates, read_spec, None).await
}

/// Opens the target generation slice and, when supplied, a separate old-model
/// hierarchy slice used exclusively by incremental cleanup.
pub async fn gen_all_geos_data_with_read_specs(
    manual_refnos: Vec<RefnoEnum>,
    db_option: &DbOptionExt,
    incr_updates: Option<IncrGeoUpdateLog>,
    read_spec: GenerationReadSpec,
    cleanup_read_spec: Option<GenerationReadSpec>,
) -> Result<GenModelResult> {
    let scope_refnos = generation_scope_refnos(&manual_refnos, incr_updates.as_ref());
    let session =
        crate::generation_read::open_generation_read_session_with_spec(db_option, &read_spec)
            .await
            .map_err(anyhow::Error::new)?;
    let cleanup_hierarchy = if let Some(cleanup_read_spec) = cleanup_read_spec {
        let cleanup_session = crate::generation_read::open_generation_read_session_with_spec(
            db_option,
            &cleanup_read_spec,
        )
        .await
        .map_err(anyhow::Error::new)?;
        let hierarchy = if scope_refnos.is_empty() {
            crate::generation_read::HierarchySnapshot::load(
                Arc::clone(&cleanup_session),
                &cleanup_session.manifest().dbnums(),
            )
            .await
        } else {
            crate::generation_read::HierarchySnapshot::load_for_refnos(
                Arc::clone(&cleanup_session),
                &scope_refnos,
            )
            .await
        };
        Some(Arc::new(hierarchy.map_err(anyhow::Error::new)?))
    } else {
        None
    };
    let generation_read = GenerationReadContext::load_for_refnos(session, &scope_refnos).await?;
    gen_all_geos_data_inner(
        manual_refnos,
        db_option,
        incr_updates,
        generation_read,
        cleanup_hierarchy,
    )
    .await
}

pub async fn gen_all_geos_data_with_session(
    manual_refnos: Vec<RefnoEnum>,
    db_option: &DbOptionExt,
    incr_updates: Option<IncrGeoUpdateLog>,
    session: Arc<dyn crate::generation_read::VersionedReadSession>,
) -> Result<GenModelResult> {
    let scope_refnos = generation_scope_refnos(&manual_refnos, incr_updates.as_ref());
    let generation_read = GenerationReadContext::load_for_refnos(session, &scope_refnos).await?;
    gen_all_geos_data_inner(
        manual_refnos,
        db_option,
        incr_updates,
        generation_read,
        None,
    )
    .await
}

fn generation_scope_refnos(
    manual_refnos: &[RefnoEnum],
    incr_updates: Option<&IncrGeoUpdateLog>,
) -> Vec<RefnoEnum> {
    let mut refnos = manual_refnos.iter().copied().collect::<BTreeSet<_>>();
    if let Some(update) = incr_updates {
        refnos.extend(update.prim_refnos.iter().copied());
        refnos.extend(update.loop_owner_refnos.iter().copied());
        refnos.extend(update.bran_hanger_refnos.iter().copied());
        refnos.extend(update.basic_cata_refnos.iter().copied());
        refnos.extend(update.delete_refnos.iter().copied());
    }
    refnos.into_iter().collect()
}

#[cfg_attr(
    feature = "profile",
    tracing::instrument(skip_all, name = "gen_all_geos_data")
)]
async fn gen_all_geos_data_inner(
    manual_refnos: Vec<RefnoEnum>,
    db_option: &DbOptionExt,
    incr_updates: Option<IncrGeoUpdateLog>,
    generation_read: Arc<GenerationReadContext>,
    cleanup_hierarchy: Option<Arc<crate::generation_read::HierarchySnapshot>>,
) -> Result<GenModelResult> {
    let time = Instant::now();
    let mut perf = crate::perf_timer::PerfTimer::new("gen_all_geos_data");
    perf.mark("init");

    // specs/023 M2（D2）：每次生成 run 开始失效 pe 层级快照——增量提交后的新 run
    // 必须重新从 SurrealDB 加载最新层级（修 §0-2/§0-3 “索引永不失效”缺陷）。
    {
        let cleared = crate::versioned_db::pe_owner_snapshot::invalidate_pe_snapshots();
        if cleared > 0 {
            println!("[gen_model] pe 层级快照已失效重置: {} 个 dbnum", cleared);
        }
    }

    // 生成入口显式确保 surreal 辅助 schema 就绪（含 ses 空表兜底）。
    // 此前 ensure_surreal_init 只在 rkyv 构建等旁路被动触发：全新空库站点若
    // 首批 BRAN_TUBI tubi_relate 写入先于任何旁路调用，`dt=fn::ses_date(...)`
    // 会因 "The table 'ses' does not exist" 语句级失败直接 panic（250160 实测）。
    if db_option.use_surrealdb {
        crate::fast_model::utils::ensure_surreal_init()
            .await
            .map_err(GenPipelineError::Other)?;
    }

    cache_miss_report::init_global_cache_miss_report(db_option, "Direct");
    // 按 sesno 的增量生成只走 IncrementRun 采集后直传的 update_log。
    let final_incr_updates = incr_updates;

    let incr_count = final_incr_updates
        .as_ref()
        .map(|log| log.count())
        .unwrap_or(0);
    println!(
        "[gen_model] 启动 gen_all_geos_data: manual_refnos={}, incr_updates={}",
        manual_refnos.len(),
        incr_count,
    );

    // 增量：先失效受影响子树的 pe_transform / 内存 cache，再进 precheck/生成。
    // 避免 owner/POS 变更后 lazy miss 命中陈旧 world；禁止整库 clear。
    if db_option.use_surrealdb
        && let Some(log) = final_incr_updates.as_ref().filter(|l| l.count() > 0)
    {
        let change_roots: Vec<RefnoEnum> = log.get_all_visible_refnos().into_iter().collect();
        if !change_roots.is_empty() {
            match crate::pe_transform_refresh::invalidate_pe_transform_for_root_refnos(
                &change_roots,
            )
            .await
            {
                Ok(n) => {
                    println!(
                        "[gen_model] pe_transform 增量子树已失效: roots={} affected={}",
                        change_roots.len(),
                        n
                    );
                }
                Err(e) => {
                    log::warn!("[gen_model] pe_transform 增量子树失效失败（继续生成）: {e}");
                }
            }
        }
        let deleted: Vec<RefnoEnum> = log.delete_refnos.iter().copied().collect();
        if !deleted.is_empty() {
            match crate::pe_transform_store::clear_pe_transform_for_refnos(&deleted).await {
                Ok(n) => {
                    let _ = crate::fast_model::gen_model::transform_cache::clear_global_transform_cache_for_refnos(
                        &deleted,
                    );
                    println!("[gen_model] pe_transform 已清理删除节点: keys={}", n);
                }
                Err(e) => {
                    log::warn!("[gen_model] pe_transform 删除节点清理失败（继续生成）: {e}");
                }
            }
        }
    }

    // 性能剖析：尽量在最上层启用 tracing，覆盖 precheck -> gen_model -> mesh -> room 计算全链路。

    #[cfg(feature = "profile")]
    let _ = crate::profiling::init_chrome_tracing_for_db_option(db_option, "full_flow_room");
    perf.mark("precheck");

    // ✨ 执行预检查：确保 Tree / db_meta 就绪；pe_transform 按需 L0/L1/L2
    if db_option.use_surrealdb {
        use crate::fast_model::gen_model::precheck_coordinator::{
            PeTransformPrecheckMode, PrecheckConfig, run_precheck,
        };

        // GenPipeline 已通过 VersionedReadSession 预加载 transforms → L0。
        // L1（子树）/ L2（整库）仅在显式配置 pe_transform_mode 时使用；
        // debug-model 入口仍会在 gen 前自行 refresh_pe_transform_for_root_refnos。
        // specs/027（ADR-0007/0008）：generation_read_backend 已退役，版本读取统一走
        // VersionedReadSession（Surreal 主表直读），生成管线 pe_transform 预检查固定 L0/Skip。
        let pe_transform_mode = PeTransformPrecheckMode::Skip;
        let precheck_config = PrecheckConfig {
            enabled: true,
            check_tree: true,
            pe_transform_mode,
            pe_transform_roots: Vec::new(),
            check_db_meta: true,
            tree_output_dir: db_option
                .get_project_output_dir()
                .join("scene_tree")
                .to_string_lossy()
                .to_string(),
        };
        println!(
            "[gen_model] pe_transform precheck mode={:?} (generation read=VersionedReadSession/L0)",
            pe_transform_mode
        );
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
        // 非 Surreal 运行仍需要 refno -> dbnum 元数据。
        let _ = db_meta().ensure_loaded();
    }

    // 调试：打印 GenPipeline配置
    println!(
        "[gen_model] GenPipeline 默认管线配置: concurrency={}, batch_size={}",
        db_option.get_gen_pipeline_concurrency(),
        db_option.get_gen_pipeline_batch_size()
    );
    db_option
        .validate_model_writer_features()
        .map_err(GenPipelineError::Other)?;
    println!(
        "[gen_model] 模型写入后端: {}",
        db_option.model_writer_mode.as_str()
    );

    // ✅ SurrealDB 写入侧初始化：仅在 use_surrealdb=true 时需要。
    if db_option.use_surrealdb
        && !db_option.defer_db_write
        && db_option.model_writer_mode.writes_to_surreal()
    {
        if let Err(e) = aios_core::rs_surreal::inst::init_model_tables().await {
            eprintln!("[gen_model] ❌ 初始化 inst_relate 表结构失败: {}", e);

            // 严重错误，建议直接中断，否则后续写入必挂
            return Err(GenPipelineError::Other(e));
        }
    }

    // =========================
    // GenPipeline：新管线

    // =========================
    // 统一入口：manual/debug/incr/full 全部收敛到 GenPipeline 生成管线
    perf.mark("route_decision");
    let debug_roots = db_option.inner.get_all_debug_refnos().await;
    let incr_visible_roots: Vec<RefnoEnum> = final_incr_updates
        .as_ref()
        .map(|log| log.get_all_visible_refnos().into_iter().collect())
        .unwrap_or_default();
    let has_incr_log = final_incr_updates.is_some();
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

    perf.mark("gen_pipeline_generation");
    let result =
        process_gen_pipeline(scope, db_option, time, generation_read, cleanup_hierarchy).await;
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

/// 处理 GenPipeline的生成流程
async fn process_gen_pipeline(
    scope: GenerationScope,
    db_option: &DbOptionExt,
    time: Instant,
    generation_read: Arc<GenerationReadContext>,
    cleanup_hierarchy: Option<Arc<crate::generation_read::HierarchySnapshot>>,
) -> Result<GenModelResult> {
    let authoritative_snapshot_id = generation_read.session.manifest().authoritative_snapshot_id;
    let mut perf = crate::perf_timer::PerfTimer::new("gen_pipeline_generation");
    perf.mark("init");
    println!("[gen_model] 进入 GenPipeline 生成模式（统一管线）");
    if db_option.manual_db_nums.is_some() || db_option.exclude_db_nums.is_some() {
        println!(
            "[gen_model] 提示: GenPipeline已支持 manual_db_nums / exclude_db_nums 过滤，当前仍按配置执行"
        );
    }

    let config = GenPipelineConfig::from_db_option_ext(db_option)
        .map_err(|e| anyhow::anyhow!("配置错误: {}", e))?;
    let generation_contract = Arc::new(GenerationContract::from_db_option(db_option, &config));
    let execution_tuning = ExecutionTuning::from_db_option(db_option);
    let targets = match &scope {
        GenerationScope::Full => {
            println!("[gen_model] 当前 scope: Full");
            resolve_full_generation_targets(db_option, &generation_read.hierarchy, &config).await?
        }
        GenerationScope::Manual { roots } => {
            println!("[gen_model] 当前 scope: Manual roots={}", roots.len());
            resolve_root_generation_targets(&generation_read.hierarchy, &config, roots)?
        }
        GenerationScope::Debug { roots } => {
            println!("[gen_model] 当前 scope: Debug roots={}", roots.len());
            resolve_root_generation_targets(&generation_read.hierarchy, &config, roots)?
        }
        GenerationScope::Incremental { log } => {
            println!(
                "[gen_model] 当前 scope: Incremental visible={} deletes={}",
                log.get_all_visible_refnos().len(),
                log.delete_refnos.len()
            );
            resolve_incremental_generation_targets(&generation_read.hierarchy, &config, log)?
        }
    };
    println!(
        "[gen_model] GenerationTargets hash={} generation={} deletes={}",
        targets.target_hash(),
        targets.bran_hang_refnos().len()
            + targets.loop_refnos().len()
            + targets.cate_refnos().len()
            + targets.prim_refnos().len(),
        targets.delete_refnos().len()
    );
    let provenance = GenerationRunProvenance::new(
        generation_read.session.manifest().manifest_hash.clone(),
        generation_contract.contract_hash(),
        targets.target_hash().to_string(),
    );
    println!(
        "[gen_model] provenance manifest_hash={} contract_hash={} target_hash={}",
        provenance.input_manifest_hash(),
        provenance.contract_hash(),
        provenance.target_hash()
    );
    println!(
        "[gen_model] execution tuning noun_concurrency={} noun_batch={} channel={} base_write={} mesh_compute={} inst_aabb={} read_backend={} writer_backend={} output_root={:?} export_formats={:?} export_instances={} export_parquet={} parquet_stream={} perf_disabled={}",
        execution_tuning.noun_concurrency,
        execution_tuning.noun_batch_size,
        execution_tuning.channel_capacity,
        execution_tuning.base_write_concurrency,
        execution_tuning.mesh_compute_concurrency,
        execution_tuning.inst_aabb_write_concurrency,
        execution_tuning.read_backend,
        execution_tuning.writer_backend,
        execution_tuning.output_root,
        execution_tuning.export_formats,
        execution_tuning.export_instances,
        execution_tuning.export_parquet_after_gen,
        execution_tuning.parquet_stream_writer_enabled,
        execution_tuning.perf_report_disabled,
    );
    if !should_start_write_pipeline(&generation_contract) {
        println!(
            "[gen_model] dry-run：targets 与 provenance 已解析，跳过 cleanup、writer lifecycle、几何生成及全部后处理"
        );
        perf.mark("dry_run_complete");
        perf.end_current();
        return Ok(GenModelResult {
            success: true,
            authoritative_snapshot_id,
            artifacts: None,
            read_metrics: validated_read_metrics(&generation_read)
                .map_err(GenPipelineError::Other)?,
            provenance,
        });
    }
    let incremental_cleanup_roots = match &scope {
        GenerationScope::Incremental { .. } => targets
            .bran_hang_refnos()
            .iter()
            .chain(targets.loop_refnos())
            .chain(targets.cate_refnos())
            .chain(targets.prim_refnos())
            .chain(targets.delete_refnos())
            .copied()
            .collect::<Vec<_>>(),
        _ => Vec::new(),
    };
    let is_boolean_scoped_generation = matches!(
        &scope,
        GenerationScope::Debug { .. } | GenerationScope::Incremental { .. }
    ) || db_option
        .inner
        .debug_model_refnos
        .as_ref()
        .map(|refnos| !refnos.is_empty())
        .unwrap_or(false);
    let full_start = Instant::now();
    crate::perf_metrics::record_generate_progress("gen_pipeline_init", None, 0);
    perf.mark("categorize_and_inst_relate");

    // 1️⃣ 生成/更新 inst_relate，并获取分类后的根 refno
    let use_surrealdb = db_option.use_surrealdb;
    let defer_db_write = false;

    if matches!(&scope, GenerationScope::Incremental { .. }) && !targets.has_generation_targets() {
        println!(
            "[gen_model] 增量日志没有生成目标，进入统一空 producer/write pipeline 路径 (delete_only={})",
            targets.is_delete_only()
        );
    }

    let gen_mesh = db_option.inner.gen_mesh;
    let mesh_aabb_map: Arc<DashMap<String, parry3d::bounding_volume::Aabb>> =
        Arc::new(DashMap::new());
    let mesh_pts_map: Arc<DashMap<u64, String>> = Arc::new(DashMap::new());
    let missing_neg_carriers_for_reconcile = Arc::new(std::sync::Mutex::new(HashSet::new()));
    let artifacts = Arc::new(GenerationArtifacts::new(authoritative_snapshot_id));
    let inst_relate_precomputed = if db_option.model_writer_mode.writes_to_surreal() {
        Some(Arc::new(
            crate::fast_model::pdms_inst::InstRelatePrecomputed::from_generation_read(
                &generation_read,
            )?,
        ))
    } else {
        None
    };
    let base_model_writer = create_model_writer(
        db_option.model_writer_mode,
        Arc::clone(&mesh_aabb_map),
        missing_neg_carriers_for_reconcile,
        inst_relate_precomputed,
    );
    println!(
        "[gen_model] ModelWriter={} writes_to_surreal={} runs_downstream_pipeline={}",
        base_model_writer.name(),
        base_model_writer.writes_to_surreal(),
        base_model_writer.runs_downstream_pipeline()
    );

    let (sender, write_pipeline) = ModelWritePipeline::start(WritePipelineStart {
        db_option: db_option.clone(),
        cleanup_hierarchy,
        incremental_cleanup_roots,
        model_writer: Arc::clone(&base_model_writer),
        artifacts: Arc::clone(&artifacts),
        mesh_aabb_map,
        mesh_pts_map,
        channel_capacity: execution_tuning.channel_capacity,
        base_write_concurrency: execution_tuning.base_write_concurrency,
        mesh_compute_concurrency: execution_tuning.mesh_compute_concurrency,
        inst_aabb_write_concurrency: execution_tuning.inst_aabb_write_concurrency,
        skip_inst_relate_aabb: generation_contract.skip_inst_relate_aabb(),
        skip_final_aabb_sweep: generation_contract.skip_final_aabb_sweep(),
        use_surrealdb,
    })
    .await
    .map_err(GenPipelineError::Other)?;
    println!("⏳ [1/5] 几何体生成 (BRAN/HANG + LOOP/CATE/PRIM)...");
    crate::perf_metrics::record_generate_progress(
        "geometry_generation",
        None,
        full_start.elapsed().as_millis() as u64,
    );
    let generation_result = execute_generation_targets(
        Arc::new(db_option.clone()),
        Arc::clone(&generation_read),
        &config,
        Arc::clone(&generation_contract),
        sender.clone(),
        &targets,
        Some(Arc::clone(&artifacts)),
    )
    .await;

    println!("⏳ [2/5] write pipeline barrier...");
    crate::perf_metrics::record_generate_progress(
        "instance_data_write",
        None,
        full_start.elapsed().as_millis() as u64,
    );
    drop(sender);
    let write_result = write_pipeline.finish().await;
    let (categorized, write_report) = match (generation_result, write_result) {
        (Ok(categorized), Ok(write_report)) => (categorized, write_report),
        (Err(generation_error), Ok(_)) => {
            return Err(GenPipelineError::Other(anyhow::anyhow!(
                "GenPipeline 生成失败: {generation_error}"
            )));
        }
        (Ok(_), Err(write_error)) => return Err(GenPipelineError::Other(write_error)),
        (Err(generation_error), Err(write_error)) => {
            return Err(GenPipelineError::Other(anyhow::anyhow!(
                "GenPipeline 生成失败: {generation_error}; write pipeline 收敛失败: {write_error}"
            )));
        }
    };
    println!(
        "✅ [1/5] 几何体生成完成, 用时 {}ms",
        full_start.elapsed().as_millis()
    );
    crate::perf_metrics::record_generate_progress(
        "geometry_generation_done",
        None,
        full_start.elapsed().as_millis() as u64,
    );
    println!(
        "[gen_model] write pipeline finish: writer={} batches={} completed={} barrier_wait_ms={} mesh_cache_hit={} mesh_new={} missing_neg={}",
        write_report.writer_finish.writer_name,
        write_report.batch_count,
        write_report.completed_batches,
        write_report.barrier_wait_ms,
        write_report.mesh_cache_hits,
        write_report.mesh_new_generated,
        write_report.missing_neg_carrier_count,
    );

    if write_report.is_drain_only() {
        if let Some(stats) = &write_report.writer_finish.drain_only_stats {
            stats.print_summary();
        }
        perf.mark("drain_only_complete");
        perf.end_current();
        let artifact_summary = artifacts.summary().map_err(GenPipelineError::Other)?;
        return Ok(GenModelResult {
            success: true,
            authoritative_snapshot_id,
            artifacts: Some(artifact_summary),
            read_metrics: validated_read_metrics(&generation_read)
                .map_err(GenPipelineError::Other)?,
            provenance,
        });
    }

    crate::perf_metrics::record_generate_progress(
        "batch_barrier_done",
        Some(&format!(
            "batches={} mesh_cache_hit={} mesh_new_generated={}",
            write_report.completed_batches,
            write_report.mesh_cache_hits,
            write_report.mesh_new_generated
        )),
        full_start.elapsed().as_millis() as u64,
    );
    let insert_batch_count = write_report.batch_count;
    let mut bool_tasks = write_report.bool_tasks;
    let boolean_task_count = bool_tasks.len();
    let boolean_task_semantic_hash = super::boolean_task::semantic_hash_boolean_tasks(&bool_tasks);
    let mut boolean_execution_report: Option<BooleanBridgeReport> = None;
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
                "[gen_model] GenPipeline mesh 并行阶段完成，用时 {} ms",
                mesh_start.elapsed().as_millis()
            );
        }

        perf.mark("aabb_write");
        println!("⏳ [3/5] AABB 写入...");

        // 3️⃣ batch barrier 之后，inst_relate_aabb 已按 batch 写入完成
        if use_surrealdb {
            let skip_aabb_write = generation_contract.skip_inst_relate_aabb();
            if skip_aabb_write {
                println!(
                    "[gen_model] GenPipeline已跳过 batch inst_relate_aabb 写入（AIOS_SKIP_INST_RELATE_AABB=1）"
                );
            } else {
                println!("[gen_model] GenPipeline batch inst_relate_aabb 写入已完成");
            }
        }

        perf.mark("boolean_operation");
        println!("⏳ [4/5] 布尔运算...");
        crate::perf_metrics::record_generate_progress(
            "boolean_operation",
            None,
            full_start.elapsed().as_millis() as u64,
        );

        // 4️⃣ 可选执行布尔运算
        if db_option.inner.apply_boolean_operation {
            let bool_start = Instant::now();
            println!("[gen_model] GenPipeline开始布尔运算（boolean worker）");
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
                insert_batch_count
            );

            let report = base_model_writer
                .run_boolean_bridge(BooleanBridgeRequest {
                    mode: db_option.boolean_pipeline_mode.clone(),
                    db_option: Arc::new(db_option.inner.clone()),
                    use_surrealdb,
                    defer_db_write,
                    enable_db_backfill: db_option.enable_db_backfill,
                    scope_refnos: if is_boolean_scoped_generation {
                        all_refnos.clone()
                    } else {
                        Vec::new()
                    },
                    bool_tasks: std::mem::take(&mut bool_tasks),
                })
                .await?;
            println!(
                "[gen_model] ModelWriter boolean_bridge 完成: total={} success={} failed={} skipped={} skipped_reason={:?}",
                report.total, report.success, report.failed, report.skipped, report.skipped_reason
            );
            crate::perf_metrics::add_boolean_counters(report.success, report.failed);
            if report.failed > 0 {
                return Err(GenPipelineError::Other(anyhow::anyhow!(
                    "boolean worker failed tasks: total={} failed={}",
                    report.total,
                    report.failed
                )));
            }
            boolean_execution_report = Some(report);

            println!(
                "[gen_model] GenPipeline布尔运算完成，用时 {} ms",
                bool_start.elapsed().as_millis()
            );
        }
        perf.mark("web_bundle_export");
        println!("⏳ [5/5] 导出...");
        crate::perf_metrics::record_generate_progress(
            "web_bundle_export",
            None,
            full_start.elapsed().as_millis() as u64,
        );

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
    let boolean_execution_report =
        boolean_execution_report.unwrap_or_else(|| BooleanBridgeReport {
            total: boolean_task_count,
            skipped: boolean_task_count,
            skipped_reason: Some("boolean operation disabled"),
            ..BooleanBridgeReport::default()
        });
    artifacts
        .record_boolean_execution(
            boolean_task_count,
            boolean_task_semantic_hash,
            &boolean_execution_report,
        )
        .map_err(GenPipelineError::Other)?;

    perf.mark("sqlite_spatial_index");
    crate::perf_metrics::record_generate_progress(
        "sqlite_spatial_index",
        None,
        full_start.elapsed().as_millis() as u64,
    );
    println!(
        "[gen_model] GenPipeline全部完成，总用时 {} ms",
        full_start.elapsed().as_millis()
    );
    println!(
        "[gen_model] gen_all_geos_data 总耗时: {} ms",
        time.elapsed().as_millis()
    );
    perf.mark("instances_export");

    // ✅ 模型生成完毕后导出 instances.json（按 dbno）
    if db_option.export_instances {
        let (dbno_source, mut dbnos): (&str, Vec<u32>) =
            if let Some(nums) = db_option.inner.manual_db_nums.clone() {
                ("manual_db_nums", nums)
            } else if db_meta().ensure_loaded().is_ok() {
                // touched_dbnums 会包含 DESI 解析过程中访问到的 CATA/DICT 引用；导出面向
                // viewer 的实例数据时，只应该导出当前 module（通常 DESI）自己的数据库。
                (
                    "db_meta_module",
                    db_meta().get_dbnums_by_type(&db_option.inner.module),
                )
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
                eprintln!("[instances] GenPipeline 导出失败: {}", e);
            }
        }
    }

    #[cfg(all(not(target_arch = "wasm32"), feature = "sqlite-index"))]
    {
        if db_option.inner.enable_sqlite_rtree && db_option.inner.gen_spatial_tree {
            if db_option.export_parquet_after_gen {
                println!(
                    "[sqlite-index] 跳过生成后 SurrealDB 刷新：SQLite spatial index 将在 Parquet 导出成功后刷新"
                );
            } else {
                let mut sqlite_dbnums: Vec<u32> =
                    if let Some(nums) = db_option.inner.manual_db_nums.clone() {
                        nums
                    } else if db_meta().ensure_loaded().is_ok() {
                        db_meta().get_dbnums_by_type(&db_option.inner.module)
                    } else {
                        aios_core::query_mdb_db_nums(None, aios_core::DBType::DESI).await?
                    };
                if let Some(exclude_nums) = &db_option.inner.exclude_db_nums {
                    let exclude: HashSet<u32> = exclude_nums.iter().copied().collect();
                    sqlite_dbnums.retain(|dbnum| !exclude.contains(dbnum));
                }
                sqlite_dbnums.sort_unstable();
                sqlite_dbnums.dedup();

                if sqlite_dbnums.is_empty() {
                    println!("[sqlite-index] 跳过刷新：本轮未解析到可用 dbnum");
                } else {
                    println!(
                        "[sqlite-index] 模型生成后刷新空间索引并聚合中间节点 AABB: dbnums={:?}",
                        sqlite_dbnums
                    );
                    match crate::fast_model::room_model::refresh_sqlite_spatial_index_from_inst_relate_aabb(
                        Some(&sqlite_dbnums),
                        None,
                    )
                    .await
                    {
                        Ok(count) if count > 0 => {
                            println!("[sqlite-index] 空间索引刷新完成: inserted={count}")
                        }
                        Ok(_) => {
                            return Err(anyhow::anyhow!(
                                "gen_spatial_tree 已启用，但空间索引刷新结果为空: dbnums={sqlite_dbnums:?}"
                            )
                            .into());
                        }
                        Err(err) => {
                            return Err(anyhow::anyhow!(
                                "gen_spatial_tree 已启用，但空间索引刷新失败: {err:#}"
                            )
                            .into());
                        }
                    }
                }
            }
        } else if db_option.inner.enable_sqlite_rtree {
            println!("[sqlite-index] 跳过刷新：gen_spatial_tree 未启用");
        }
    }

    perf.end_current();
    crate::perf_metrics::record_generate_progress(
        "gen_pipeline_finished",
        None,
        full_start.elapsed().as_millis() as u64,
    );

    // spec 004：生成阶段分段耗时（直接来自 PerfTimer 分段记录）。
    {
        let stage_ms: Vec<(String, u64)> = perf
            .stages()
            .iter()
            .filter_map(|s| {
                s.ended_at.map(|end| {
                    (
                        s.name.clone(),
                        end.duration_since(s.started_at).as_millis() as u64,
                    )
                })
            })
            .collect();
        crate::perf_metrics::record_generate_stage_ms(&stage_ms);
    }

    // 输出性能摘要到控制台
    perf.print_summary();
    let artifact_summary = artifacts.summary().map_err(GenPipelineError::Other)?;
    let read_metrics = validated_read_metrics(&generation_read).map_err(GenPipelineError::Other)?;

    // 保存性能报告为 JSON 和 CSV（可通过 AIOS_DISABLE_PERF_REPORT=1 禁用）
    let perf_report_disabled = execution_tuning.perf_report_disabled;

    if !perf_report_disabled {
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
        let enabled_nouns = db_option.gen_pipeline_enabled_target_types.clone();
        let metadata = serde_json::json!({
            "mode": "index_tree",
            "project_name": project_name,
            "dbnum": dbnum_tag,
            "enabled_nouns": enabled_nouns,
            "use_surrealdb": db_option.use_surrealdb,
            "model_cache_write": true,
            "apply_boolean": db_option.inner.apply_boolean_operation,
            "gen_mesh": db_option.inner.gen_mesh,
            "concurrency": db_option.get_gen_pipeline_concurrency(),
            "batch_size": db_option.get_gen_pipeline_batch_size(),
            "generation_read_backend": generation_read.session.backend_kind().as_str(),
            "authoritative_snapshot_id": authoritative_snapshot_id,
            "generation_read_metrics": read_metrics.clone(),
            "geometry_artifact_hash": artifact_summary.geometry_artifact_hash.clone(),
            "generation_artifacts_semantic_hash": artifact_summary.semantic_hash.clone(),
            "final_model_semantic_hash": artifact_summary.model_semantic_hash.clone(),
            "input_manifest_hash": provenance.input_manifest_hash(),
            "generation_contract_hash": provenance.contract_hash(),
            "generation_target_hash": provenance.target_hash(),
        });
        let json_path = profile_dir.join(format!(
            "perf_gen_model_gen_pipeline_dbnum_{}_{}.json",
            dbnum_tag, timestamp
        ));
        let csv_path = profile_dir.join(format!(
            "perf_gen_model_gen_pipeline_dbnum_{}_{}.csv",
            dbnum_tag, timestamp
        ));
        if let Err(e) = perf.save_json(&json_path, metadata.clone()) {
            eprintln!("[perf] 保存 JSON 报告失败: {}", e);
        }

        if let Err(e) = perf.save_csv(&csv_path, metadata) {
            eprintln!("[perf] 保存 CSV 报告失败: {}", e);
        }
    }

    println!(
        "[gen_model] GenerationArtifacts snapshot={} batches={} mesh_results={} geometry_hash={} semantic_hash={}",
        artifact_summary.authoritative_snapshot_id,
        artifact_summary.batch_count,
        artifact_summary.mesh_result_count,
        artifact_summary.geometry_artifact_hash,
        artifact_summary.semantic_hash
    );
    Ok(GenModelResult {
        success: true,
        authoritative_snapshot_id,
        artifacts: Some(artifact_summary),
        read_metrics,
        provenance,
    })
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

    #[test]
    fn generation_scope_priority_is_stable() {
        let manual = RefnoEnum::from("1/1");
        let debug = RefnoEnum::from("1/2");
        let incremental = RefnoEnum::from("1/3");
        let mut log = IncrGeoUpdateLog::default();
        log.prim_refnos.insert(incremental);

        assert!(matches!(
            decide_generation_scope(&[], &[], false, &[], None),
            GenerationScope::Full
        ));
        assert!(matches!(
            decide_generation_scope(&[manual], &[], false, &[], None),
            GenerationScope::Manual { roots } if roots == vec![manual]
        ));
        assert!(matches!(
            decide_generation_scope(&[], &[debug], false, &[], None),
            GenerationScope::Debug { roots } if roots == vec![debug]
        ));
        assert!(matches!(
            decide_generation_scope(&[], &[], true, &[incremental], Some(&log)),
            GenerationScope::Incremental { .. }
        ));

        let mixed = decide_generation_scope(&[manual], &[debug], true, &[incremental], Some(&log));
        let GenerationScope::Manual { roots } = mixed else {
            panic!("mixed scope must normalize to Manual roots");
        };
        let roots: HashSet<_> = roots.into_iter().collect();
        assert_eq!(roots, HashSet::from([manual, debug, incremental]));
    }

    #[test]
    fn delete_only_incremental_scope_does_not_fall_back_to_full() {
        let mut log = IncrGeoUpdateLog::default();
        log.delete_refnos.insert(RefnoEnum::from("1/9"));

        let scope = decide_generation_scope(&[], &[], true, &[], Some(&log));
        assert!(matches!(scope, GenerationScope::Incremental { .. }));
    }

    #[test]
    fn empty_incremental_scope_does_not_fall_back_to_full() {
        let log = IncrGeoUpdateLog::default();
        let scope = decide_generation_scope(&[], &[], true, &[], Some(&log));
        assert!(matches!(scope, GenerationScope::Incremental { .. }));
    }

    #[test]
    fn dry_run_never_starts_write_pipeline() {
        let mut opt = DbOptionExt::from(aios_core::options::DbOption::default());
        opt.gen_model_dry_run = true;
        let config = GenPipelineConfig::from_db_option_ext(&opt).expect("config");
        let contract = GenerationContract::from_db_option(&opt, &config);
        assert!(!should_start_write_pipeline(&contract));
    }
}
