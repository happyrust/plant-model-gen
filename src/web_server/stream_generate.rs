//! 流式模型生成模块
//!
//! 实现增量模型生成 API，通过 SSE 推送生成进度。

use aios_core::{RefU64, RefnoEnum};
use axum::{
    extract::{Json, State},
    response::sse::{Event, KeepAlive, Sse},
};
use futures::stream::{self, Stream, StreamExt};
use serde::{Deserialize, Serialize};
use std::convert::Infallible;
use std::str::FromStr;
use std::time::Instant;
use tracing::{error, info, warn};

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
    #[serde(default = "default_max_depth")]
    pub max_depth: u32,
}

fn default_true() -> bool {
    true
}

fn default_batch_size() -> usize {
    50
}

fn default_max_depth() -> u32 {
    10
}

/// SSE 事件类型
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
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
        generated_refnos: Vec<String>,
        progress: f32,
    },
    /// 批次生成失败
    BatchFailed {
        batch_index: usize,
        error: String,
    },
    /// 全部完成
    Finished {
        total_generated: usize,
        total_skipped: usize,
        duration_ms: u64,
    },
    /// 错误
    Error {
        message: String,
    },
}

// ============================================================================
// 核心查询函数
// ============================================================================

/// 查询可见子节点（使用 aios_core 现有函数）
///
/// 从根节点开始，递归查询所有可见的后代节点
pub async fn query_visible_descendants(
    root_refnos: &[RefnoEnum],
    _max_depth: u32,
) -> anyhow::Result<Vec<RefnoEnum>> {
    if root_refnos.is_empty() {
        return Ok(Vec::new());
    }

    let mut all_descendants = Vec::new();

    for root_refno in root_refnos {
        // 使用 aios_core 的现有函数获取子元素
        match aios_core::get_children_refnos(*root_refno).await {
            Ok(children) => {
                all_descendants.extend(children);
            }
            Err(e) => {
                warn!(
                    "[StreamGenerate] 获取 {} 的子节点失败: {}",
                    root_refno, e
                );
            }
        }
    }

    info!(
        "[StreamGenerate] 展开 {} 个根节点 -> {} 个子节点",
        root_refnos.len(),
        all_descendants.len()
    );

    Ok(all_descendants)
}

/// 检查哪些 refno 还没有生成模型
///
/// 通过查询 inst_relate 表判断
pub async fn filter_missing_inst_relate(
    refnos: &[RefnoEnum],
) -> anyhow::Result<Vec<RefnoEnum>> {
    if refnos.is_empty() {
        return Ok(Vec::new());
    }

    // 使用 aios_core 的 query_insts 检查哪些已经有数据
    let existing = aios_core::query_insts(refnos, false).await.unwrap_or_default();
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
                                warn!("[StreamGenerate] 展开子节点失败: {}", e);
                                parsed_refnos.clone()
                            }
                        }
                    } else {
                        parsed_refnos.clone()
                    };

                    // 3. 过滤已生成的
                    let to_generate = if req.force_regenerate {
                        expanded.clone()
                    } else {
                        filter_missing_inst_relate(&expanded)
                            .await
                            .unwrap_or_else(|_| expanded.clone())
                    };

                    let skipped_count = expanded.len().saturating_sub(to_generate.len());
                    let event = StreamGenerateEvent::ExpandComplete {
                        original_count: parsed_refnos.len(),
                        expanded_count: expanded.len(),
                        skipped_count,
                    };

                    if to_generate.is_empty() {
                        // 全部已生成，直接完成
                        Some((
                            event,
                            (
                                req.clone(),
                                StreamGenerateState::Finishing {
                                    total_generated: 0,
                                    total_skipped: expanded.len(),
                                    start_time: Instant::now(),
                                },
                            ),
                        ))
                    } else {
                        Some((
                            event,
                            (
                                req.clone(),
                                StreamGenerateState::Generating {
                                    to_generate,
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
                    to_generate,
                    batch_index,
                    total_generated,
                    total_skipped,
                    start_time,
                } => {
                    // 4. 分批生成
                    let batch_size = req.batch_size;
                    let total_batches = (to_generate.len() + batch_size - 1) / batch_size;

                    if batch_index >= total_batches {
                        // 全部批次完成
                        Some((
                            StreamGenerateEvent::Finished {
                                total_generated,
                                total_skipped,
                                duration_ms: start_time.elapsed().as_millis() as u64,
                            },
                            (req.clone(), StreamGenerateState::Done),
                        ))
                    } else {
                        let start_idx = batch_index * batch_size;
                        let end_idx = (start_idx + batch_size).min(to_generate.len());
                        let batch = to_generate[start_idx..end_idx].to_vec();
                        let batch_len = batch.len();

                        // 调用模型生成
                        let db_option = aios_core::get_db_option();
                        let db_option_ext = crate::options::DbOptionExt::from(db_option.clone());

                        match crate::fast_model::gen_all_geos_data(
                            batch.clone(),
                            &db_option_ext,
                            None,
                            None,
                        )
                        .await
                        {
                            Ok(_) => {
                                let generated_refnos: Vec<String> =
                                    batch.iter().map(|r| r.to_string()).collect();
                                let progress =
                                    ((batch_index + 1) as f32 / total_batches as f32) * 100.0;

                                Some((
                                    StreamGenerateEvent::BatchComplete {
                                        batch_index,
                                        batch_count: total_batches,
                                        generated_refnos,
                                        progress,
                                    },
                                    (
                                        req.clone(),
                                        StreamGenerateState::Generating {
                                            to_generate: to_generate.clone(),
                                            batch_index: batch_index + 1,
                                            total_generated: total_generated + batch_len,
                                            total_skipped,
                                            start_time,
                                        },
                                    ),
                                ))
                            }
                            Err(e) => {
                                error!("[StreamGenerate] 批次 {} 生成失败: {}", batch_index, e);
                                Some((
                                    StreamGenerateEvent::BatchFailed {
                                        batch_index,
                                        error: e.to_string(),
                                    },
                                    (
                                        req.clone(),
                                        StreamGenerateState::Generating {
                                            to_generate: to_generate.clone(),
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

/// 流式生成状态机
enum StreamGenerateState {
    Init,
    Expanding {
        parsed_refnos: Vec<RefnoEnum>,
    },
    Generating {
        to_generate: Vec<RefnoEnum>,
        batch_index: usize,
        total_generated: usize,
        total_skipped: usize,
        start_time: Instant,
    },
    Finishing {
        total_generated: usize,
        total_skipped: usize,
        start_time: Instant,
    },
    Done,
}
