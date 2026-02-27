//! MBD 管道标注 API（首期：管道分支 BRAN/HANG）
//!
//! 目标：为 plant3d-web 提供“管道 MBD 标注”所需的结构化数据（段/尺寸/焊缝/坡度）。
//! 说明：本接口采用“后端提供语义点位 + 前端做屏幕布局/避让”的分层方式，便于渐进式对齐 MBD(PML)。

use std::collections::{HashMap, HashSet};
use std::path::{Path as FsPath, PathBuf};
use std::sync::Mutex;

use once_cell::sync::Lazy;

use aios_core::RefnoEnum;
use axum::{
    Router,
    extract::{Path, Query},
    http::{HeaderValue, header::CONTENT_TYPE},
    response::{IntoResponse, Response},
    routing::get,
    Json,
};
use glam::Vec3;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum MbdPipeSource {
    Db,
    Cache,
    Parquet,
}

impl Default for MbdPipeSource {
    fn default() -> Self {
        Self::Db
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct MbdPipeQuery {
    /// 数据来源：parquet=Parquet 文件（默认），db=SurrealDB，cache=model cache
    pub source: MbdPipeSource,
    /// dbno（可选；若不传则尝试从 output/scene_tree/db_meta_info.json 推导）
    pub dbno: Option<u32>,
    /// model instance_cache 的 batch_id（可选；若不传则默认按 latest）
    pub batch_id: Option<String>,
    /// 调试开关：返回 debug_info（包含实际使用的 cache/dbnum/batch 等）
    pub debug: bool,
    /// 严格 dbno：若传入 dbno 但该 dbno 无 batch，则不进行跨库回退探测
    pub strict_dbno: bool,
    /// 最小坡度（0.001 对齐 MBD 默认）
    pub min_slope: f32,
    /// 最大坡度（0.1 对齐 MBD 默认）
    pub max_slope: f32,
    /// 最小尺寸长度（mm）
    pub dim_min_length: f32,
    /// 是否额外输出“焊口链式尺寸”（包含两端）到 dims 数组（kind=chain）
    pub include_chain_dims: bool,
    /// 是否额外输出“总长尺寸”（kind=overall）到 dims 数组
    pub include_overall_dim: bool,
    /// 是否额外输出“端口间距尺寸”（优先用 arrive_axis_pt/leave_axis_pt；kind=port）到 dims 数组
    pub include_port_dims: bool,
    /// 焊缝合并阈值（mm）：相邻段端口距离小于该值则认为是焊缝
    pub weld_merge_threshold: f32,
    pub include_dims: bool,
    pub include_welds: bool,
    pub include_slopes: bool,
    /// 是否尝试填充分支属性（失败则忽略，不影响 success）
    pub include_branch_attrs: bool,
    /// 是否尝试用 TreeIndex 的 noun 辅助推断 weld_type（默认关闭，避免额外依赖/误判）
    pub include_weld_nouns: bool,
}

impl Default for MbdPipeQuery {
    fn default() -> Self {
        Self {
            source: MbdPipeSource::Db,
            dbno: None,
            batch_id: None,
            debug: false,
            strict_dbno: false,
            min_slope: 0.001,
            max_slope: 0.1,
            dim_min_length: 1.0,
            include_chain_dims: false,
            include_overall_dim: false,
            include_port_dims: false,
            weld_merge_threshold: 1.0,
            include_dims: true,
            include_welds: true,
            include_slopes: true,
            include_branch_attrs: true,
            include_weld_nouns: false,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct MbdPipeResponse {
    pub success: bool,
    pub error_message: Option<String>,
    pub data: Option<MbdPipeData>,
}

#[derive(Debug, Clone, Serialize)]
pub struct MbdPipeData {
    pub input_refno: String,
    pub branch_refno: String,
    pub branch_name: String,
    pub branch_attrs: BranchAttrsDto,
    pub segments: Vec<MbdPipeSegmentDto>,
    pub dims: Vec<MbdDimDto>,
    pub welds: Vec<MbdWeldDto>,
    pub slopes: Vec<MbdSlopeDto>,
    pub stats: MbdPipeStats,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub debug_info: Option<MbdPipeDebugInfo>,
}

#[derive(Debug, Clone, Serialize, Default)]
pub struct MbdPipeStats {
    pub segments_count: usize,
    pub dims_count: usize,
    pub welds_count: usize,
    pub slopes_count: usize,
}

/// 分支属性（对齐 MBD/markpipe/branAttlist.txt 的 BranAttarr）
#[derive(Debug, Clone, Serialize, Default)]
pub struct BranchAttrsDto {
    pub duty: Option<String>,
    pub pspec: Option<String>,
    pub rccm: Option<String>,
    pub clean: Option<String>,
    pub temp: Option<String>,
    pub pressure: Option<f32>,
    pub ispec: Option<String>,
    pub insuthick: Option<f32>,
    pub tspec: Option<String>,
    pub swgd: Option<String>,
    pub drawnum: Option<String>,
    pub rev: Option<String>,
    pub status: Option<String>,
    pub fluid: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct MbdPipeSegmentDto {
    pub id: String,
    pub refno: String,
    pub noun: String,
    pub name: Option<String>,
    pub arrive: Option<[f32; 3]>,
    pub leave: Option<[f32; 3]>,
    pub length: f32,
    pub straight_length: f32,
    pub outside_diameter: Option<f32>,
    pub bore: Option<f32>,
}

#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum MbdDimKind {
    /// 每段长度（tubi 段 start/end）
    Segment,
    /// 焊口链式尺寸（包含两端）
    Chain,
    /// 总长（累计长度）
    Overall,
    /// 端口间距（优先轴线点 arrive_axis/leave_axis）
    Port,
}

#[derive(Debug, Clone, Serialize)]
pub struct MbdDimDto {
    pub id: String,
    pub kind: MbdDimKind,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub group_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub seq: Option<u32>,
    pub start: [f32; 3],
    pub end: [f32; 3],
    pub length: f32,
    pub text: String,
}

#[derive(Debug, Clone, Copy, Serialize)]
pub enum MbdWeldType {
    Butt = 0,
    Fillet = 1,
    Socket = 2,
}

#[derive(Debug, Clone, Serialize)]
pub struct MbdWeldDto {
    pub id: String,
    pub position: [f32; 3],
    pub weld_type: MbdWeldType,
    /// true=车间焊（A），false=现场焊（M）
    pub is_shop: bool,
    pub label: String,
    pub left_refno: String,
    pub right_refno: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct MbdSlopeDto {
    pub id: String,
    pub start: [f32; 3],
    pub end: [f32; 3],
    /// 坡度（dz / horizontal_dist），保留符号
    pub slope: f32,
    pub text: String,
}

#[derive(Debug, Clone, Serialize, Default)]
pub struct MbdPipeDebugInfo {
    pub cache_dir: Option<String>,
    pub requested_dbno: Option<u32>,
    pub inferred_dbnum: Option<u32>,
    pub active_dbnum: Option<u32>,
    pub requested_batch_id: Option<String>,
    pub batches_all: Vec<String>,
    pub batches_used: Vec<String>,
    pub fallback_used: bool,
    pub fallback_reason: Option<String>,
    pub notes: Vec<String>,
}

pub fn create_mbd_pipe_routes() -> Router {
    Router::new().route("/api/mbd/pipe/{refno}", get(get_mbd_pipe))
}

fn json_utf8<T: Serialize>(value: T) -> Response {
    let mut res = Json(value).into_response();
    res.headers_mut().insert(
        CONTENT_TYPE,
        HeaderValue::from_static("application/json; charset=utf-8"),
    );
    res
}

/// 尝试修复“UTF-8 被当作 Latin1 解码后又按 UTF-8 输出”的常见乱码（如：`æ°` → `新`）。
///
/// 说明：此问题通常源于上游数据采集/入库链路。这里做“只读修复”，便于前端调试与对齐。
fn fix_mojibake_utf8_latin1(s: String) -> String {
    if s.is_empty() {
        return s;
    }
    // 只有当字符串完全落在 0x00..=0xFF 时，才可能是这类 mojibake（例如 "æ°"）。
    if !s.chars().all(|c| (c as u32) <= 0xFF) {
        return s;
    }

    let high_cnt = s
        .chars()
        .filter(|c| {
            let u = *c as u32;
            (0x80..=0xFF).contains(&u)
        })
        .count();
    if high_cnt < 2 {
        return s;
    }

    let bytes: Vec<u8> = s.chars().map(|c| c as u8).collect();
    match String::from_utf8(bytes) {
        Ok(fixed) => {
            let has_cjk = fixed.chars().any(|c| ('\u{4E00}'..='\u{9FFF}').contains(&c));
            if has_cjk {
                fixed
            } else {
                s
            }
        }
        Err(_) => s,
    }
}

#[allow(dead_code)]
#[derive(Debug, Clone)]
struct CacheTubiSeg {
    /// tubi 段 refno（约定：使用 leave_refno 作为段标识）
    refno: RefnoEnum,
    /// tubi_relate 的 out（到达元件 refno）
    arrive_refno: Option<RefnoEnum>,
    /// 连通顺序（tubi_relate 的 id[1] / PdmsTubing.index）
    order: Option<u32>,
    /// 段起点（与 cache 一致：tubi_start_pt）
    start: Vec3,
    /// 段终点（与 cache 一致：tubi_end_pt）
    end: Vec3,
    /// arrive 端口轴线点（可选；来自 EleGeosInfo.arrive_axis_pt）
    arrive_axis: Option<Vec3>,
    /// leave 端口轴线点（可选；来自 EleGeosInfo.leave_axis_pt）
    leave_axis: Option<Vec3>,
}

#[inline]
fn segment_port_points(seg: &CacheTubiSeg) -> (Vec3, Vec3) {
    // 口径对齐既有 cache 实现：
    // - leave_axis 对应 seg.start 一侧
    // - arrive_axis 对应 seg.end 一侧
    let start = seg.leave_axis.unwrap_or(seg.start);
    let end = seg.arrive_axis.unwrap_or(seg.end);
    (start, end)
}

#[inline]
fn format_dim_length_text_mm(length: f32) -> String {
    // 约定：后端输出稳定的“纯数字”文本；单位/语义由前端按 kind 展示。
    // - 避免 NaN/inf 传播到前端
    // - 避免 "-0"（浮点格式化的边界情况）
    if !length.is_finite() {
        return "0".to_string();
    }
    let s = format!("{:.0}", length);
    if s == "-0" { "0".to_string() } else { s }
}

async fn get_mbd_pipe(
    Path(refno): Path<String>,
    Query(query): Query<MbdPipeQuery>,
) -> impl IntoResponse {
    let input_refno_enum = match refno.parse::<RefnoEnum>() {
        Ok(v) => v,
        Err(e) => {
            return json_utf8(MbdPipeResponse {
                success: false,
                error_message: Some(format!("无效的 refno: {e}")),
                data: None,
            });
        }
    };

    // cache-only 约定：当前接口以“输入即 BRAN/HANG refno”为前提，不回退 SurrealDB 做祖先解析。
    // plant3d-web 的测试路由与面板逻辑也是以分支 refno 为输入。
    let branch_refno = input_refno_enum.clone();

    let (segments, mut debug_info) = match query.source {
        MbdPipeSource::Parquet => match fetch_tubi_segments_from_parquet_with_debug(
            branch_refno.clone(),
            query.dbno,
        )
        .await
        {
            Ok(v) => v,
            Err(parquet_err) => {
                // Parquet 失败 → 自动 fallback 到 SurrealDB
                match fetch_tubi_segments_from_surreal_with_debug(branch_refno.clone()).await {
                    Ok((segs, mut db_debug)) => {
                        db_debug.fallback_used = true;
                        db_debug.fallback_reason = Some(format!(
                            "parquet 失败({parquet_err})，已自动回退到 SurrealDB"
                        ));
                        db_debug.notes.push("auto-fallback: parquet→db".into());

                        // 后台异步导出 parquet（不阻塞当前请求）
                        // DB 路径不会设置 inferred_dbnum，需要主动推导
                        let dbno_for_export = query.dbno.or_else(|| {
                            use crate::data_interface::db_meta_manager::db_meta;
                            let _ = db_meta().ensure_loaded();
                            let d = db_meta().get_dbnum_by_refno(branch_refno.clone()).unwrap_or(0);
                            if d > 0 { Some(d) } else { None }
                        });
                        if let Some(dbnum) = dbno_for_export {
                            tokio::spawn(async move {
                                if let Err(e) = trigger_async_parquet_export(dbnum).await {
                                    eprintln!("[mbd-pipe] 后台 parquet 导出失败: {e}");
                                }
                            });
                            db_debug.notes.push(format!(
                                "已触发后台 parquet 导出 dbnum={dbnum}"
                            ));
                        }

                        (segs, db_debug)
                    }
                    Err(db_err) => {
                        return json_utf8(MbdPipeResponse {
                            success: false,
                            error_message: Some(format!(
                                "Parquet 失败({parquet_err})，SurrealDB 也失败({db_err})"
                            )),
                            data: None,
                        });
                    }
                }
            }
        },
        MbdPipeSource::Db => match fetch_tubi_segments_from_surreal_with_debug(branch_refno.clone()).await {
            Ok(v) => v,
            Err(e) => {
                return json_utf8(MbdPipeResponse {
                    success: false,
                    error_message: Some(format!(
                        "从 SurrealDB 读取分支管段失败: {e}（可尝试 ?source=cache 走 model cache）"
                    )),
                    data: None,
                });
            }
        },
        MbdPipeSource::Cache => match fetch_tubi_segments_from_cache_with_debug(
            branch_refno.clone(),
            query.dbno,
            query.batch_id.as_deref(),
            query.strict_dbno,
        )
        .await
        {
            Ok(v) => v,
            Err(e) => {
                return json_utf8(MbdPipeResponse {
                    success: false,
                    error_message: Some(format!("从 model cache 读取分支管段失败: {e}")),
                    data: None,
                });
            }
        },
    };

    if matches!(query.source, MbdPipeSource::Db) {
        if query.dbno.is_some() || query.batch_id.is_some() || query.strict_dbno {
            debug_info.notes.push(format!(
                "db 模式已忽略 dbno={:?} batch_id={:?} strict_dbno={}",
                query.dbno, query.batch_id, query.strict_dbno
            ));
        }
    }

    if matches!(query.source, MbdPipeSource::Parquet) {
        if query.batch_id.is_some() || query.strict_dbno {
            debug_info.notes.push(format!(
                "parquet 模式已忽略 batch_id={:?} strict_dbno={}",
                query.batch_id, query.strict_dbno
            ));
        }
    }

    let (branch_name, branch_attrs) = if query.include_branch_attrs {
        match try_fill_branch_name_and_attrs(branch_refno).await {
            Ok(v) => v,
            Err(e) => {
                debug_info.notes.push(format!("分支属性填充失败（已忽略）: {e}"));
                (branch_refno.to_string(), BranchAttrsDto::default())
            }
        }
    } else {
        (branch_refno.to_string(), BranchAttrsDto::default())
    };

    let mut out_segments: Vec<MbdPipeSegmentDto> = Vec::with_capacity(segments.len());
    for (i, seg) in segments.iter().enumerate() {
        out_segments.push(MbdPipeSegmentDto {
            id: format!("seg:{}:{i}", seg.refno),
            refno: seg.refno.to_string(),
            // cache-only：目前仅能稳定提供 tubi 段的几何与连通顺序；noun/规格等语义字段后续再补齐
            noun: "TUBI".to_string(),
            name: None,
            arrive: Some([seg.start.x, seg.start.y, seg.start.z]),
            leave: Some([seg.end.x, seg.end.y, seg.end.z]),
            length: seg.start.distance(seg.end),
            straight_length: seg.start.distance(seg.end),
            outside_diameter: None,
            bore: None,
        });
    }

    // ===== dims / welds / slopes =====
    let mut dims: Vec<MbdDimDto> = Vec::new();
    if query.include_dims {
        for (i, seg) in segments.iter().enumerate() {
            let length = seg.start.distance(seg.end);
            if length < query.dim_min_length {
                continue;
            }
            dims.push(MbdDimDto {
                id: format!("dim:{}:{i}", seg.refno),
                kind: MbdDimKind::Segment,
                group_id: None,
                seq: Some(i as u32),
                start: [seg.start.x, seg.start.y, seg.start.z],
                end: [seg.end.x, seg.end.y, seg.end.z],
                length,
                text: format_dim_length_text_mm(length),
            });
        }
    }

    if query.include_port_dims {
        for (i, seg) in segments.iter().enumerate() {
            let (start, end) = segment_port_points(seg);
            let length = start.distance(end);
            if length < query.dim_min_length {
                continue;
            }
            dims.push(MbdDimDto {
                id: format!("dim:port:{}:{i}", seg.refno),
                kind: MbdDimKind::Port,
                group_id: None,
                seq: Some(i as u32),
                start: [start.x, start.y, start.z],
                end: [end.x, end.y, end.z],
                length,
                text: format_dim_length_text_mm(length),
            });
        }
    }

    let mut welds: Vec<MbdWeldDto> = Vec::new();
    let mut weld_joints: Vec<WeldJoint> = Vec::new();

    if query.include_welds || query.include_chain_dims || query.include_overall_dim {
        let mut shop_idx = 0usize;
        let mut field_idx = 0usize;

        // 可选：用 TreeIndex 查询 noun，辅助推断 weld_type（仅当输出 welds 时才有意义）。
        let noun_lookup = if query.include_welds && query.include_weld_nouns {
            match try_build_tree_index_for_refno(branch_refno).await {
                Ok(v) => Some(v),
                Err(e) => {
                    debug_info
                        .notes
                        .push(format!("TreeIndex 初始化失败（weld_nouns 已忽略）: {e}"));
                    None
                }
            }
        } else {
            None
        };

        for i in 0..segments.len().saturating_sub(1) {
            let seg1 = &segments[i];
            let seg2 = &segments[i + 1];

            let (p1, p2, dist) = closest_endpoints(seg1.start, seg1.end, seg2.start, seg2.end);
            if dist >= query.weld_merge_threshold {
                continue;
            }

            weld_joints.push(WeldJoint {
                left_endpoint: p1,
                right_endpoint: p2,
                mid: midpoint(p1, p2),
            });

            if !query.include_welds {
                continue;
            }

            let mut noun1: Option<&str> = Some("TUBI");
            let mut noun2: Option<&str> = Some("TUBI");
            let mut _noun_s1_owned: Option<String> = None;
            let mut _noun_s2_owned: Option<String> = None;
            if let Some(lookup) = noun_lookup.as_ref() {
                // 说明：instance_cache 仅稳定提供 tubi_arrive_refno（段起点关联元件）。
                // 此处仅做渐进式增强：若能查到 arrive_refno 的 noun，则用于辅助分类（可能不完全准确）。
                if let Some(r1) = seg1.arrive_refno {
                    if let Some(n) = lookup.get_noun(r1) {
                        _noun_s1_owned = Some(n);
                        noun1 = _noun_s1_owned.as_deref();
                    }
                }
                if let Some(r2) = seg2.arrive_refno {
                    if let Some(n) = lookup.get_noun(r2) {
                        _noun_s2_owned = Some(n);
                        noun2 = _noun_s2_owned.as_deref();
                    }
                }
            }
            let weld_type = determine_weld_type(noun1, noun2);

            // 首期：简单近似 MBD shop/field 规则：分支两端优先现场焊；中间按“常见预制件”判断是否车间焊。
            let at_ends = i == 0 || (i + 1) == (segments.len().saturating_sub(1));
            let shop_candidate = false;
            let is_shop = !at_ends && shop_candidate;

            let label = if is_shop {
                shop_idx += 1;
                format!("A{shop_idx}")
            } else {
                field_idx += 1;
                format!("M{field_idx}")
            };

            welds.push(MbdWeldDto {
                id: format!("weld:{}:{i}", branch_refno),
                position: midpoint(p1, p2).to_array(),
                weld_type,
                is_shop,
                label,
                left_refno: seg1.refno.to_string(),
                right_refno: seg2.refno.to_string(),
            });
        }
    }

    if query.include_chain_dims {
        let ends: Vec<(Vec3, Vec3)> = segments.iter().map(|s| (s.start, s.end)).collect();
        let chain_pts = build_chain_points_from_ends(&ends, &weld_joints);

        let group_id = Some(format!("chain:{}", branch_refno));
        for i in 0..chain_pts.len().saturating_sub(1) {
            let a = chain_pts[i];
            let b = chain_pts[i + 1];
            let length = a.distance(b);
            if length < query.dim_min_length {
                continue;
            }
            dims.push(MbdDimDto {
                id: format!("dim:chain:{}:{i}", branch_refno),
                kind: MbdDimKind::Chain,
                group_id: group_id.clone(),
                seq: Some(i as u32),
                start: [a.x, a.y, a.z],
                end: [b.x, b.y, b.z],
                length,
                text: format_dim_length_text_mm(length),
            });
        }
    }

    if query.include_overall_dim {
        let mut total = 0.0f32;
        for seg in &segments {
            total += seg.start.distance(seg.end);
        }

        if !segments.is_empty() {
            // 挂点：用链式端点选择逻辑（若无 weld_joints，则退化为首段 start/end）。
            let ends: Vec<(Vec3, Vec3)> = segments.iter().map(|s| (s.start, s.end)).collect();
            let chain_pts = build_chain_points_from_ends(&ends, &weld_joints);
            let (a, b) = if chain_pts.len() >= 2 {
                (chain_pts[0], chain_pts[chain_pts.len() - 1])
            } else {
                (segments[0].start, segments[0].end)
            };

            if total >= query.dim_min_length {
                dims.push(MbdDimDto {
                    id: format!("dim:overall:{}", branch_refno),
                    kind: MbdDimKind::Overall,
                    group_id: Some(format!("overall:{}", branch_refno)),
                    seq: None,
                    start: [a.x, a.y, a.z],
                    end: [b.x, b.y, b.z],
                    length: total,
                    // overall 需要显式语义，避免与 segment/chain 的“纯数字”混淆（3D label 侧更明显）。
                    text: format!("TOTAL {}", format_dim_length_text_mm(total)),
                });
            }
        }
    }

    let mut slopes: Vec<MbdSlopeDto> = Vec::new();
    if query.include_slopes {
        for (i, seg) in segments.iter().enumerate() {
            let dx = seg.end.x - seg.start.x;
            let dy = seg.end.y - seg.start.y;
            let dz = seg.end.z - seg.start.z;
            let horizontal = (dx * dx + dy * dy).sqrt();
            if horizontal <= 1e-3 {
                continue;
            }
            let slope = dz / horizontal;
            let abs_slope = slope.abs();
            if abs_slope < query.min_slope || abs_slope > query.max_slope {
                continue;
            }
            // 与 MBD 文本形式保持一致：slope xx.x%
            let text = format!("slope {:.1}%", abs_slope * 100.0);
            slopes.push(MbdSlopeDto {
                id: format!("slope:{}:{i}", seg.refno),
                start: [seg.start.x, seg.start.y, seg.start.z],
                end: [seg.end.x, seg.end.y, seg.end.z],
                slope,
                text,
            });
        }
    }

    let stats = MbdPipeStats {
        segments_count: out_segments.len(),
        dims_count: dims.len(),
        welds_count: welds.len(),
        slopes_count: slopes.len(),
    };

    if query.debug {
        debug_info.inferred_dbnum = debug_info.inferred_dbnum.or(query.dbno);
        debug_info.requested_dbno = query.dbno;
        debug_info.requested_batch_id = query.batch_id.clone();
        debug_info.notes.push(format!(
            "stats: segs={} dims={} welds={} slopes={}",
            stats.segments_count, stats.dims_count, stats.welds_count, stats.slopes_count
        ));
    }

    json_utf8(MbdPipeResponse {
        success: true,
        error_message: None,
        data: Some(MbdPipeData {
            input_refno: input_refno_enum.to_string(),
            branch_refno: branch_refno.to_string(),
            branch_name,
            branch_attrs,
            segments: out_segments,
            dims,
            welds,
            slopes,
            stats,
            debug_info: query.debug.then_some(debug_info),
        }),
    })
}

/// 正在后台导出的 dbnum 集合（防重复并发触发）
static EXPORTING_DBNUMS: Lazy<Mutex<HashSet<u32>>> = Lazy::new(|| Mutex::new(HashSet::new()));

/// 后台异步触发 parquet 导出（不阻塞当前请求）
async fn trigger_async_parquet_export(dbnum: u32) -> anyhow::Result<()> {
    use crate::fast_model::export_model::export_dbnum_instances_parquet::export_dbnum_instances_parquet;
    use std::sync::Arc;

    // 防重复：如果已在导出中则跳过
    {
        let mut set = EXPORTING_DBNUMS.lock().unwrap();
        if set.contains(&dbnum) {
            println!("[mbd-pipe] dbnum={dbnum} 已在后台导出中，跳过");
            return Ok(());
        }
        set.insert(dbnum);
    }

    let result = async {
        let db_option = Arc::new(aios_core::get_db_option().clone());
        let project_name = &db_option.project_name;

        let output_dir = if project_name.is_empty() {
            PathBuf::from("output/instances")
        } else {
            PathBuf::from(format!("output/{project_name}/instances"))
        };

        println!(
            "[mbd-pipe] 后台导出 parquet: dbnum={dbnum} → {}",
            output_dir.display()
        );

        let stats = export_dbnum_instances_parquet(
            dbnum,
            &output_dir,
            db_option,
            false, // verbose
            None,  // target_unit (默认 mm)
            None,  // root_refno (全量)
        )
        .await?;

        println!(
            "[mbd-pipe] 后台导出完成: dbnum={dbnum} instances={} tubings={} ({} bytes, {:?})",
            stats.instance_count, stats.tubing_count, stats.total_bytes, stats.elapsed
        );

        Ok::<(), anyhow::Error>(())
    }
    .await;

    // 导出完成（无论成功失败），移除标记
    {
        let mut set = EXPORTING_DBNUMS.lock().unwrap();
        set.remove(&dbnum);
    }

    result
}

async fn fetch_tubi_segments_from_parquet_with_debug(
    branch_refno: RefnoEnum,
    dbno: Option<u32>,
) -> anyhow::Result<(Vec<CacheTubiSeg>, MbdPipeDebugInfo)> {
    use crate::data_interface::db_meta_manager::db_meta;
    use polars::prelude::*;

    let mut debug = MbdPipeDebugInfo::default();
    debug.notes.push("source=parquet".to_string());
    debug.requested_dbno = dbno;

    let inferred_dbnum = if let Some(d) = dbno {
        d
    } else {
        db_meta().ensure_loaded()?;
        db_meta().get_dbnum_by_refno(branch_refno).unwrap_or(0)
    };
    if inferred_dbnum == 0 {
        anyhow::bail!("无法推导 dbno（请传 dbno 或先生成 output/scene_tree/db_meta_info.json）");
    }
    debug.inferred_dbnum = Some(inferred_dbnum);

    // 确定 parquet 输出目录：优先 output/{project}/instances，回退 output/{project}/parquet
    let db_option = aios_core::get_db_option();
    let project_name = &db_option.project_name;
    let instances_dir = if project_name.is_empty() {
        PathBuf::from("output/instances")
    } else {
        let d1 = PathBuf::from(format!("output/{project_name}/instances"));
        if d1.exists() {
            d1
        } else {
            let d2 = PathBuf::from(format!("output/{project_name}/parquet"));
            if d2.exists() { d2 } else { d1 }
        }
    };
    debug.cache_dir = Some(instances_dir.display().to_string());

    let tubings_path = instances_dir.join(format!("tubings_{inferred_dbnum}.parquet"));
    if !tubings_path.exists() {
        anyhow::bail!(
            "tubings parquet 文件不存在: {}",
            tubings_path.display()
        );
    }
    let transforms_path = instances_dir.join("transforms.parquet");

    // 读取 tubings parquet，按 owner_refno_str 过滤
    let owner_refno_str = branch_refno.to_string();
    let tubings_df = {
        let file = std::fs::File::open(&tubings_path)?;
        let full_df = ParquetReader::new(file).finish()?;
        let mask = full_df.column("owner_refno_str")?.str()?.into_iter()
            .map(|opt| opt.map_or(false, |v| v == owner_refno_str))
            .collect::<BooleanChunked>();
        let filtered = full_df.filter(&mask)?;
        filtered.sort(["order"], Default::default())?
    };

    if tubings_df.height() == 0 {
        anyhow::bail!(
            "tubings parquet 中无 owner_refno_str={} 的记录（file={}）",
            owner_refno_str,
            tubings_path.display()
        );
    }
    debug.notes.push(format!("tubings rows={}", tubings_df.height()));

    // 收集需要的 trans_hash 值
    let trans_hashes: Vec<String> = tubings_df
        .column("trans_hash")?
        .str()?
        .into_no_null_iter()
        .map(|s| s.to_string())
        .collect();

    // 读取 transforms parquet，按需过滤
    let trans_map: HashMap<String, glam::Mat4> = if transforms_path.exists() {
        let file = std::fs::File::open(&transforms_path)?;
        let full_trans_df = ParquetReader::new(file).finish()?;
        let hash_set: std::collections::HashSet<&str> = trans_hashes.iter().map(|s| s.as_str()).collect();
        let mask = full_trans_df.column("trans_hash")?.str()?.into_iter()
            .map(|opt| opt.map_or(false, |v| hash_set.contains(v)))
            .collect::<BooleanChunked>();
        let trans_df = full_trans_df.filter(&mask)?;
        let mut m: HashMap<String, glam::Mat4> = HashMap::new();
        for i in 0..trans_df.height() {
            let hash = trans_df.column("trans_hash")?.str()?.get(i).unwrap_or_default().to_string();
            let get_f = |name: &str| -> f32 {
                trans_df.column(name).ok()
                    .and_then(|c| c.f64().ok())
                    .and_then(|ca| ca.get(i))
                    .unwrap_or(0.0) as f32
            };
            let mat = glam::Mat4::from_cols(
                glam::Vec4::new(get_f("m00"), get_f("m10"), get_f("m20"), get_f("m30")),
                glam::Vec4::new(get_f("m01"), get_f("m11"), get_f("m21"), get_f("m31")),
                glam::Vec4::new(get_f("m02"), get_f("m12"), get_f("m22"), get_f("m32")),
                glam::Vec4::new(get_f("m03"), get_f("m13"), get_f("m23"), get_f("m33")),
            );
            m.insert(hash, mat);
        }
        debug.notes.push(format!("transforms loaded={}", m.len()));
        m
    } else {
        debug.notes.push("transforms.parquet 不存在，使用单位矩阵".to_string());
        HashMap::new()
    };

    // 构建 CacheTubiSeg
    let tubi_refno_col = tubings_df.column("tubi_refno_str")?.str()?;
    let order_col = tubings_df.column("order")?.u32()?;
    let trans_hash_col = tubings_df.column("trans_hash")?.str()?;

    let mut segs: Vec<CacheTubiSeg> = Vec::with_capacity(tubings_df.height());
    for i in 0..tubings_df.height() {
        let tubi_refno_s = tubi_refno_col.get(i).unwrap_or_default();
        let order = order_col.get(i);
        let th = trans_hash_col.get(i).unwrap_or_default();

        let mat = trans_map.get(th).copied().unwrap_or(glam::Mat4::IDENTITY);
        let start = mat.transform_point3(Vec3::new(0.0, 0.0, 0.0));
        let end = mat.transform_point3(Vec3::new(0.0, 0.0, 1.0));

        segs.push(CacheTubiSeg {
            refno: RefnoEnum::from(tubi_refno_s),
            arrive_refno: None,
            order,
            start,
            end,
            arrive_axis: None,
            leave_axis: None,
        });
    }

    segs.sort_by(|a, b| {
        let ao = a.order.unwrap_or(u32::MAX);
        let bo = b.order.unwrap_or(u32::MAX);
        ao.cmp(&bo).then_with(|| a.refno.to_string().cmp(&b.refno.to_string()))
    });

    Ok((segs, debug))
}

async fn fetch_tubi_segments_from_cache(
    branch_refno: RefnoEnum,
    dbno: Option<u32>,
    batch_id: Option<&str>,
    strict_dbno: bool,
) -> anyhow::Result<Vec<CacheTubiSeg>> {
    use crate::data_interface::db_meta_manager::db_meta;
    use crate::fast_model::instance_cache::InstanceCacheManager;

    let (segs, _debug) = fetch_tubi_segments_from_cache_with_debug(
        branch_refno,
        dbno,
        batch_id,
        strict_dbno,
    )
    .await?;
    Ok(segs)
}

async fn fetch_tubi_segments_from_cache_with_debug(
    branch_refno: RefnoEnum,
    dbno: Option<u32>,
    batch_id: Option<&str>,
    strict_dbno: bool,
) -> anyhow::Result<(Vec<CacheTubiSeg>, MbdPipeDebugInfo)> {
    use crate::data_interface::db_meta_manager::db_meta;
    use crate::fast_model::instance_cache::InstanceCacheManager;

    let mut debug = MbdPipeDebugInfo::default();
    debug.notes.push("source=cache".to_string());
    debug.requested_dbno = dbno;
    debug.requested_batch_id = batch_id.map(|s| s.to_string());

    let inferred_dbnum = if let Some(dbno) = dbno {
        dbno
    } else {
        db_meta().ensure_loaded()?;
        db_meta().get_dbnum_by_refno(branch_refno).unwrap_or(0)
    };
    if inferred_dbnum == 0 {
        anyhow::bail!("无法推导 dbno（请传 dbno 或先生成 output/scene_tree/db_meta_info.json）");
    }
    debug.inferred_dbnum = Some(inferred_dbnum);

    // 运行时约定：
    // - 若 MODEL_CACHE_DIR 指定，则优先使用
    // - 否则优先尝试项目内默认输出目录（AvevaMarineSample），再回退到 output/instance_cache
    let cache_dir = std::env::var("MODEL_CACHE_DIR")
        .ok()
        .filter(|s| !s.trim().is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            let p1 = PathBuf::from("output/AvevaMarineSample/instance_cache");
            if FsPath::new(&p1).exists() {
                return p1;
            }
            PathBuf::from("output/instance_cache")
        });
    debug.cache_dir = Some(cache_dir.display().to_string());

    let cache = InstanceCacheManager::new(&cache_dir).await?;
    let branch_u64 = branch_refno.refno();
    let mut active_dbnum = inferred_dbnum;
    let mut cached_refnos = cache.list_refnos(active_dbnum);
    if cached_refnos.is_empty() {
        // 兼容：前端传入的 dbno 可能是"db_meta 的 dbnum"（例如 7997），
        // 但 instance_cache 的 key 可能是"本次解析/缓存生成的 db 文件编号"（例如 1112）。
        // 因此当指定 dbno 无数据时，尝试回退到 cache 里实际存在的 dbnum。
        if strict_dbno && dbno.is_some() {
            anyhow::bail!(
                "instance_cache 无数据：dbno={} dir={}（strict_dbno=true，已禁止回退）",
                inferred_dbnum,
                cache_dir.display()
            );
        }
        let candidates = cache.list_dbnums();
        if candidates.len() == 1 {
            active_dbnum = candidates[0];
            cached_refnos = cache.list_refnos(active_dbnum);
            debug.fallback_used = true;
            debug.fallback_reason = Some("指定 dbno 无数据；cache 仅有 1 个 dbnum，已自动回退".to_string());
        } else {
            'outer: for cand in candidates {
                let cand_refnos = cache.list_refnos(cand);
                if cand_refnos.is_empty() {
                    continue;
                }
                // 探测该 dbnum 下是否有属于目标 branch 的 tubi 数据
                for &r in &cand_refnos {
                    if let Some(info) = cache.get_inst_info(cand, r).await {
                        if let Some(ref tubi) = info.tubi {
                            if info.info.owner_refno.refno() == branch_u64 {
                                active_dbnum = cand;
                                cached_refnos = cand_refnos;
                                debug.fallback_used = true;
                                debug.fallback_reason = Some(format!(
                                    "指定 dbno 无数据；已在候选 dbnum 中探测到分支数据，回退到 {}",
                                    cand
                                ));
                                break 'outer;
                            }
                        }
                    }
                }
            }
        }
    }
    if cached_refnos.is_empty() {
        anyhow::bail!(
            "instance_cache 无数据：dbno={} dir={}（且回退失败）",
            inferred_dbnum,
            cache_dir.display()
        );
    }
    debug.active_dbnum = Some(active_dbnum);
    debug.batches_all = vec!["per-refno".to_string()];

    // per-refno 存储：直接遍历 cached_refnos，读取 tubi 数据。
    // batch_id 参数在 per-refno 模式下不再有意义（每个 refno 只有一条记录）。
    let mut merged: HashMap<RefnoEnum, CacheTubiSeg> = HashMap::new();
    debug.batches_used = vec!["per-refno".to_string()];
    for &leave_refno in &cached_refnos {
        let Some(cached) = cache.get_inst_info(active_dbnum, leave_refno).await else { continue };
        let Some(ref tubi_data) = cached.info.tubi else { continue };
        if cached.info.owner_refno.refno() != branch_u64 {
            continue;
        }
        // cache 里 tubi start_pt/end_pt 可能未写入（或被裁剪），此时用 tubi 的 world_transform
        // 将 unit cylinder 的端点 (0,0,0)-(0,0,1) 变换到世界坐标，作为稳定兜底。
        let tubi_start = tubi_data.start_pt;
        let tubi_end = tubi_data.end_pt;
        let (start, end) = match (tubi_start, tubi_end) {
            (Some(s), Some(e)) => (s, e),
            _ => {
                let wt = cached.info.get_ele_world_transform();
                let m = wt.to_matrix();
                (
                    tubi_start
                        .unwrap_or_else(|| m.transform_point3(Vec3::new(0.0, 0.0, 0.0))),
                    tubi_end
                        .unwrap_or_else(|| m.transform_point3(Vec3::new(0.0, 0.0, 1.0))),
                )
            }
        };
        merged.insert(
            leave_refno,
            CacheTubiSeg {
                refno: leave_refno,
                arrive_refno: tubi_data.arrive_refno,
                order: tubi_data.index,
                start,
                end,
                arrive_axis: tubi_data.arrive_axis_pt.map(Vec3::from),
                leave_axis: tubi_data.leave_axis_pt.map(Vec3::from),
            },
        );
    }

    let mut segs: Vec<CacheTubiSeg> = merged.into_values().collect();
    segs.sort_by(|a, b| {
        let ao = a.order.unwrap_or(u32::MAX);
        let bo = b.order.unwrap_or(u32::MAX);
        ao.cmp(&bo).then_with(|| a.refno.to_string().cmp(&b.refno.to_string()))
    });
    Ok((segs, debug))
}

async fn fetch_tubi_segments_from_surreal_with_debug(
    branch_refno: RefnoEnum,
) -> anyhow::Result<(Vec<CacheTubiSeg>, MbdPipeDebugInfo)> {
    use aios_core::rs_surreal::geometry_query::PlantTransform;
    use aios_core::shape::pdms_shape::RsVec3;
    use aios_core::{SUL_DB, SurrealQueryExt};
    use serde::{Deserialize, Serialize};
    use surrealdb::types::SurrealValue;

    aios_core::init_surreal().await?;

    #[derive(Serialize, Deserialize, Debug, SurrealValue)]
    struct TubiRelateRow {
        pub owner_refno: RefnoEnum,
        pub leave_refno: RefnoEnum,
        pub arrive_refno: RefnoEnum,
        #[serde(default)]
        pub world_trans: Option<PlantTransform>,
        #[serde(default)]
        pub start_pt: Option<RsVec3>,
        #[serde(default)]
        pub end_pt: Option<RsVec3>,
        /// 端口轴线点（可选；由 cache_flush 写入到 tubi_relate.arrive_axis/leave_axis -> vec3）
        #[serde(default)]
        pub arrive_axis: Option<RsVec3>,
        #[serde(default)]
        pub leave_axis: Option<RsVec3>,
        #[serde(default)]
        pub index: Option<i64>,
    }

    let mut debug = MbdPipeDebugInfo::default();
    debug.notes.push("source=db".to_string());

    let pe_key = branch_refno.to_pe_key();
    let sql = format!(
        r#"
        SELECT
            id[0] as owner_refno,
            in as leave_refno,
            out as arrive_refno,
            world_trans.d as world_trans,
            start_pt.d as start_pt,
            end_pt.d as end_pt,
            arrive_axis.d as arrive_axis,
            leave_axis.d as leave_axis,
            id[1] as index
        FROM tubi_relate:[{pe_key}, 0]..[{pe_key}, ..];
        "#
    );

    let rows: Vec<TubiRelateRow> = SUL_DB.query_take(&sql, 0).await?;
    if rows.is_empty() {
        anyhow::bail!("tubi_relate 无结果（branch_refno={} pe_key={}）", branch_refno, pe_key);
    }

    let mut segs: Vec<CacheTubiSeg> = Vec::with_capacity(rows.len());
    for row in rows {
        // DB 里 start/end 可能未写入（或被裁剪），此时用 world_trans 将 unit cylinder 的端点
        // (0,0,0)-(0,0,1) 变换到世界坐标，作为稳定兜底。
        let wt = row.world_trans.unwrap_or_default();
        let m = wt.to_matrix();
        let start = row
            .start_pt
            .map(|p| p.0)
            .unwrap_or_else(|| m.transform_point3(Vec3::new(0.0, 0.0, 0.0)));
        let end = row
            .end_pt
            .map(|p| p.0)
            .unwrap_or_else(|| m.transform_point3(Vec3::new(0.0, 0.0, 1.0)));

        segs.push(CacheTubiSeg {
            refno: row.leave_refno,
            arrive_refno: Some(row.arrive_refno),
            order: row.index.and_then(|i| u32::try_from(i).ok()),
            start,
            end,
            arrive_axis: row.arrive_axis.map(|p| p.0),
            leave_axis: row.leave_axis.map(|p| p.0),
        });
    }

    segs.sort_by(|a, b| {
        let ao = a.order.unwrap_or(u32::MAX);
        let bo = b.order.unwrap_or(u32::MAX);
        ao.cmp(&bo).then_with(|| a.refno.to_string().cmp(&b.refno.to_string()))
    });

    Ok((segs, debug))
}

fn determine_weld_type(noun1: Option<&str>, noun2: Option<&str>) -> MbdWeldType {
    let n1 = noun1.unwrap_or("");
    let n2 = noun2.unwrap_or("");
    // 承插焊
    if n1.contains("SW") || n2.contains("SW") {
        return MbdWeldType::Socket;
    }
    // 角焊（法兰等）
    if n1.contains("FLAN") || n2.contains("FLAN") || n1.contains("FBLI") || n2.contains("FBLI") {
        return MbdWeldType::Fillet;
    }
    MbdWeldType::Butt
}

async fn try_fill_branch_name_and_attrs(
    branch_refno: RefnoEnum,
) -> anyhow::Result<(String, BranchAttrsDto)> {
    let att = aios_core::get_named_attmap(branch_refno).await?;

    let mut attrs = BranchAttrsDto::default();

    // 说明：字段键名按常见 PDMS 属性名直取；若不存在则保持 None。
    // 这些键名以 markpipe/branAttlist.txt 的语义为准，后续若需映射/单位换算，可在此集中处理。
    attrs.duty = att.get_as_string("DUTY").map(fix_mojibake_utf8_latin1);
    attrs.pspec = att.get_as_string("PSPEC").map(fix_mojibake_utf8_latin1);
    attrs.rccm = att.get_as_string("RCCM").map(fix_mojibake_utf8_latin1);
    attrs.clean = att.get_as_string("CLEAN").map(fix_mojibake_utf8_latin1);
    attrs.temp = att.get_as_string("TEMP").map(fix_mojibake_utf8_latin1);
    attrs.pressure = att.get_f64("PRESSURE").map(|v| v as f32);
    attrs.ispec = att.get_as_string("ISPEC").map(fix_mojibake_utf8_latin1);
    attrs.insuthick = att.get_f64("INSUTHICK").map(|v| v as f32);
    attrs.tspec = att.get_as_string("TSPEC").map(fix_mojibake_utf8_latin1);
    attrs.swgd = att.get_as_string("SWGD").map(fix_mojibake_utf8_latin1);
    attrs.drawnum = att.get_as_string("DRAWNUM").map(fix_mojibake_utf8_latin1);
    attrs.rev = att.get_as_string("REV").map(fix_mojibake_utf8_latin1);
    attrs.status = att.get_as_string("STATUS").map(fix_mojibake_utf8_latin1);
    attrs.fluid = att.get_as_string("FLUID").map(fix_mojibake_utf8_latin1);

    Ok((fix_mojibake_utf8_latin1(att.get_name_or_default()), attrs))
}

async fn try_build_tree_index_for_refno(
    refno: RefnoEnum,
) -> anyhow::Result<crate::fast_model::gen_model::tree_index_manager::TreeIndexManager> {
    use crate::fast_model::gen_model::tree_index_manager::TreeIndexManager;
    let dbnum = TreeIndexManager::resolve_dbnum_for_refno(refno)?;
    Ok(TreeIndexManager::with_default_dir(vec![dbnum]))
}

#[derive(Clone, Copy, Debug)]
struct WeldJoint {
    left_endpoint: Vec3,
    right_endpoint: Vec3,
    mid: Vec3,
}

#[inline]
fn other_endpoint(a: Vec3, b: Vec3, used: Vec3) -> Vec3 {
    // closest_endpoints 选出来的端点必然等于 a 或 b（拷贝值），此处用极小阈值判断。
    const EPS: f32 = 1e-4;
    if a.distance(used) < EPS {
        b
    } else if b.distance(used) < EPS {
        a
    } else {
        // 兜底：若 used 不是端点（理论上不应发生），则取离 used 更远的端点作为“外侧端点”。
        if a.distance(used) > b.distance(used) { a } else { b }
    }
}

/// 生成“焊口链式尺寸”的点序列：左端点 -> (各焊口中点) -> 右端点。
///
/// - weld_joints 按段序（i, i+1）顺序输入
/// - 若 weld_joints 为空，则退化为 `[first.start, first.end]`
fn build_chain_points_from_ends(ends: &[(Vec3, Vec3)], weld_joints: &[WeldJoint]) -> Vec<Vec3> {
    let mut out: Vec<Vec3> = Vec::new();
    if ends.is_empty() {
        return out;
    }

    if weld_joints.is_empty() {
        out.push(ends[0].0);
        out.push(ends[0].1);
        return out;
    }

    let left_end = other_endpoint(ends[0].0, ends[0].1, weld_joints[0].left_endpoint);
    let right_end = other_endpoint(
        ends[ends.len() - 1].0,
        ends[ends.len() - 1].1,
        weld_joints[weld_joints.len() - 1].right_endpoint,
    );

    out.push(left_end);
    for j in weld_joints {
        out.push(j.mid);
    }
    out.push(right_end);
    out
}

#[inline]
fn midpoint(a: Vec3, b: Vec3) -> Vec3 {
    (a + b) * 0.5
}

/// 计算两条线段的“最近端点对”（仅端点，不做线段到线段距离）。
///
/// 目的：容忍段方向反转（start/end 颠倒）导致的焊缝漏检。
#[inline]
fn closest_endpoints(a0: Vec3, a1: Vec3, b0: Vec3, b1: Vec3) -> (Vec3, Vec3, f32) {
    let pairs = [
        (a0, b0),
        (a0, b1),
        (a1, b0),
        (a1, b1),
    ];
    let mut best = (pairs[0].0, pairs[0].1, pairs[0].0.distance(pairs[0].1));
    for (pa, pb) in pairs.into_iter().skip(1) {
        let d = pa.distance(pb);
        if d < best.2 {
            best = (pa, pb, d);
        }
    }
    best
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_closest_endpoints_direction_flip() {
        let seg1_start = Vec3::new(0.0, 0.0, 0.0);
        let seg1_end = Vec3::new(1.0, 0.0, 0.0);

        // seg2 方向反转：本应与 seg1_end 相连
        let seg2_start = Vec3::new(2.0, 0.0, 0.0);
        let seg2_end = Vec3::new(1.0, 0.0, 0.0);

        let (_p1, _p2, dist) = closest_endpoints(seg1_start, seg1_end, seg2_start, seg2_end);
        assert!((dist - 0.0).abs() < 1e-6);
    }

    #[test]
    fn test_midpoint() {
        let a = Vec3::new(0.0, 0.0, 0.0);
        let b = Vec3::new(2.0, 0.0, 0.0);
        let m = midpoint(a, b);
        assert_eq!(m.to_array(), [1.0, 0.0, 0.0]);
    }

    #[test]
    fn test_other_endpoint() {
        let a = Vec3::new(0.0, 0.0, 0.0);
        let b = Vec3::new(1.0, 0.0, 0.0);
        assert_eq!(other_endpoint(a, b, a).to_array(), b.to_array());
        assert_eq!(other_endpoint(a, b, b).to_array(), a.to_array());
    }

    #[test]
    fn test_build_chain_points_from_ends_two_segments() {
        // 两段直线：seg0: 0->1, seg1: 1->2
        let ends = vec![
            (Vec3::new(0.0, 0.0, 0.0), Vec3::new(1.0, 0.0, 0.0)),
            (Vec3::new(1.0, 0.0, 0.0), Vec3::new(2.0, 0.0, 0.0)),
        ];
        let joints = vec![WeldJoint {
            left_endpoint: Vec3::new(1.0, 0.0, 0.0),
            right_endpoint: Vec3::new(1.0, 0.0, 0.0),
            mid: Vec3::new(1.0, 0.0, 0.0),
        }];

        let pts = build_chain_points_from_ends(&ends, &joints);
        assert_eq!(pts.len(), 3);
        assert_eq!(pts[0].to_array(), [0.0, 0.0, 0.0]);
        assert_eq!(pts[1].to_array(), [1.0, 0.0, 0.0]);
        assert_eq!(pts[2].to_array(), [2.0, 0.0, 0.0]);
    }

    #[test]
    fn test_segment_port_points_use_axis_when_present() {
        let seg = CacheTubiSeg {
            refno: RefnoEnum::from("1_1"),
            arrive_refno: None,
            order: None,
            start: Vec3::new(0.0, 0.0, 0.0),
            end: Vec3::new(10.0, 0.0, 0.0),
            arrive_axis: Some(Vec3::new(9.0, 0.0, 0.0)),
            leave_axis: Some(Vec3::new(1.0, 0.0, 0.0)),
        };

        let (a, b) = segment_port_points(&seg);
        assert_eq!(a.to_array(), [1.0, 0.0, 0.0]);
        assert_eq!(b.to_array(), [9.0, 0.0, 0.0]);
    }

    #[test]
    fn test_segment_port_points_fallback_to_start_end() {
        let seg = CacheTubiSeg {
            refno: RefnoEnum::from("1_1"),
            arrive_refno: None,
            order: None,
            start: Vec3::new(2.0, 0.0, 0.0),
            end: Vec3::new(5.0, 0.0, 0.0),
            arrive_axis: None,
            leave_axis: None,
        };

        let (a, b) = segment_port_points(&seg);
        assert_eq!(a.to_array(), [2.0, 0.0, 0.0]);
        assert_eq!(b.to_array(), [5.0, 0.0, 0.0]);
    }
}



