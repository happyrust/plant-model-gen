//! 流式模型生成模块
//!
//! 实现增量模型生成 API，通过 SSE 推送生成进度。

use aios_core::{RefU64, RefnoEnum, SurrealQueryExt, project_primary_db};
use axum::{
    extract::{Json, Path, Query, State},
    response::sse::{Event, KeepAlive, Sse},
};
use futures::stream::{self, Stream, StreamExt};
use serde::{Deserialize, Serialize};
use std::convert::Infallible;
use std::str::FromStr;
use std::sync::Arc;
use std::time::Instant;
use surrealdb::types::SurrealValue;
use tracing::{error, info, warn};

use crate::data_interface::db_meta_manager::db_meta;
use crate::fast_model::gen_model::tree_index_manager::TreeIndexManager;

use super::AppState;

// ============================================================================
// 请求/响应类型
// ============================================================================

/// 流式生成请求
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StreamGenerateRequest {
    /// 根节点 refno 列表（会展开所有子节点）
    pub refnos: Vec<String>,
    /// 是否展开子节点（默认 true）
    #[serde(default = "default_true")]
    pub expand_children: bool,
    /// 是否强制重新生成（默认 false）
    #[serde(default)]
    pub force_regenerate: bool,
    /// 每批处理的 refno 数量（默认 50）
    #[serde(default = "default_batch_size")]
    pub batch_size: usize,
    /// 最大展开深度（默认 10）
    /// - `0` 表示不限深度（会一直向下遍历到叶子；内部仍有安全上限避免 OOM）
    #[serde(default = "default_max_depth")]
    pub max_depth: u32,
    /// 是否执行布尔运算（孔洞/负实体结果）（默认 false）
    ///
    /// - 该开关只影响“是否尝试产出 inst_relate_bool 的结果网格”
    /// - 不影响基础 inst_relate/geo_relate 的生成
    #[serde(default)]
    pub apply_boolean: bool,

    /// 是否在生成完成后导出 instances_{dbno}.json（默认 false）
    ///
    /// - 用于前端按需加载：生成完 mesh 后把实例清单增量写入 output/instances
    #[serde(default)]
    pub export_instances: bool,

    /// 导出 instances 时是否“合并追加”到既有 instances_{dbno}.json（默认 true）
    ///
    /// - 仅当 export_instances=true 时生效
    #[serde(default = "default_true")]
    pub merge_instances: bool,
}

fn default_true() -> bool {
    true
}

fn default_batch_size() -> usize {
    50
}

fn default_max_depth() -> u32 {
    0
}

/// SSE 事件类型
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(
    tag = "type",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum StreamGenerateEvent {
    /// 开始处理
    Started {
        total_refnos: usize,
        message: String,
    },
    /// 展开子节点完成
    ExpandComplete {
        original_count: usize,
        expanded_count: usize,
        skipped_count: usize,
    },
    /// 批次生成完成
    BatchComplete {
        batch_index: usize,
        batch_count: usize,
        /// 本批次内实际触发“生成”的 refno（可能为空）
        generated_refnos: Vec<String>,
        /// 本批次内已存在数据、无需生成的 refno（可能为空）
        skipped_refnos: Vec<String>,
        progress: f32,
        completed_count: usize,
        total_count: usize,
        #[serde(skip_serializing_if = "Option::is_none")]
        current_refno: Option<String>,
        /// 可选警告（例如布尔运算失败时仍继续推进，以便前端至少能加载基础模型）
        #[serde(skip_serializing_if = "Option::is_none")]
        warning: Option<String>,
    },
    /// 批次生成失败
    BatchFailed {
        batch_index: usize,
        /// 本批次内已存在数据、仍可加载的 refno（即使生成失败也可返回）
        skipped_refnos: Vec<String>,
        error: String,
    },
    /// 全部完成
    Finished {
        total_generated: usize,
        total_skipped: usize,
        duration_ms: u64,
    },
    /// 导出 instances 开始
    ExportInstancesStarted { message: String },
    /// 导出 instances 完成
    ExportInstancesFinished { dbnos: Vec<u32>, duration_ms: u64 },
    /// 错误
    Error { message: String },
}

// ============================================================================
// 核心查询函数
// ============================================================================

/// 查询可见子节点（使用 aios_core 现有函数）
///
/// 从根节点开始，递归查询所有可见的后代节点
pub async fn query_visible_descendants(
    root_refnos: &[RefnoEnum],
    max_depth: u32,
) -> anyhow::Result<Vec<RefnoEnum>> {
    if root_refnos.is_empty() {
        return Ok(Vec::new());
    }

    const MAX_NODES: usize = 200_000;
    const MAX_DEPTH_SAFETY: usize = 5_000;

    let mut visited: std::collections::HashSet<RefnoEnum> = std::collections::HashSet::new();
    let mut frontier: Vec<RefnoEnum> = root_refnos.to_vec();
    for r in &frontier {
        visited.insert(*r);
    }

    let mut out: Vec<RefnoEnum> = Vec::new();
    let depth_limit: Option<usize> = if max_depth == 0 {
        None
    } else {
        Some(max_depth as usize)
    };
    let mut depth: usize = 0;

    loop {
        if frontier.is_empty() {
            break;
        }
        if let Some(limit) = depth_limit {
            if depth >= limit {
                break;
            }
        }
        if depth >= MAX_DEPTH_SAFETY {
            anyhow::bail!(
                "descendant traversal exceeded safety depth limit: {}",
                MAX_DEPTH_SAFETY
            );
        }
        if visited.len() >= MAX_NODES {
            anyhow::bail!(
                "descendant traversal exceeded safety node limit: {}",
                MAX_NODES
            );
        }

        let mut next: Vec<RefnoEnum> = Vec::new();
        for node in &frontier {
            match aios_core::get_children_refnos(*node).await {
                Ok(children) => {
                    for child in children {
                        if visited.insert(child) {
                            next.push(child);
                            out.push(child);
                            if visited.len() >= MAX_NODES {
                                anyhow::bail!(
                                    "descendant traversal exceeded safety node limit: {}",
                                    MAX_NODES
                                );
                            }
                        }
                    }
                }
                Err(e) => {
                    warn!("[StreamGenerate] 获取 {} 的子节点失败: {}", node, e);
                }
            }
        }
        frontier = next;
        depth += 1;
    }

    info!(
        "[StreamGenerate] 展开 {} 个根节点 (max_depth={}) -> {} 个子孙节点",
        root_refnos.len(),
        max_depth,
        out.len()
    );

    Ok(out)
}

async fn filter_geo_refnos(refnos: &[RefnoEnum]) -> anyhow::Result<Vec<RefnoEnum>> {
    if refnos.is_empty() {
        return Ok(Vec::new());
    }

    // 这里的“有几何体”定义与 scene_tree 一致：按 noun 分类判断 has_geo。
    // 为了避免 N+1，这里批量从 pe 表查询 noun。
    const CHUNK: usize = 500;
    let mut out: Vec<RefnoEnum> = Vec::new();

    for chunk in refnos.chunks(CHUNK) {
        let id_list = chunk
            .iter()
            .map(|r| r.to_pe_key())
            .collect::<Vec<_>>()
            .join(",");

        let sql = format!("SELECT id as refno, noun FROM [{id_list}]");
        let rows: Vec<PeNounRow> = project_primary_db()
            .query_take(&sql, 0)
            .await
            .unwrap_or_default();
        for row in rows {
            if row.noun.is_empty() {
                continue;
            }
            if !crate::scene_tree::is_geo_noun(&row.noun) {
                continue;
            }
            out.push(row.refno);
        }
    }

    Ok(out)
}

#[derive(Debug, Deserialize, SurrealValue)]
struct PeNounRow {
    refno: RefnoEnum,
    noun: String,
}

/// 检查哪些 refno 还没有生成模型
///
/// 通过查询 inst_relate 表判断
pub async fn filter_missing_inst_relate(refnos: &[RefnoEnum]) -> anyhow::Result<Vec<RefnoEnum>> {
    if refnos.is_empty() {
        return Ok(Vec::new());
    }

    // 使用 aios_core 的 query_insts 检查哪些已经有数据
    let existing = aios_core::query_insts(refnos, false)
        .await
        .unwrap_or_default();
    let existing_set: std::collections::HashSet<RefnoEnum> =
        existing.into_iter().map(|inst| inst.refno).collect();

    let missing: Vec<RefnoEnum> = refnos
        .iter()
        .filter(|r| !existing_set.contains(r))
        .cloned()
        .collect();

    info!(
        "[StreamGenerate] 过滤: {} 个 refno -> {} 个需要生成",
        refnos.len(),
        missing.len()
    );

    Ok(missing)
}

// ============================================================================
// SSE 流式生成 API
// ============================================================================

/// POST /api/model/stream-generate
///
/// 流式增量生成模型，通过 SSE 推送进度
pub async fn api_stream_generate(
    State(_state): State<AppState>,
    Json(req): Json<StreamGenerateRequest>,
) -> Sse<impl Stream<Item = Result<Event, Infallible>>> {
    // 将请求处理逻辑封装到异步流中
    let event_stream = stream::unfold(
        (req, StreamGenerateState::Init),
        |(req, state)| async move {
            match state {
                StreamGenerateState::Init => {
                    // 1. 解析 refno
                    let mut parsed_refnos = Vec::new();
                    for refno_str in &req.refnos {
                        if let Ok(r) = RefnoEnum::from_str(refno_str) {
                            parsed_refnos.push(r);
                        } else if let Ok(num) = refno_str.parse::<u64>() {
                            parsed_refnos.push(RefnoEnum::Refno(RefU64(num)));
                        }
                    }

                    if parsed_refnos.is_empty() {
                        let event = StreamGenerateEvent::Error {
                            message: "没有有效的 refno".to_string(),
                        };
                        return Some((event, (req, StreamGenerateState::Done)));
                    }

                    let event = StreamGenerateEvent::Started {
                        total_refnos: parsed_refnos.len(),
                        message: format!("开始处理 {} 个 refno", parsed_refnos.len()),
                    };

                    Some((
                        event,
                        (
                            req.clone(),
                            StreamGenerateState::Expanding { parsed_refnos },
                        ),
                    ))
                }

                StreamGenerateState::Expanding { parsed_refnos } => {
                    // 2. 展开子节点
                    let expanded = if req.expand_children {
                        match query_visible_descendants(&parsed_refnos, req.max_depth).await {
                            Ok(mut descendants) => {
                                // 合并根节点和后代节点
                                descendants.extend(parsed_refnos.clone());
                                descendants
                            }
                            Err(e) => {
                                let event = StreamGenerateEvent::Error {
                                    message: format!("展开子节点失败: {e}"),
                                };
                                return Some((event, (req, StreamGenerateState::Done)));
                            }
                        }
                    } else {
                        parsed_refnos.clone()
                    };

                    // 2.1 过滤出“有几何体”的节点（has_geo=true）
                    let expanded_geo = match filter_geo_refnos(&expanded).await {
                        Ok(v) => v,
                        Err(e) => {
                            let event = StreamGenerateEvent::Error {
                                message: format!("过滤几何节点失败: {e}"),
                            };
                            return Some((event, (req, StreamGenerateState::Done)));
                        }
                    };

                    // 2.2 若启用布尔运算：提前生成“深度负实体”依赖，避免后续布尔阶段缺少切割体网格
                    if req.apply_boolean {
                        let mut neg_deps: Vec<RefnoEnum> = Vec::new();
                        for &root in &parsed_refnos {
                            match aios_core::query_deep_neg_inst_refnos(root).await {
                                Ok(mut v) => neg_deps.append(&mut v),
                                Err(e) => {
                                    warn!(
                                        "[StreamGenerate] 查询负实体依赖失败 root={} err={}",
                                        root, e
                                    );
                                }
                            }
                        }
                        neg_deps.sort();
                        neg_deps.dedup();

                        // 避免重复生成：剔除本就包含在 expanded_geo 内的项
                        let expanded_geo_set: std::collections::HashSet<RefnoEnum> =
                            expanded_geo.iter().copied().collect();
                        neg_deps.retain(|r| !expanded_geo_set.contains(r));

                        if !neg_deps.is_empty() {
                            let neg_missing = if req.force_regenerate {
                                neg_deps.clone()
                            } else {
                                filter_missing_inst_relate(&neg_deps)
                                    .await
                                    .unwrap_or_else(|_| neg_deps.clone())
                            };

                            if !neg_missing.is_empty() {
                                let db_option = aios_core::get_db_option();
                                let db_option_ext = crate::options::DbOptionExt::from(db_option.clone());
                                if let Err(e) = crate::fast_model::gen_all_geos_data(
                                    neg_missing,
                                    &db_option_ext,
                                    None,
                                    None,
                                )
                                .await
                                {
                                    warn!("[StreamGenerate] 负实体依赖预生成失败: {}", e);
                                }
                            }
                        }
                    }

                    // 3. 预过滤：计算“需要生成”的集合（用于后续按批次切分）
                    let to_generate = if req.force_regenerate {
                        expanded_geo.clone()
                    } else {
                        filter_missing_inst_relate(&expanded_geo)
                            .await
                            .unwrap_or_else(|_| expanded_geo.clone())
                    };

                    let skipped_count = expanded_geo.len().saturating_sub(to_generate.len());
                    let event = StreamGenerateEvent::ExpandComplete {
                        original_count: parsed_refnos.len(),
                        expanded_count: expanded_geo.len(),
                        skipped_count,
                    };

                    // 不管是否需要生成，都进入按 expanded 进行批次处理：
                    // - 这样 skipped 的 refno 也能“边加载”
                    if expanded_geo.is_empty() {
                        // 全部已生成，直接完成
                        Some((
                            event,
                            (
                                req.clone(),
                                StreamGenerateState::Finishing {
                                    total_generated: 0,
                                    total_skipped: 0,
                                    start_time: Instant::now(),
                                },
                            ),
                        ))
                    } else {
                        let missing_set: std::collections::HashSet<RefnoEnum> =
                            to_generate.iter().cloned().collect();
                        Some((
                            event,
                            (
                                req.clone(),
                                StreamGenerateState::Generating {
                                    root_refnos: parsed_refnos.clone(),
                                    expanded: expanded_geo,
                                    missing_set,
                                    batch_index: 0,
                                    total_generated: 0,
                                    total_skipped: skipped_count,
                                    start_time: Instant::now(),
                                },
                            ),
                        ))
                    }
                }

                StreamGenerateState::Generating {
                    root_refnos,
                    expanded,
                    missing_set,
                    batch_index,
                    total_generated,
                    total_skipped,
                    start_time,
                } => {
                    // 4. 分批生成
                    let batch_size = req.batch_size;
                    let total_batches = (expanded.len() + batch_size - 1) / batch_size;

                    if batch_index >= total_batches {
                        // 全部批次完成
                        Some((
                            StreamGenerateEvent::Finished {
                                total_generated,
                                total_skipped,
                                duration_ms: start_time.elapsed().as_millis() as u64,
                            },
                            if req.export_instances {
                                (
                                    req.clone(),
                                    StreamGenerateState::ExportingInstancesStart {
                                        root_refnos: root_refnos.clone(),
                                    },
                                )
                            } else {
                                (req.clone(), StreamGenerateState::Done)
                            },
                        ))
                    } else {
                        let start_idx = batch_index * batch_size;
                        let end_idx = (start_idx + batch_size).min(expanded.len());
                        let batch_all = expanded[start_idx..end_idx].to_vec();

                        let mut to_generate_in_batch: Vec<RefnoEnum> = Vec::new();
                        let mut skipped_in_batch: Vec<RefnoEnum> = Vec::new();
                        if req.force_regenerate {
                            to_generate_in_batch = batch_all.clone();
                        } else {
                            for r in &batch_all {
                                if missing_set.contains(r) {
                                    to_generate_in_batch.push(*r);
                                } else {
                                    skipped_in_batch.push(*r);
                                }
                            }
                        }

                        // 调用模型生成（本批次可能为空：全部 skipped）
                        let db_option = aios_core::get_db_option();
                        let db_option_ext = crate::options::DbOptionExt::from(db_option.clone());

                        if to_generate_in_batch.is_empty() {
                            let progress =
                                ((batch_index + 1) as f32 / total_batches as f32) * 100.0;
                            let skipped_refnos: Vec<String> =
                                skipped_in_batch.iter().map(|r| r.to_string()).collect();
                            Some((
                                StreamGenerateEvent::BatchComplete {
                                    batch_index,
                                    batch_count: total_batches,
                                    generated_refnos: Vec::new(),
                                    skipped_refnos,
                                    progress,
                                    completed_count: end_idx,
                                    total_count: expanded.len(),
                                    current_refno: batch_all.last().map(|r| r.to_string()),
                                    warning: None,
                                },
                                (
                                    req.clone(),
                                    StreamGenerateState::Generating {
                                        root_refnos: root_refnos.clone(),
                                        expanded: expanded.clone(),
                                        missing_set: missing_set.clone(),
                                        batch_index: batch_index + 1,
                                        total_generated,
                                        total_skipped,
                                        start_time,
                                    },
                                ),
                            ))
                        } else {
                            match crate::fast_model::gen_all_geos_data(
                                to_generate_in_batch.clone(),
                                &db_option_ext,
                                None,
                                None,
                            )
                            .await
                            {
                                Ok(_) => {
                                    // 生成 mesh（GLB 强制输出）——保证前端可直接拉取 /files/meshes/lod_L1/{geo_hash}_L1.glb
                                    let replace_exist = req.force_regenerate || db_option.is_replace_mesh();
                                    let meshes_dir = db_option.get_meshes_path();
                                    let precision = Arc::new(db_option.mesh_precision().clone());
                                    if let Err(e) = crate::fast_model::mesh_generate::gen_inst_meshes(
                                        &meshes_dir,
                                        &precision,
                                        &batch_all,
                                        replace_exist,
                                        &[crate::options::MeshFormat::PdmsMesh],
                                    )
                                    .await
                                    {
                                        warn!(
                                            "[StreamGenerate] 批次 {} 生成 mesh 失败(继续推进): {}",
                                            batch_index, e
                                        );
                                    }

                                    // 可选：执行布尔运算（在本批次生成完成之后、发 BatchComplete 之前）
                                    // 目标选择：batch_all（包含 skipped），这样能为“已有 inst 但缺少 bool 结果”的节点补齐孔洞结果。
                                    let mut warning: Option<String> = None;
                                    if req.apply_boolean {
                                        // 是否在配置层启用布尔运算（即使请求要求，也尊重 DbOption 开关）
                                        let apply_by_config = db_option.apply_boolean_operation;
                                        if apply_by_config {
                                            // 布尔运算依赖：必须先有 inst_geo 的 mesh + aabb.d（否则 query_aabb_params 会返回空）
                                            // 这里按需补齐 mesh，避免依赖 DbOption.gen_mesh（SSE 端点应“边生成边加载”）。
                                            let replace_exist =
                                                req.force_regenerate || db_option.is_replace_mesh();
                                            let meshes_dir = db_option.get_meshes_path();
                                            let precision =
                                                Arc::new(db_option.mesh_precision().clone());
                                            if let Err(e) =
                                                crate::fast_model::mesh_generate::gen_inst_meshes(
                                                    &meshes_dir,
                                                    &precision,
                                                    &batch_all,
                                                    replace_exist,
                                                    &[crate::options::MeshFormat::PdmsMesh],
                                                )
                                                .await
                                            {
                                                warn!(
                                                    "[StreamGenerate] 批次 {} 生成 mesh 失败(继续推进): {}",
                                                    batch_index, e
                                                );
                                            }

                                            if let Err(e) =
                                                crate::fast_model::mesh_generate::booleans_meshes_in_db(
                                                    Some(std::sync::Arc::new(db_option.clone())),
                                                    &batch_all,
                                                )
                                                .await
                                            {
                                                warn!(
                                                    "[StreamGenerate] 批次 {} 布尔运算失败(继续推进): {}",
                                                    batch_index, e
                                                );
                                                warning = Some(format!("布尔运算失败(本批次仍继续): {e:#}"));
                                            }
                                        }
                                    }

                                    let generated_refnos: Vec<String> = to_generate_in_batch
                                        .iter()
                                        .map(|r| r.to_string())
                                        .collect();
                                    let skipped_refnos: Vec<String> =
                                        skipped_in_batch.iter().map(|r| r.to_string()).collect();
                                    let progress =
                                        ((batch_index + 1) as f32 / total_batches as f32) * 100.0;

                                    Some((
                                        StreamGenerateEvent::BatchComplete {
                                            batch_index,
                                            batch_count: total_batches,
                                            generated_refnos,
                                            skipped_refnos,
                                            progress,
                                            completed_count: end_idx,
                                            total_count: expanded.len(),
                                            current_refno: batch_all.last().map(|r| r.to_string()),
                                            warning,
                                        },
                                        (
                                            req.clone(),
                                            StreamGenerateState::Generating {
                                                root_refnos: root_refnos.clone(),
                                                expanded: expanded.clone(),
                                                missing_set: missing_set.clone(),
                                                batch_index: batch_index + 1,
                                                total_generated: total_generated + to_generate_in_batch.len(),
                                                total_skipped,
                                                start_time,
                                            },
                                        ),
                                    ))
                                }
                                Err(e) => {
                                    error!("[StreamGenerate] 批次 {} 生成失败: {}", batch_index, e);
                                    let skipped_refnos: Vec<String> =
                                        skipped_in_batch.iter().map(|r| r.to_string()).collect();
                                    Some((
                                        StreamGenerateEvent::BatchFailed {
                                            batch_index,
                                            skipped_refnos,
                                            error: e.to_string(),
                                        },
                                        (
                                            req.clone(),
                                            StreamGenerateState::Generating {
                                                root_refnos: root_refnos.clone(),
                                                expanded: expanded.clone(),
                                                missing_set: missing_set.clone(),
                                                batch_index: batch_index + 1,
                                                total_generated,
                                                total_skipped,
                                                start_time,
                                            },
                                        ),
                                    ))
                                }
                            }
                        }
                    }
                }

                StreamGenerateState::ExportingInstancesStart { root_refnos } => {
                    Some((
                        StreamGenerateEvent::ExportInstancesStarted {
                            message: "开始导出并合并 instances_{dbno}.json...".to_string(),
                        },
                        (
                            req.clone(),
                            StreamGenerateState::ExportingInstancesRun {
                                root_refnos,
                                export_start: Instant::now(),
                            },
                        ),
                    ))
                }

                StreamGenerateState::ExportingInstancesRun {
                    root_refnos,
                    export_start,
                } => {
                    let db_option = aios_core::get_db_option();
                    let mesh_dir = db_option.get_meshes_path();

                    // 注意：refno 的第一段是 ref0，不是 dbno/dbnum；不能用字符串 split 推导。
                    // 这里必须通过 db_meta 或 tree_index 映射得到 dbnum。
                    let _ = db_meta().ensure_loaded();
                    let mut dbnos: Vec<u32> = Vec::new();
                    for r in &root_refnos {
                        if let Some(dbnum) = db_meta().get_dbnum_by_refno(*r) {
                            dbnos.push(dbnum);
                            continue;
                        }
                        match TreeIndexManager::resolve_dbnum_for_refno(*r) {
                            Ok(dbnum) => dbnos.push(dbnum),
                            Err(e) => {
                                warn!("[StreamGenerate] 无法解析 dbnum: refno={}, err={}", r, e);
                            }
                        }
                    }
                    dbnos.sort();
                    dbnos.dedup();

                    let export_result = if req.merge_instances {
                        crate::fast_model::export_model::export_prepack_lod::export_instances_json_for_refnos_grouped_by_dbno_merge(
                            &root_refnos,
                            &mesh_dir,
                            std::path::Path::new("output"),
                            Arc::new(db_option.clone()),
                            false,
                        )
                        .await
                    } else {
                        crate::fast_model::export_model::export_prepack_lod::export_instances_json_for_refnos_grouped_by_dbno(
                            &root_refnos,
                            &mesh_dir,
                            std::path::Path::new("output"),
                            Arc::new(db_option.clone()),
                            false,
                        )
                        .await
                    };

                    match export_result {
                        Ok(_) => Some((
                            StreamGenerateEvent::ExportInstancesFinished {
                                dbnos,
                                duration_ms: export_start.elapsed().as_millis() as u64,
                            },
                            (req.clone(), StreamGenerateState::Done),
                        )),
                        Err(e) => Some((
                            StreamGenerateEvent::Error {
                                message: format!("导出 instances 失败: {e}"),
                            },
                            (req.clone(), StreamGenerateState::Done),
                        )),
                    }
                }

                StreamGenerateState::Finishing {
                    total_generated,
                    total_skipped,
                    start_time,
                } => Some((
                    StreamGenerateEvent::Finished {
                        total_generated,
                        total_skipped,
                        duration_ms: start_time.elapsed().as_millis() as u64,
                    },
                    (req.clone(), StreamGenerateState::Done),
                )),

                StreamGenerateState::Done => None,
            }
        },
    )
    .map(|event| {
        let json = serde_json::to_string(&event).unwrap_or_else(|_| "{}".to_string());
        Ok::<_, Infallible>(Event::default().data(json).event("message"))
    });

    Sse::new(event_stream).keep_alive(KeepAlive::default())
}

#[derive(Debug, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct StreamGenerateQuery {
    #[serde(default = "default_true")]
    pub expand_children: bool,
    #[serde(default)]
    pub force_regenerate: bool,
    #[serde(default = "default_batch_size")]
    pub batch_size: usize,
    #[serde(default = "default_max_depth")]
    pub max_depth: u32,
    #[serde(default)]
    pub apply_boolean: bool,
    #[serde(default)]
    pub export_instances: bool,
    #[serde(default = "default_true")]
    pub merge_instances: bool,
}

/// GET /api/model/stream-generate-by-root/{refno}
///
/// 兼容浏览器 `EventSource`（GET-only），用于“选择某节点时按需生成其子孙并合并 instances_{dbno}.json”。
pub async fn api_stream_generate_by_root(
    State(state): State<AppState>,
    Path(refno): Path<String>,
    Query(q): Query<StreamGenerateQuery>,
) -> Sse<impl Stream<Item = Result<Event, Infallible>>> {
    let req = StreamGenerateRequest {
        refnos: vec![refno],
        expand_children: q.expand_children,
        force_regenerate: q.force_regenerate,
        batch_size: q.batch_size,
        max_depth: q.max_depth,
        apply_boolean: q.apply_boolean,
        export_instances: q.export_instances,
        merge_instances: q.merge_instances,
    };
    api_stream_generate(State(state), Json(req)).await
}

/// 流式生成状态机
enum StreamGenerateState {
    Init,
    Expanding {
        parsed_refnos: Vec<RefnoEnum>,
    },
    Generating {
        root_refnos: Vec<RefnoEnum>,
        expanded: Vec<RefnoEnum>,
        missing_set: std::collections::HashSet<RefnoEnum>,
        batch_index: usize,
        total_generated: usize,
        total_skipped: usize,
        start_time: Instant,
    },
    ExportingInstancesStart {
        root_refnos: Vec<RefnoEnum>,
    },
    ExportingInstancesRun {
        root_refnos: Vec<RefnoEnum>,
        export_start: Instant,
    },
    Finishing {
        total_generated: usize,
        total_skipped: usize,
        start_time: Instant,
    },
    Done,
}
