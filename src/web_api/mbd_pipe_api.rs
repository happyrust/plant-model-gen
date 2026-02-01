//! MBD 管道标注 API（首期：管道分支 BRAN/HANG）
//!
//! 目标：为 plant3d-web 提供“管道 MBD 标注”所需的结构化数据（段/尺寸/焊缝/坡度）。
//! 说明：本接口采用“后端提供语义点位 + 前端做屏幕布局/避让”的分层方式，便于渐进式对齐 MBD(PML)。

use aios_core::{
    NamedAttrMap, NamedAttrValue, RefnoEnum, get_named_attmap, query_filter_ancestors,
    rs_surreal::pipeline::PipelineQueryService,
};
use axum::{
    Router,
    extract::{Path, Query},
    response::IntoResponse,
    routing::get,
    Json,
};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct MbdPipeQuery {
    /// 最小坡度（0.001 对齐 MBD 默认）
    pub min_slope: f32,
    /// 最大坡度（0.1 对齐 MBD 默认）
    pub max_slope: f32,
    /// 最小尺寸长度（mm）
    pub dim_min_length: f32,
    /// 焊缝合并阈值（mm）：相邻段端口距离小于该值则认为是焊缝
    pub weld_merge_threshold: f32,
    pub include_dims: bool,
    pub include_welds: bool,
    pub include_slopes: bool,
}

impl Default for MbdPipeQuery {
    fn default() -> Self {
        Self {
            min_slope: 0.001,
            max_slope: 0.1,
            dim_min_length: 1.0,
            weld_merge_threshold: 1.0,
            include_dims: true,
            include_welds: true,
            include_slopes: true,
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

#[derive(Debug, Clone, Serialize)]
pub struct MbdDimDto {
    pub id: String,
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

pub fn create_mbd_pipe_routes() -> Router {
    Router::new().route("/api/mbd/pipe/{refno}", get(get_mbd_pipe))
}

async fn get_mbd_pipe(
    Path(refno): Path<String>,
    Query(query): Query<MbdPipeQuery>,
) -> impl IntoResponse {
    let input_refno_enum = match refno.parse::<RefnoEnum>() {
        Ok(v) => v,
        Err(e) => {
            return Json(MbdPipeResponse {
                success: false,
                error_message: Some(format!("无效的 refno: {e}")),
                data: None,
            });
        }
    };

    let branch_refno = match resolve_branch_refno(input_refno_enum.clone()).await {
        Ok(v) => v,
        Err(e) => {
            return Json(MbdPipeResponse {
                success: false,
                error_message: Some(e.to_string()),
                data: None,
            });
        }
    };

    let branch_att = match get_named_attmap(branch_refno.clone()).await {
        Ok(v) => v,
        Err(e) => {
            return Json(MbdPipeResponse {
                success: false,
                error_message: Some(format!("读取分支属性失败: {e}")),
                data: None,
            });
        }
    };
    let branch_name = attr_string(&branch_att, "NAME").unwrap_or_else(|| branch_refno.to_string());
    let branch_attrs = BranchAttrsDto {
        duty: attr_string(&branch_att, "DUTY"),
        pspec: attr_string(&branch_att, "PSPEC"),
        rccm: attr_string(&branch_att, "RCCM"),
        clean: attr_string(&branch_att, "CLEAN"),
        temp: attr_string(&branch_att, "TEMP"),
        pressure: attr_f32(&branch_att, "PRESS"),
        ispec: attr_string(&branch_att, "ISPEC"),
        insuthick: attr_f32(&branch_att, "INSUTHICK"),
        tspec: attr_string(&branch_att, "TSPEC"),
        swgd: attr_string(&branch_att, "SWGD"),
        drawnum: attr_string(&branch_att, "DRAWNUM"),
        rev: attr_string(&branch_att, "REV"),
        status: attr_string(&branch_att, "STATUS").or_else(|| attr_string(&branch_att, "status")),
        fluid: attr_string(&branch_att, "FLUID"),
    };

    let segments = match PipelineQueryService::fetch_branch_segments(branch_refno.clone()).await {
        Ok(v) => v,
        Err(e) => {
            return Json(MbdPipeResponse {
                success: false,
                error_message: Some(format!("查询分支管段失败: {e}")),
                data: None,
            });
        }
    };

    let mut out_segments: Vec<MbdPipeSegmentDto> = Vec::with_capacity(segments.len());
    for (i, seg) in segments.iter().enumerate() {
        out_segments.push(MbdPipeSegmentDto {
            id: format!("seg:{}:{i}", seg.refno),
            refno: seg.refno.to_string(),
            noun: seg.noun_raw.clone().unwrap_or_else(|| seg.noun.to_string()),
            name: seg.name.clone(),
            arrive: seg.arrive.map(|p| [p.world_pos.x, p.world_pos.y, p.world_pos.z]),
            leave: seg.leave.map(|p| [p.world_pos.x, p.world_pos.y, p.world_pos.z]),
            length: seg.length,
            straight_length: seg.straight_length,
            outside_diameter: seg.outside_diameter,
            bore: seg.bore,
        });
    }

    // ===== dims / welds / slopes =====
    let mut dims: Vec<MbdDimDto> = Vec::new();
    if query.include_dims {
        for (i, seg) in segments.iter().enumerate() {
            let Some(span) = seg.main_span() else { continue };
            if span.length < query.dim_min_length {
                continue;
            }
            dims.push(MbdDimDto {
                id: format!("dim:{}:{i}", seg.refno),
                start: [span.start.world_pos.x, span.start.world_pos.y, span.start.world_pos.z],
                end: [span.end.world_pos.x, span.end.world_pos.y, span.end.world_pos.z],
                length: span.length,
                text: format!("{:.0}", span.length),
            });
        }
    }

    let mut welds: Vec<MbdWeldDto> = Vec::new();
    if query.include_welds {
        let mut shop_idx = 0usize;
        let mut field_idx = 0usize;

        for i in 0..segments.len().saturating_sub(1) {
            let seg1 = &segments[i];
            let seg2 = &segments[i + 1];
            let (Some(leave1), Some(arrive2)) = (seg1.leave, seg2.arrive) else { continue };
            if leave1.world_pos.distance(arrive2.world_pos) >= query.weld_merge_threshold {
                continue;
            }

            let weld_type =
                determine_weld_type(seg1.noun_raw.as_deref(), seg2.noun_raw.as_deref());

            // 首期：简单近似 MBD shop/field 规则：分支两端优先现场焊；中间按“常见预制件”判断是否车间焊。
            let at_ends = i == 0 || (i + 1) == (segments.len().saturating_sub(1));
            let shop_candidate = is_shop_candidate(seg1.noun_raw.as_deref())
                || is_shop_candidate(seg2.noun_raw.as_deref());
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
                position: [leave1.world_pos.x, leave1.world_pos.y, leave1.world_pos.z],
                weld_type,
                is_shop,
                label,
                left_refno: seg1.refno.to_string(),
                right_refno: seg2.refno.to_string(),
            });
        }
    }

    let mut slopes: Vec<MbdSlopeDto> = Vec::new();
    if query.include_slopes {
        for (i, seg) in segments.iter().enumerate() {
            let (Some(a), Some(b)) = (seg.arrive, seg.leave) else { continue };
            let dx = b.world_pos.x - a.world_pos.x;
            let dy = b.world_pos.y - a.world_pos.y;
            let dz = b.world_pos.z - a.world_pos.z;
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
                start: [a.world_pos.x, a.world_pos.y, a.world_pos.z],
                end: [b.world_pos.x, b.world_pos.y, b.world_pos.z],
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

    Json(MbdPipeResponse {
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
        }),
    })
}

async fn resolve_branch_refno(input: RefnoEnum) -> anyhow::Result<RefnoEnum> {
    let att = get_named_attmap(input.clone()).await.unwrap_or_default();
    let ty = att.get_type_str();
    if ty == "BRAN" || ty == "HANG" {
        return Ok(input);
    }

    // aios_core::query_filter_ancestors 返回 root->parent 顺序（TreeIndex 路径）。
    // 我们取最后一个（最靠近 input 的祖先）。
    let ancestors = query_filter_ancestors(input, &["BRAN", "HANG"]).await?;
    ancestors
        .last()
        .cloned()
        .ok_or_else(|| anyhow::anyhow!("该 refno 不在 BRAN/HANG 分支下，无法生成管道标注"))
}

fn attr_value<'a>(attrs: &'a NamedAttrMap, key: &str) -> Option<&'a NamedAttrValue> {
    attrs.get(key).or_else(|| {
        let alt = format!(":{key}");
        attrs.get(alt.as_str())
    })
}

fn attr_string(attrs: &NamedAttrMap, key: &str) -> Option<String> {
    attr_value(attrs, key).and_then(|v| match v {
        NamedAttrValue::StringType(s)
        | NamedAttrValue::WordType(s)
        | NamedAttrValue::ElementType(s) => Some(s.clone()),
        _ => None,
    })
}

fn attr_f32(attrs: &NamedAttrMap, key: &str) -> Option<f32> {
    attr_value(attrs, key).and_then(|v| match v {
        NamedAttrValue::F32Type(x) => Some(*x),
        NamedAttrValue::IntegerType(x) => Some(*x as f32),
        NamedAttrValue::LongType(x) => Some(*x as f32),
        _ => None,
    })
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

fn is_shop_candidate(noun: Option<&str>) -> bool {
    let n = noun.unwrap_or("");
    // 与 MBD（PML）首期近似：中间段的常见预制件倾向车间焊
    n.contains("ELBO")
        || n.contains("TEE")
        || n.contains("REDU")
        || n.contains("CROS")
        || n.contains("FLAN")
        || n.contains("FBLI")
}

