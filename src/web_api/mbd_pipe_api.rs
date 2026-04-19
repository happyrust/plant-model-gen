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
    Json, Router,
    extract::{Path, Query},
    http::{HeaderValue, header::CONTENT_TYPE},
    response::{IntoResponse, Response},
    routing::{get, post},
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

#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum MbdPipeMode {
    /// 与 plant3d-web 默认视图一致：后台排版优先（查询串为 `layout_first`）
    #[serde(rename = "layout_first")]
    LayoutFirst,
    Construction,
    Inspection,
}

impl Default for MbdPipeMode {
    fn default() -> Self {
        Self::Construction
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct MbdPipeQuery {
    /// 语义模式：layout_first=排版优先（与 construction 默认开关一致），construction=施工表达，inspection=几何校核
    pub mode: Option<MbdPipeMode>,
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
    pub include_chain_dims: Option<bool>,
    /// 是否额外输出“总长尺寸”（kind=overall）到 dims 数组
    pub include_overall_dim: Option<bool>,
    /// 是否额外输出“端口间距尺寸”（优先用 arrive_axis_pt/leave_axis_pt；kind=port）到 dims 数组
    pub include_port_dims: Option<bool>,
    /// 焊缝合并阈值（mm）：相邻段端口距离小于该值则认为是焊缝
    pub weld_merge_threshold: f32,
    pub include_dims: Option<bool>,
    pub include_welds: Option<bool>,
    pub include_slopes: Option<bool>,
    /// 是否输出切管/直管长度清单
    pub include_cut_tubis: Option<bool>,
    /// 是否输出离散管件标注目标
    pub include_fittings: Option<bool>,
    /// 是否输出类型化标签
    pub include_tags: Option<bool>,
    /// 是否输出布局提示（内嵌到各类标注对象）
    pub include_layout_hints: Option<bool>,
    /// 是否尝试填充分支属性（失败则忽略，不影响 success）
    pub include_branch_attrs: bool,
    /// 是否尝试用 TreeIndex 的 noun 辅助推断 weld_type（默认关闭，避免额外依赖/误判）
    pub include_weld_nouns: bool,
    /// 是否输出弯头数据（BEND/ELBO）
    pub include_bends: Option<bool>,
    /// 弯头标注模式：workpoint（中心线交点，默认）/ facecenter（端面中心）
    pub bend_mode: MbdBendMode,
}

impl Default for MbdPipeQuery {
    fn default() -> Self {
        Self {
            mode: None,
            source: MbdPipeSource::Db,
            dbno: None,
            batch_id: None,
            debug: false,
            strict_dbno: false,
            min_slope: 0.001,
            max_slope: 0.1,
            dim_min_length: 1.0,
            include_chain_dims: None,
            include_overall_dim: None,
            include_port_dims: None,
            weld_merge_threshold: 1.0,
            include_dims: None,
            include_welds: None,
            include_slopes: None,
            include_cut_tubis: None,
            include_fittings: None,
            include_tags: None,
            include_layout_hints: None,
            include_branch_attrs: true,
            include_weld_nouns: false,
            include_bends: None,
            bend_mode: MbdBendMode::default(),
        }
    }
}

#[derive(Debug, Clone, Copy)]
struct MbdPipeModeDefaults {
    include_dims: bool,
    include_chain_dims: bool,
    include_overall_dim: bool,
    include_port_dims: bool,
    include_welds: bool,
    include_slopes: bool,
    include_bends: bool,
    include_cut_tubis: bool,
    include_fittings: bool,
    include_tags: bool,
    include_layout_hints: bool,
}

impl MbdPipeModeDefaults {
    fn for_mode(mode: MbdPipeMode) -> Self {
        match mode {
            // layout_first 与 construction 使用同一套几何/标注默认；layout 结果由前端或其它查询参数控制
            MbdPipeMode::LayoutFirst | MbdPipeMode::Construction => Self {
                include_dims: true,
                include_chain_dims: true,
                include_overall_dim: false,
                include_port_dims: false,
                include_welds: true,
                include_slopes: true,
                include_bends: false,
                include_cut_tubis: true,
                include_fittings: true,
                include_tags: true,
                include_layout_hints: true,
            },
            MbdPipeMode::Inspection => Self {
                include_dims: true,
                include_chain_dims: false,
                include_overall_dim: false,
                include_port_dims: true,
                include_welds: false,
                include_slopes: false,
                include_bends: false,
                include_cut_tubis: false,
                include_fittings: false,
                include_tags: false,
                include_layout_hints: false,
            },
        }
    }
}

#[derive(Debug, Clone)]
struct ResolvedMbdPipeQuery {
    mode: MbdPipeMode,
    source: MbdPipeSource,
    dbno: Option<u32>,
    batch_id: Option<String>,
    debug: bool,
    strict_dbno: bool,
    min_slope: f32,
    max_slope: f32,
    dim_min_length: f32,
    include_chain_dims: bool,
    include_overall_dim: bool,
    include_port_dims: bool,
    weld_merge_threshold: f32,
    include_dims: bool,
    include_welds: bool,
    include_slopes: bool,
    include_cut_tubis: bool,
    include_fittings: bool,
    include_tags: bool,
    include_layout_hints: bool,
    include_branch_attrs: bool,
    include_weld_nouns: bool,
    include_bends: bool,
    bend_mode: MbdBendMode,
}

impl MbdPipeQuery {
    fn resolve(&self) -> ResolvedMbdPipeQuery {
        let mode = self.mode.unwrap_or_default();
        let defaults = MbdPipeModeDefaults::for_mode(mode);
        ResolvedMbdPipeQuery {
            mode,
            source: self.source,
            dbno: self.dbno,
            batch_id: self.batch_id.clone(),
            debug: self.debug,
            strict_dbno: self.strict_dbno,
            min_slope: self.min_slope,
            max_slope: self.max_slope,
            dim_min_length: self.dim_min_length,
            include_chain_dims: self
                .include_chain_dims
                .unwrap_or(defaults.include_chain_dims),
            include_overall_dim: self
                .include_overall_dim
                .unwrap_or(defaults.include_overall_dim),
            include_port_dims: self.include_port_dims.unwrap_or(defaults.include_port_dims),
            weld_merge_threshold: self.weld_merge_threshold,
            include_dims: self.include_dims.unwrap_or(defaults.include_dims),
            include_welds: self.include_welds.unwrap_or(defaults.include_welds),
            include_slopes: self.include_slopes.unwrap_or(defaults.include_slopes),
            include_cut_tubis: self.include_cut_tubis.unwrap_or(defaults.include_cut_tubis),
            include_fittings: self.include_fittings.unwrap_or(defaults.include_fittings),
            include_tags: self.include_tags.unwrap_or(defaults.include_tags),
            include_layout_hints: self
                .include_layout_hints
                .unwrap_or(defaults.include_layout_hints),
            include_branch_attrs: self.include_branch_attrs,
            include_weld_nouns: self.include_weld_nouns,
            include_bends: self.include_bends.unwrap_or(defaults.include_bends),
            bend_mode: self.bend_mode,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MbdPipeResponse {
    pub success: bool,
    pub error_message: Option<String>,
    pub data: Option<MbdPipeData>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MbdPipeData {
    pub input_refno: String,
    pub branch_refno: String,
    pub branch_name: String,
    pub branch_attrs: BranchAttrsDto,
    pub segments: Vec<MbdPipeSegmentDto>,
    pub dims: Vec<MbdDimDto>,
    pub cut_tubis: Vec<MbdCutTubiDto>,
    pub welds: Vec<MbdWeldDto>,
    pub slopes: Vec<MbdSlopeDto>,
    pub fittings: Vec<MbdFittingDto>,
    pub tags: Vec<MbdTagDto>,
    pub bends: Vec<MbdBendDto>,
    pub stats: MbdPipeStats,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub debug_info: Option<MbdPipeDebugInfo>,
    /// 仅当 `mode=layout_first` 时填充：由 `aios_core::mbd::BranchCalculator::solve_branch`
    /// + `assemble_prelaid_out` 产出的排版结果，前端 `renderLaidOutLinearDims` 消费。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub layout_result: Option<aios_core::mbd::LayoutResult>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct MbdPipeStats {
    pub segments_count: usize,
    pub dims_count: usize,
    pub cut_tubis_count: usize,
    pub welds_count: usize,
    pub slopes_count: usize,
    pub fittings_count: usize,
    pub tags_count: usize,
    pub bends_count: usize,
}

/// 分支属性（对齐 MBD/markpipe/branAttlist.txt 的 BranAttarr）
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
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

#[derive(Debug, Clone, Serialize, Deserialize)]
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

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
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

#[derive(Debug, Clone, Serialize, Deserialize)]
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
    #[serde(skip_serializing_if = "Option::is_none")]
    pub layout_hint: Option<MbdLayoutHint>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub enum MbdWeldType {
    Butt = 0,
    Fillet = 1,
    Socket = 2,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MbdWeldDto {
    pub id: String,
    pub position: [f32; 3],
    pub weld_type: MbdWeldType,
    /// true=车间焊（A），false=现场焊（M）
    pub is_shop: bool,
    pub label: String,
    pub left_refno: String,
    pub right_refno: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub layout_hint: Option<MbdLayoutHint>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MbdSlopeDto {
    pub id: String,
    pub start: [f32; 3],
    pub end: [f32; 3],
    /// 坡度（dz / horizontal_dist），保留符号
    pub slope: f32,
    pub text: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MbdLayoutHint {
    pub anchor_point: [f32; 3],
    pub primary_axis: [f32; 3],
    pub offset_dir: [f32; 3],
    pub char_dir: [f32; 3],
    pub label_role: String,
    pub avoid_line_of_sight: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub owner_segment_id: Option<String>,
    pub offset_level: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub suppress_reason: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MbdCutTubiDto {
    pub id: String,
    pub segment_id: String,
    pub refno: String,
    pub start: [f32; 3],
    pub end: [f32; 3],
    pub length: f32,
    pub text: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub layout_hint: Option<MbdLayoutHint>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum MbdFittingKind {
    Elbo,
    Bend,
    Tee,
    Olet,
    Flan,
    Unknown,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MbdFittingDto {
    pub id: String,
    pub refno: String,
    pub noun: String,
    pub kind: MbdFittingKind,
    pub anchor_point: [f32; 3],
    #[serde(skip_serializing_if = "Option::is_none")]
    pub angle: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub radius: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub face_center_1: Option<[f32; 3]>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub face_center_2: Option<[f32; 3]>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub layout_hint: Option<MbdLayoutHint>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MbdTagDto {
    pub id: String,
    pub refno: String,
    pub noun: String,
    pub role: String,
    pub text: String,
    pub position: [f32; 3],
    #[serde(skip_serializing_if = "Option::is_none")]
    pub layout_hint: Option<MbdLayoutHint>,
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum MbdBendMode {
    /// 中心线交点（WorkPoint）
    Workpoint,
    /// 端面中心（P1/P2 FaceCenter）
    Facecenter,
}

impl Default for MbdBendMode {
    fn default() -> Self {
        Self::Workpoint
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MbdBendDto {
    pub id: String,
    pub refno: String,
    pub noun: String,
    /// 弯曲角度（度）
    pub angle: Option<f32>,
    /// 弯曲半径（mm）
    pub radius: Option<f32>,
    /// 中心线交点（WorkPoint）
    pub work_point: [f32; 3],
    /// 端面中心 P1（ARRI 侧）
    pub face_center_1: Option<[f32; 3]>,
    /// 端面中心 P2（LEAV 侧）
    pub face_center_2: Option<[f32; 3]>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
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
    Router::new()
        .route("/api/mbd/pipe/{refno}", get(get_mbd_pipe))
        .route("/api/mbd/generate", post(post_generate_mbd))
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
            let has_cjk = fixed
                .chars()
                .any(|c| ('\u{4E00}'..='\u{9FFF}').contains(&c));
            if has_cjk { fixed } else { s }
        }
        Err(_) => s,
    }
}

async fn resolve_effective_branch_refno(input_refno: RefnoEnum) -> anyhow::Result<RefnoEnum> {
    use aios_core::{SUL_DB, SurrealQueryExt};
    use serde::{Deserialize, Serialize};
    use surrealdb::types::SurrealValue;

    #[derive(Debug, Serialize, Deserialize, SurrealValue)]
    struct BranchRefLinkRow {
        #[serde(default)]
        noun: Option<String>,
        #[serde(default)]
        href: Option<RefnoEnum>,
        #[serde(default)]
        tref: Option<RefnoEnum>,
        #[serde(default)]
        owner: Option<RefnoEnum>,
    }

    #[derive(Debug, Serialize, Deserialize, SurrealValue)]
    struct BranchOwnerRow {
        #[serde(default)]
        noun: Option<String>,
        #[serde(default)]
        owner: Option<RefnoEnum>,
    }

    let sql = format!(
        "SELECT noun, refno.HREF as href, refno.TREF as tref, owner.refno as owner FROM {} LIMIT 1",
        input_refno.to_pe_key()
    );
    let row: Option<BranchRefLinkRow> = SUL_DB.query_take(&sql, 0).await?;
    let Some(row) = row else {
        return Ok(input_refno);
    };

    let noun = row.noun.unwrap_or_default().trim().to_ascii_uppercase();
    if noun != "HANG" {
        return Ok(input_refno);
    }

    async fn promote_via_owner_chain(
        start_refno: RefnoEnum,
    ) -> anyhow::Result<Option<RefnoEnum>> {
        use aios_core::{SUL_DB, SurrealQueryExt};

        let mut current = start_refno;
        for _ in 0..64 {
            let sql = format!(
                "SELECT noun, owner.refno as owner FROM {} LIMIT 1",
                current.to_pe_key()
            );
            let row: Option<BranchOwnerRow> = SUL_DB.query_take(&sql, 0).await?;
            let Some(row) = row else {
                return Ok(None);
            };

            let noun = row.noun.unwrap_or_default().trim().to_ascii_uppercase();
            if matches!(noun.as_str(), "BRAN" | "HANG") {
                return Ok(Some(current));
            }

            let Some(owner) = row.owner.filter(|owner| !owner.is_unset()) else {
                return Ok(None);
            };
            current = owner;
        }
        Ok(None)
    }

    for link_refno in [row.href, row.tref, row.owner].into_iter().flatten() {
        if let Some(candidate) = promote_via_owner_chain(link_refno).await? {
            return Ok(candidate);
        }
    }

    Ok(input_refno)
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
    /// 外径（mm）。对应 PML `aod of $!tubi`。从 pe.aod 或 pe.attrs.AOD 取；
    /// 不存在时 None，由 `compute_branch_layout_result` 回退到 default_od=229。
    outside_diameter: Option<f32>,
}

#[derive(Debug, Clone)]
struct BranchTopology {
    branch_refno: RefnoEnum,
    segments: Vec<CacheTubiSeg>,
}

struct BranchTopologyBuilder;

impl BranchTopologyBuilder {
    fn build(branch_refno: RefnoEnum, segments: Vec<CacheTubiSeg>) -> BranchTopology {
        BranchTopology {
            branch_refno,
            segments,
        }
    }
}

#[derive(Debug, Clone)]
struct RawFittingElement {
    refno: RefnoEnum,
    noun: String,
    anchor_point: Vec3,
    face_center_1: Option<Vec3>,
    face_center_2: Option<Vec3>,
    angle: Option<f32>,
    radius: Option<f32>,
}

#[derive(Debug, Clone, Default)]
struct BranchMeasurementOutput {
    segments: Vec<MbdPipeSegmentDto>,
    dims: Vec<MbdDimDto>,
    cut_tubis: Vec<MbdCutTubiDto>,
    welds: Vec<MbdWeldDto>,
    slopes: Vec<MbdSlopeDto>,
    fittings: Vec<MbdFittingDto>,
    tags: Vec<MbdTagDto>,
    bends: Vec<MbdBendDto>,
}

struct AnnotationLayoutPlanner;

impl AnnotationLayoutPlanner {
    fn segment_id(seg: &CacheTubiSeg, index: usize) -> String {
        format!("seg:{}:{index}", seg.refno)
    }

    fn axis_or_default(start: Vec3, end: Vec3) -> Vec3 {
        let axis = end - start;
        if axis.length_squared() > 1e-6 {
            axis.normalize()
        } else {
            Vec3::X
        }
    }

    fn choose_offset_dir(axis: Vec3) -> Vec3 {
        let up = if axis.cross(Vec3::Z).length_squared() > 1e-6 {
            Vec3::Z
        } else {
            Vec3::Y
        };
        let dir = axis.cross(up);
        if dir.length_squared() > 1e-6 {
            dir.normalize()
        } else {
            Vec3::X
        }
    }

    fn char_dir(axis: Vec3, offset_dir: Vec3) -> Vec3 {
        let dir = offset_dir.cross(axis);
        if dir.length_squared() > 1e-6 {
            dir.normalize()
        } else {
            Vec3::Y
        }
    }

    fn project_point_to_segment(anchor: Vec3, seg: &CacheTubiSeg) -> Vec3 {
        let axis = seg.end - seg.start;
        let denom = axis.length_squared();
        if denom <= 1e-6 {
            return seg.start;
        }
        let t = (anchor - seg.start).dot(axis) / denom;
        seg.start + axis * t.clamp(0.0, 1.0)
    }

    fn owner_segment(topology: &BranchTopology, anchor: Vec3) -> Option<(usize, &CacheTubiSeg)> {
        topology
            .segments
            .iter()
            .enumerate()
            .min_by(|(_, a), (_, b)| {
                let da = Self::project_point_to_segment(anchor, a).distance_squared(anchor);
                let db = Self::project_point_to_segment(anchor, b).distance_squared(anchor);
                da.partial_cmp(&db).unwrap_or(std::cmp::Ordering::Equal)
            })
    }

    fn linear_hint(
        topology: &BranchTopology,
        start: Vec3,
        end: Vec3,
        label_role: &str,
        offset_level: u32,
        suppress_reason: Option<String>,
    ) -> MbdLayoutHint {
        let anchor = (start + end) * 0.5;
        let owner = Self::owner_segment(topology, anchor);
        let axis = owner
            .map(|(_, seg)| Self::axis_or_default(seg.start, seg.end))
            .unwrap_or_else(|| Self::axis_or_default(start, end));
        let offset_dir = Self::choose_offset_dir(axis);
        let char_dir = Self::char_dir(axis, offset_dir);
        MbdLayoutHint {
            anchor_point: anchor.to_array(),
            primary_axis: axis.to_array(),
            offset_dir: offset_dir.to_array(),
            char_dir: char_dir.to_array(),
            label_role: label_role.to_string(),
            avoid_line_of_sight: true,
            owner_segment_id: owner.map(|(index, seg)| Self::segment_id(seg, index)),
            offset_level,
            suppress_reason,
        }
    }

    fn anchor_hint(
        topology: &BranchTopology,
        anchor: Vec3,
        label_role: &str,
        offset_level: u32,
        suppress_reason: Option<String>,
    ) -> MbdLayoutHint {
        let owner = Self::owner_segment(topology, anchor);
        let axis = owner
            .map(|(_, seg)| Self::axis_or_default(seg.start, seg.end))
            .unwrap_or(Vec3::X);
        let offset_dir = Self::choose_offset_dir(axis);
        let char_dir = Self::char_dir(axis, offset_dir);
        MbdLayoutHint {
            anchor_point: anchor.to_array(),
            primary_axis: axis.to_array(),
            offset_dir: offset_dir.to_array(),
            char_dir: char_dir.to_array(),
            label_role: label_role.to_string(),
            avoid_line_of_sight: true,
            owner_segment_id: owner.map(|(index, seg)| Self::segment_id(seg, index)),
            offset_level,
            suppress_reason,
        }
    }
}

struct BranchMeasurementPlanner<'a> {
    query: &'a ResolvedMbdPipeQuery,
    topology: &'a BranchTopology,
    fitting_elements: &'a [RawFittingElement],
    bends: &'a [MbdBendDto],
}

impl<'a> BranchMeasurementPlanner<'a> {
    fn new(
        query: &'a ResolvedMbdPipeQuery,
        topology: &'a BranchTopology,
        fitting_elements: &'a [RawFittingElement],
        bends: &'a [MbdBendDto],
    ) -> Self {
        Self {
            query,
            topology,
            fitting_elements,
            bends,
        }
    }

    fn build(&self) -> BranchMeasurementOutput {
        let mut output = BranchMeasurementOutput::default();
        output.segments = self.build_segments();
        output.dims = self.build_dims();
        output.cut_tubis = self.build_cut_tubis();
        output.welds = self.build_welds();
        output.slopes = self.build_slopes();
        output.bends = self.bends.to_vec();
        output.fittings = self.build_fittings();
        output.tags = self.build_tags(&output.cut_tubis, &output.fittings);
        output
    }

    fn build_segments(&self) -> Vec<MbdPipeSegmentDto> {
        self.topology
            .segments
            .iter()
            .enumerate()
            .map(|(i, seg)| MbdPipeSegmentDto {
                id: AnnotationLayoutPlanner::segment_id(seg, i),
                refno: seg.refno.to_string(),
                noun: "TUBI".to_string(),
                name: None,
                arrive: Some([seg.start.x, seg.start.y, seg.start.z]),
                leave: Some([seg.end.x, seg.end.y, seg.end.z]),
                length: seg.start.distance(seg.end),
                straight_length: seg.start.distance(seg.end),
                outside_diameter: seg.outside_diameter,
                bore: None,
            })
            .collect()
    }

    fn build_dims(&self) -> Vec<MbdDimDto> {
        let mut dims = Vec::new();
        if self.query.include_dims {
            for (i, seg) in self.topology.segments.iter().enumerate() {
                let length = seg.start.distance(seg.end);
                if length < self.query.dim_min_length {
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
                    layout_hint: self.query.include_layout_hints.then(|| {
                        AnnotationLayoutPlanner::linear_hint(
                            self.topology,
                            seg.start,
                            seg.end,
                            "segment",
                            0,
                            None,
                        )
                    }),
                });
            }
        }

        if self.query.include_port_dims {
            for (i, seg) in self.topology.segments.iter().enumerate() {
                let (start, end) = segment_port_points(seg);
                let length = start.distance(end);
                if length < self.query.dim_min_length {
                    continue;
                }
                dims.push(MbdDimDto {
                    id: format!("dim:port:{}:{i}", seg.refno),
                    kind: MbdDimKind::Port,
                    group_id: None,
                    seq: Some(i as u32),
                    start: start.to_array(),
                    end: end.to_array(),
                    length,
                    text: format_dim_length_text_mm(length),
                    layout_hint: self.query.include_layout_hints.then(|| {
                        AnnotationLayoutPlanner::linear_hint(
                            self.topology,
                            start,
                            end,
                            "port",
                            1,
                            None,
                        )
                    }),
                });
            }
        }

        let mut weld_joints = Vec::new();
        if self.query.include_welds
            || self.query.include_chain_dims
            || self.query.include_overall_dim
        {
            for i in 0..self.topology.segments.len().saturating_sub(1) {
                let seg1 = &self.topology.segments[i];
                let seg2 = &self.topology.segments[i + 1];
                let (p1, p2, dist) = closest_endpoints(seg1.start, seg1.end, seg2.start, seg2.end);
                if dist >= self.query.weld_merge_threshold {
                    continue;
                }
                weld_joints.push(WeldJoint {
                    left_endpoint: p1,
                    right_endpoint: p2,
                    mid: midpoint(p1, p2),
                });
            }
        }

        if self.query.include_chain_dims {
            let ends: Vec<(Vec3, Vec3)> = self
                .topology
                .segments
                .iter()
                .map(|s| (s.start, s.end))
                .collect();
            let chain_pts = build_chain_points_from_ends(&ends, &weld_joints);
            let group_id = Some(format!("chain:{}", self.topology.branch_refno));
            for i in 0..chain_pts.len().saturating_sub(1) {
                let a = chain_pts[i];
                let b = chain_pts[i + 1];
                let length = a.distance(b);
                if length < self.query.dim_min_length {
                    continue;
                }
                dims.push(MbdDimDto {
                    id: format!("dim:chain:{}:{i}", self.topology.branch_refno),
                    kind: MbdDimKind::Chain,
                    group_id: group_id.clone(),
                    seq: Some(i as u32),
                    start: a.to_array(),
                    end: b.to_array(),
                    length,
                    text: format_dim_length_text_mm(length),
                    layout_hint: self.query.include_layout_hints.then(|| {
                        AnnotationLayoutPlanner::linear_hint(self.topology, a, b, "chain", 1, None)
                    }),
                });
            }
        }

        if self.query.include_overall_dim && !self.topology.segments.is_empty() {
            let total: f32 = self
                .topology
                .segments
                .iter()
                .map(|seg| seg.start.distance(seg.end))
                .sum();
            // 总长对应 PML `hpos of branname → tpos of branname`：取首段起点到末段终点，
            // 不走 weld_joints 链条（即使 weld_joints 为空、或者段间坐标不严格相连，也能稳定产出）。
            let first_seg = &self.topology.segments[0];
            let last_seg = &self.topology.segments[self.topology.segments.len() - 1];
            let (a, b) = (first_seg.start, last_seg.end);
            if total >= self.query.dim_min_length {
                dims.push(MbdDimDto {
                    id: format!("dim:overall:{}", self.topology.branch_refno),
                    kind: MbdDimKind::Overall,
                    group_id: Some(format!("overall:{}", self.topology.branch_refno)),
                    seq: None,
                    start: a.to_array(),
                    end: b.to_array(),
                    length: total,
                    text: format_dim_length_text_mm(total),
                    layout_hint: self.query.include_layout_hints.then(|| {
                        AnnotationLayoutPlanner::linear_hint(
                            self.topology,
                            a,
                            b,
                            "overall",
                            2,
                            Some("overall_disabled_by_default".to_string()),
                        )
                    }),
                });
            }
        }

        dims
    }

    fn build_cut_tubis(&self) -> Vec<MbdCutTubiDto> {
        if !self.query.include_cut_tubis {
            return Vec::new();
        }
        self.topology
            .segments
            .iter()
            .enumerate()
            .filter_map(|(i, seg)| {
                let length = seg.start.distance(seg.end);
                if length < self.query.dim_min_length {
                    return None;
                }
                Some(MbdCutTubiDto {
                    id: format!("cut_tubi:{}:{i}", seg.refno),
                    segment_id: AnnotationLayoutPlanner::segment_id(seg, i),
                    refno: seg.refno.to_string(),
                    start: seg.start.to_array(),
                    end: seg.end.to_array(),
                    length,
                    text: format_dim_length_text_mm(length),
                    layout_hint: self.query.include_layout_hints.then(|| {
                        AnnotationLayoutPlanner::linear_hint(
                            self.topology,
                            seg.start,
                            seg.end,
                            "cut_tubi",
                            1,
                            None,
                        )
                    }),
                })
            })
            .collect()
    }

    fn build_welds(&self) -> Vec<MbdWeldDto> {
        if !self.query.include_welds {
            return Vec::new();
        }
        let mut welds = Vec::new();
        let mut field_idx = 0usize;
        for i in 0..self.topology.segments.len().saturating_sub(1) {
            let seg1 = &self.topology.segments[i];
            let seg2 = &self.topology.segments[i + 1];
            let (p1, p2, dist) = closest_endpoints(seg1.start, seg1.end, seg2.start, seg2.end);
            if dist >= self.query.weld_merge_threshold {
                continue;
            }
            field_idx += 1;
            let position = midpoint(p1, p2);
            welds.push(MbdWeldDto {
                id: format!("weld:{}:{i}", self.topology.branch_refno),
                position: position.to_array(),
                weld_type: MbdWeldType::Butt,
                is_shop: false,
                label: format!("M{field_idx}"),
                left_refno: seg1.refno.to_string(),
                right_refno: seg2.refno.to_string(),
                layout_hint: self.query.include_layout_hints.then(|| {
                    AnnotationLayoutPlanner::anchor_hint(self.topology, position, "weld", 2, None)
                }),
            });
        }
        welds
    }

    fn build_slopes(&self) -> Vec<MbdSlopeDto> {
        if !self.query.include_slopes {
            return Vec::new();
        }
        self.topology
            .segments
            .iter()
            .enumerate()
            .filter_map(|(i, seg)| {
                let dx = seg.end.x - seg.start.x;
                let dy = seg.end.y - seg.start.y;
                let dz = seg.end.z - seg.start.z;
                let horizontal = (dx * dx + dy * dy).sqrt();
                if horizontal <= 1e-3 {
                    return None;
                }
                let slope = dz / horizontal;
                let abs_slope = slope.abs();
                if abs_slope < self.query.min_slope || abs_slope > self.query.max_slope {
                    return None;
                }
                Some(MbdSlopeDto {
                    id: format!("slope:{}:{i}", seg.refno),
                    start: seg.start.to_array(),
                    end: seg.end.to_array(),
                    slope,
                    text: format!("slope {:.1}%", abs_slope * 100.0),
                })
            })
            .collect()
    }

    fn build_fittings(&self) -> Vec<MbdFittingDto> {
        if !self.query.include_fittings {
            return Vec::new();
        }
        self.fitting_elements
            .iter()
            .filter_map(|item| {
                let kind = fitting_kind_from_noun(&item.noun);
                if kind == MbdFittingKind::Unknown {
                    return None;
                }
                Some(MbdFittingDto {
                    id: format!("fitting:{}", item.refno),
                    refno: item.refno.to_string(),
                    noun: item.noun.clone(),
                    kind,
                    anchor_point: item.anchor_point.to_array(),
                    angle: item.angle,
                    radius: item.radius,
                    face_center_1: item.face_center_1.map(|v| v.to_array()),
                    face_center_2: item.face_center_2.map(|v| v.to_array()),
                    layout_hint: self.query.include_layout_hints.then(|| {
                        AnnotationLayoutPlanner::anchor_hint(
                            self.topology,
                            item.anchor_point,
                            fitting_label_role(kind),
                            2,
                            None,
                        )
                    }),
                })
            })
            .collect()
    }

    fn build_tags(
        &self,
        cut_tubis: &[MbdCutTubiDto],
        fittings: &[MbdFittingDto],
    ) -> Vec<MbdTagDto> {
        if !self.query.include_tags {
            return Vec::new();
        }
        let mut tags = Vec::new();
        for cut in cut_tubis {
            let start = Vec3::from_array(cut.start);
            let end = Vec3::from_array(cut.end);
            let pos = (start + end) * 0.5;
            tags.push(MbdTagDto {
                id: format!("tag:tubi:{}", cut.refno),
                refno: cut.refno.clone(),
                noun: "TUBI".to_string(),
                role: "tubi".to_string(),
                text: format!("L={}", cut.text),
                position: pos.to_array(),
                layout_hint: self.query.include_layout_hints.then(|| {
                    AnnotationLayoutPlanner::anchor_hint(self.topology, pos, "tag_tubi", 1, None)
                }),
            });
        }
        for fitting in fittings {
            let text = match fitting.kind {
                MbdFittingKind::Elbo | MbdFittingKind::Bend => {
                    match (fitting.angle, fitting.radius) {
                        (Some(angle), Some(radius)) => {
                            format!("{} {:.1}° R{:.0}", fitting.noun, angle, radius)
                        }
                        (Some(angle), None) => format!("{} {:.1}°", fitting.noun, angle),
                        _ => fitting.noun.clone(),
                    }
                }
                _ => fitting.noun.clone(),
            };
            let pos = Vec3::from_array(fitting.anchor_point);
            tags.push(MbdTagDto {
                id: format!("tag:fitting:{}", fitting.refno),
                refno: fitting.refno.clone(),
                noun: fitting.noun.clone(),
                role: fitting_label_role(fitting.kind).to_string(),
                text,
                position: pos.to_array(),
                layout_hint: fitting.layout_hint.clone(),
            });
        }
        tags
    }
}

fn fitting_kind_from_noun(noun: &str) -> MbdFittingKind {
    match noun {
        "ELBO" => MbdFittingKind::Elbo,
        "BEND" => MbdFittingKind::Bend,
        "TEE" => MbdFittingKind::Tee,
        "OLET" => MbdFittingKind::Olet,
        "FLAN" | "FLNG" => MbdFittingKind::Flan,
        _ => MbdFittingKind::Unknown,
    }
}

fn fitting_label_role(kind: MbdFittingKind) -> &'static str {
    match kind {
        MbdFittingKind::Elbo | MbdFittingKind::Bend => "fitting_bend",
        MbdFittingKind::Tee | MbdFittingKind::Olet => "fitting_branch",
        MbdFittingKind::Flan => "fitting_flan",
        MbdFittingKind::Unknown => "fitting",
    }
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
    let query = query.resolve();
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

    // 优先读取预生成 JSON；若命中则直接返回，避免重复实时计算。
    {
        let json_path = get_mbd_output_dir().join(format!("{}.json", input_refno_enum));
        if json_path.exists() {
            match std::fs::read_to_string(&json_path) {
                Ok(content) => match serde_json::from_str::<MbdPipeResponse>(&content) {
                    Ok(mut resp) => {
                        if query.debug {
                            let debug_info =
                                resp.data.as_mut().and_then(|data| data.debug_info.as_mut());
                            if let Some(info) = debug_info {
                                info.notes.push("source=pregenerated_json".to_string());
                                info.notes.push(format!("file={}", json_path.display()));
                            } else if let Some(ref mut data) = resp.data {
                                data.debug_info = Some(MbdPipeDebugInfo {
                                    notes: vec![
                                        "source=pregenerated_json".to_string(),
                                        format!("file={}", json_path.display()),
                                    ],
                                    ..Default::default()
                                });
                            }
                        }
                        println!("[mbd-pipe] 命中预生成 JSON: {}", json_path.display());
                        return json_utf8(resp);
                    }
                    Err(e) => {
                        eprintln!(
                            "[mbd-pipe] 预生成 JSON 反序列化失败（回退实时计算）: {} — {e}",
                            json_path.display()
                        );
                    }
                },
                Err(e) => {
                    eprintln!(
                        "[mbd-pipe] 预生成 JSON 读取失败（回退实时计算）: {} — {e}",
                        json_path.display()
                    );
                }
            }
        }
    }

    // 兼容 HANG：若输入不是可直接产出 tubi_relate 的 branch root，则尝试沿 HREF/TREF
    // 指向对象向上解析到最近的 BRAN/HANG 祖先，再统一走现有 BRAN/HANG 主链。
    let branch_refno = match resolve_effective_branch_refno(input_refno_enum.clone()).await {
        Ok(v) => v,
        Err(_) => input_refno_enum.clone(),
    };

    let (segments, mut debug_info) = match query.source {
        MbdPipeSource::Parquet => {
            match fetch_tubi_segments_from_parquet_with_debug(branch_refno.clone(), query.dbno)
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
                                let d = db_meta()
                                    .get_dbnum_by_refno(branch_refno.clone())
                                    .unwrap_or(0);
                                if d > 0 { Some(d) } else { None }
                            });
                            if let Some(dbnum) = dbno_for_export {
                                tokio::spawn(async move {
                                    if let Err(e) = trigger_async_parquet_export(dbnum).await {
                                        eprintln!("[mbd-pipe] 后台 parquet 导出失败: {e}");
                                    }
                                });
                                db_debug
                                    .notes
                                    .push(format!("已触发后台 parquet 导出 dbnum={dbnum}"));
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
            }
        }
        MbdPipeSource::Db => {
            match fetch_tubi_segments_from_surreal_with_debug(branch_refno.clone()).await {
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
            }
        }
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
                debug_info
                    .notes
                    .push(format!("分支属性填充失败（已忽略）: {e}"));
                (branch_refno.to_string(), BranchAttrsDto::default())
            }
        }
    } else {
        (branch_refno.to_string(), BranchAttrsDto::default())
    };

    debug_info.inferred_dbnum = debug_info.inferred_dbnum.or(query.dbno);
    debug_info.requested_dbno = query.dbno;
    debug_info.requested_batch_id = query.batch_id.clone();
    if branch_refno != input_refno_enum {
        debug_info.notes.push(format!(
            "effective_branch_refno={} (from input={})",
            branch_refno, input_refno_enum
        ));
    }

    match build_mbd_pipe_data_from_segments(
        branch_refno.clone(),
        &query,
        segments,
        branch_name,
        branch_attrs,
        debug_info,
    )
    .await
    {
        Ok(mut data) => {
            data.input_refno = input_refno_enum.to_string();
            if let Some(debug) = data.debug_info.as_mut() {
                debug.notes.push(format!(
                    "stats: segs={} dims={} cut_tubis={} welds={} slopes={} fittings={} tags={} bends={}",
                    data.stats.segments_count,
                    data.stats.dims_count,
                    data.stats.cut_tubis_count,
                    data.stats.welds_count,
                    data.stats.slopes_count,
                    data.stats.fittings_count,
                    data.stats.tags_count,
                    data.stats.bends_count,
                ));
            }
            json_utf8(MbdPipeResponse {
                success: true,
                error_message: None,
                data: Some(data),
            })
        }
        Err(e) => json_utf8(MbdPipeResponse {
            success: false,
            error_message: Some(format!("生成管道标注失败: {e}")),
            data: None,
        }),
    }
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
            PathBuf::from("output/instances").join(dbnum.to_string())
        } else {
            PathBuf::from(format!("output/{project_name}/instances")).join(dbnum.to_string())
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

    // 确定 parquet 输出目录：仅使用 output/{project}/instances/{dbnum}
    let db_option = aios_core::get_db_option();
    let project_name = &db_option.project_name;
    let instances_root = if project_name.is_empty() {
        PathBuf::from("output/instances")
    } else {
        PathBuf::from(format!("output/{project_name}/instances"))
    };
    let instances_dir = instances_root.join(inferred_dbnum.to_string());
    debug.cache_dir = Some(instances_dir.display().to_string());

    let tubings_path = instances_dir.join("tubings.parquet");
    if !tubings_path.exists() {
        anyhow::bail!("tubings parquet 文件不存在: {}", tubings_path.display());
    }
    let transforms_path = instances_dir.join("transforms.parquet");

    // 读取 tubings parquet，按 owner_refno_str 过滤
    let owner_refno_str = branch_refno.to_string();
    let tubings_df = {
        let file = std::fs::File::open(&tubings_path)?;
        let full_df = ParquetReader::new(file).finish()?;
        let mask = full_df
            .column("owner_refno_str")?
            .str()?
            .into_iter()
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
    debug
        .notes
        .push(format!("tubings rows={}", tubings_df.height()));

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
        let hash_set: std::collections::HashSet<&str> =
            trans_hashes.iter().map(|s| s.as_str()).collect();
        let mask = full_trans_df
            .column("trans_hash")?
            .str()?
            .into_iter()
            .map(|opt| opt.map_or(false, |v| hash_set.contains(v)))
            .collect::<BooleanChunked>();
        let trans_df = full_trans_df.filter(&mask)?;
        let mut m: HashMap<String, glam::Mat4> = HashMap::new();
        for i in 0..trans_df.height() {
            let hash = trans_df
                .column("trans_hash")?
                .str()?
                .get(i)
                .unwrap_or_default()
                .to_string();
            let get_f = |name: &str| -> f32 {
                trans_df
                    .column(name)
                    .ok()
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
        debug
            .notes
            .push("transforms.parquet 不存在，使用单位矩阵".to_string());
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
            outside_diameter: None,
        });
    }

    segs.sort_by(|a, b| {
        let ao = a.order.unwrap_or(u32::MAX);
        let bo = b.order.unwrap_or(u32::MAX);
        ao.cmp(&bo)
            .then_with(|| a.refno.to_string().cmp(&b.refno.to_string()))
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

    let (segs, _debug) =
        fetch_tubi_segments_from_cache_with_debug(branch_refno, dbno, batch_id, strict_dbno)
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
            debug.fallback_reason =
                Some("指定 dbno 无数据；cache 仅有 1 个 dbnum，已自动回退".to_string());
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
        let Some(cached) = cache.get_inst_info(active_dbnum, leave_refno).await else {
            continue;
        };
        let Some(ref tubi_data) = cached.info.tubi else {
            continue;
        };
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
                    tubi_start.unwrap_or_else(|| m.transform_point3(Vec3::new(0.0, 0.0, 0.0))),
                    tubi_end.unwrap_or_else(|| m.transform_point3(Vec3::new(0.0, 0.0, 1.0))),
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
                outside_diameter: None,
            },
        );
    }

    let mut segs: Vec<CacheTubiSeg> = merged.into_values().collect();
    segs.sort_by(|a, b| {
        let ao = a.order.unwrap_or(u32::MAX);
        let bo = b.order.unwrap_or(u32::MAX);
        ao.cmp(&bo)
            .then_with(|| a.refno.to_string().cmp(&b.refno.to_string()))
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
        /// 外径（mm），通常存在 TUBI 元件的 pe.aod 或 pe.attrs.AOD；
        /// 任一缺失则回退到 `compute_branch_layout_result` 的 default_od。
        #[serde(default)]
        pub aod: Option<f32>,
    }

    let mut debug = MbdPipeDebugInfo::default();
    debug.notes.push("source=db".to_string());

    let pe_key = branch_refno.to_pe_key();
    // Stage B.2: 顺便试探 TUBI 元件的外径。tubi_relate.in 是 record reference (pe:...),
    // 直接 `in.aod` 走 record join，O(1) 命中，pe 不存在该字段时返回 NONE → Option<f32>=None。
    // 不命中时由 `compute_branch_layout_result` 的 default_od=229 兜住。
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
            id[1] as index,
            in.aod as aod
        FROM tubi_relate:[{pe_key}, 0]..[{pe_key}, ..];
        "#
    );

    let rows: Vec<TubiRelateRow> = SUL_DB.query_take(&sql, 0).await?;
    if rows.is_empty() {
        anyhow::bail!(
            "tubi_relate 无结果（branch_refno={} pe_key={}）",
            branch_refno,
            pe_key
        );
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
            outside_diameter: row.aod,
        });
    }

    segs.sort_by(|a, b| {
        let ao = a.order.unwrap_or(u32::MAX);
        let bo = b.order.unwrap_or(u32::MAX);
        ao.cmp(&bo)
            .then_with(|| a.refno.to_string().cmp(&b.refno.to_string()))
    });

    let found_aod = segs.iter().filter(|s| s.outside_diameter.is_some()).count();
    debug.notes.push(format!(
        "tubi_aod_found={}/{}",
        found_aod,
        segs.len()
    ));

    Ok((segs, debug))
}

fn build_branch_child_element_query(pe_key: &str, nouns: &[&str]) -> String {
    let nouns_sql = nouns
        .iter()
        .map(|noun| format!("'{noun}'"))
        .collect::<Vec<_>>()
        .join(", ");

    format!(
        r#"
        SELECT
            record::id(in) as refno,
            in.noun as noun,
            in.world_trans as world_trans,
            in.ptset[*].pt as ptset_pts
        FROM {pe_key}<-pe_owner
        WHERE in.noun IN [{nouns_sql}];
        "#
    )
}

/// 查询分支下的 BEND/ELBO 元件，返回弯头标注数据
async fn fetch_bend_elements_for_branch(
    branch_refno: RefnoEnum,
    bend_mode: MbdBendMode,
) -> anyhow::Result<Vec<MbdBendDto>> {
    use aios_core::rs_surreal::geometry_query::PlantTransform;
    use aios_core::shape::pdms_shape::RsVec3;
    use aios_core::{SUL_DB, SurrealQueryExt};
    use serde::{Deserialize, Serialize};
    use surrealdb::types::SurrealValue;

    aios_core::init_surreal().await?;

    let pe_key = branch_refno.to_pe_key();

    // 查询 BEND/ELBO 子元件：refno、noun、world_trans（→ work_point）、ptset（→ face_center）
    #[derive(Serialize, Deserialize, Debug, SurrealValue)]
    struct BendRow {
        pub refno: RefnoEnum,
        pub noun: String,
        #[serde(default)]
        pub world_trans: Option<PlantTransform>,
        #[serde(default)]
        pub ptset_pts: Option<Vec<Option<Vec<RsVec3>>>>,
    }

    let sql = build_branch_child_element_query(&pe_key, &["BEND", "ELBO"]);

    let rows: Vec<BendRow> = match SUL_DB.query_take(&sql, 0).await {
        Ok(v) => v,
        Err(e) => {
            eprintln!("[mbd-pipe] fetch_bend_elements 查询失败: {e}");
            return Ok(Vec::new());
        }
    };

    let mut bends: Vec<MbdBendDto> = Vec::with_capacity(rows.len());

    for row in &rows {
        let wt = row.world_trans.clone().unwrap_or_default();
        let m = wt.to_matrix();
        let work_point = m.transform_point3(glam::Vec3::ZERO);

        // ptset_pts: Vec<Vec<RsVec3>> — 每个 ptset entry 的 pt 数组
        // face_center = 每组 pt 的第一个点（如果存在）
        let (fc1, fc2) = if let Some(ref pts_groups) = row.ptset_pts {
            let valid_groups = pts_groups.iter().filter_map(|group| group.as_ref());
            let mut centers = valid_groups.filter_map(|group| group.first()).map(|p| p.0);
            let p1 = centers.next();
            let p2 = centers.next();
            (p1, p2)
        } else {
            (None, None)
        };

        // 获取 ANGL、RADI 属性
        let (angle, radius) = match aios_core::get_named_attmap(row.refno.clone()).await {
            Ok(att) => {
                let angl = att.get_f64("ANGL").map(|v| v as f32);
                let radi = att.get_f64("RADI").map(|v| v as f32);
                (angl, radi)
            }
            Err(_) => (None, None),
        };

        bends.push(MbdBendDto {
            id: format!("bend:{}", row.refno),
            refno: row.refno.to_string(),
            noun: row.noun.clone(),
            angle,
            radius,
            work_point: work_point.to_array(),
            face_center_1: fc1.map(|v| v.to_array()),
            face_center_2: fc2.map(|v| v.to_array()),
        });
    }

    Ok(bends)
}

/// 查询分支下的 TEE/OLET/FLAN 元件，返回第一阶段离散标注目标
async fn fetch_discrete_fitting_elements_for_branch(
    branch_refno: RefnoEnum,
) -> anyhow::Result<Vec<RawFittingElement>> {
    use aios_core::rs_surreal::geometry_query::PlantTransform;
    use aios_core::shape::pdms_shape::RsVec3;
    use aios_core::{SUL_DB, SurrealQueryExt};
    use serde::{Deserialize, Serialize};
    use surrealdb::types::SurrealValue;

    aios_core::init_surreal().await?;

    #[derive(Serialize, Deserialize, Debug, SurrealValue)]
    struct FittingRow {
        pub refno: RefnoEnum,
        pub noun: String,
        #[serde(default)]
        pub world_trans: Option<PlantTransform>,
        #[serde(default)]
        pub ptset_pts: Option<Vec<Option<Vec<RsVec3>>>>,
    }

    let pe_key = branch_refno.to_pe_key();
    let sql = build_branch_child_element_query(&pe_key, &["TEE", "OLET", "FLAN", "FLNG"]);

    let rows: Vec<FittingRow> = match SUL_DB.query_take(&sql, 0).await {
        Ok(v) => v,
        Err(e) => {
            eprintln!("[mbd-pipe] fetch_discrete_fitting_elements 查询失败: {e}");
            return Ok(Vec::new());
        }
    };

    let mut items = Vec::with_capacity(rows.len());
    for row in rows {
        let anchor = row
            .world_trans
            .map(|wt| wt.to_matrix().transform_point3(Vec3::ZERO))
            .unwrap_or(Vec3::ZERO);

        let mut flat_pts = row
            .ptset_pts
            .unwrap_or_default()
            .into_iter()
            .flatten()
            .flat_map(|pts| pts.into_iter())
            .map(|p| p.0)
            .collect::<Vec<_>>();
        flat_pts.sort_by(|a, b| a.x.partial_cmp(&b.x).unwrap_or(std::cmp::Ordering::Equal));

        let face_center_1 = flat_pts.first().copied();
        let face_center_2 = flat_pts.get(1).copied();

        items.push(RawFittingElement {
            refno: row.refno,
            noun: row.noun,
            anchor_point: face_center_1.unwrap_or(anchor),
            face_center_1,
            face_center_2,
            angle: None,
            radius: None,
        });
    }

    Ok(items)
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
        if a.distance(used) > b.distance(used) {
            a
        } else {
            b
        }
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
        // 无焊口时仍然需要走整条 branch 的首尾而不是只报第一段：对应 PML 对 overall
        // 尺寸 "hpos of branname → tpos of branname" 的语义。多段情况下取首段起点和末段终点。
        out.push(ends[0].0);
        for pair in ends.iter().skip(1) {
            out.push(pair.0);
        }
        out.push(ends[ends.len() - 1].1);
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
    let pairs = [(a0, b0), (a0, b1), (a1, b0), (a1, b1)];
    let mut best = (pairs[0].0, pairs[0].1, pairs[0].0.distance(pairs[0].1));
    for (pa, pb) in pairs.into_iter().skip(1) {
        let d = pa.distance(pb);
        if d < best.2 {
            best = (pa, pb, d);
        }
    }
    best
}

// ===== MBD JSON 预生成与导出 =====

/// 导出范围
#[derive(Debug, Clone)]
pub enum MbdExportScope {
    /// 查询该 dbnum 下所有 noun=BRAN/HANG 的 refno
    ByDbnum(u32),
    /// 查询该 refno 的子孙中所有 BRAN/HANG（含自身若为 BRAN）
    ByRefno(RefnoEnum),
    /// 遍历所有已知 dbnum
    AllDbnums,
}

/// 导出统计
#[derive(Debug, Clone, Serialize)]
pub struct MbdExportStats {
    pub total: usize,
    pub success: usize,
    pub failed: Vec<MbdExportFailure>,
    pub elapsed_ms: u64,
    pub output_dir: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct MbdExportFailure {
    pub refno: String,
    pub error: String,
}

/// manifest.json 结构
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MbdManifest {
    pub project: String,
    pub generated_at: String,
    pub count: usize,
    pub branches: Vec<MbdManifestEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MbdManifestEntry {
    pub refno: String,
    pub file: String,
    pub segments: usize,
}

/// API 请求体
#[derive(Debug, Clone, Deserialize)]
struct MbdGenerateRequest {
    dbnum: Option<u32>,
    refno: Option<String>,
}

/// 预生成默认查询参数（全量，source=Db）
fn export_default_query() -> MbdPipeQuery {
    MbdPipeQuery {
        mode: Some(MbdPipeMode::Construction),
        source: MbdPipeSource::Db,
        include_dims: Some(true),
        include_chain_dims: Some(true),
        include_overall_dim: Some(false),
        include_port_dims: Some(false),
        include_welds: Some(true),
        include_slopes: Some(true),
        include_bends: Some(true),
        include_cut_tubis: Some(true),
        include_fittings: Some(true),
        include_tags: Some(true),
        include_layout_hints: Some(true),
        include_branch_attrs: true,
        include_weld_nouns: false,
        debug: false,
        ..Default::default()
    }
}

async fn build_mbd_pipe_data_from_segments(
    branch_refno: RefnoEnum,
    query: &ResolvedMbdPipeQuery,
    segments: Vec<CacheTubiSeg>,
    branch_name: String,
    branch_attrs: BranchAttrsDto,
    debug_info: MbdPipeDebugInfo,
) -> anyhow::Result<MbdPipeData> {
    let topology = BranchTopologyBuilder::build(branch_refno.clone(), segments);

    let bends: Vec<MbdBendDto> =
        if query.include_bends || query.include_fittings || query.include_tags {
            fetch_bend_elements_for_branch(branch_refno.clone(), query.bend_mode)
                .await
                .unwrap_or_default()
        } else {
            Vec::new()
        };

    let mut fitting_elements = if query.include_fittings || query.include_tags {
        fetch_discrete_fitting_elements_for_branch(branch_refno.clone())
            .await
            .unwrap_or_default()
    } else {
        Vec::new()
    };

    for bend in &bends {
        fitting_elements.push(RawFittingElement {
            refno: bend
                .refno
                .parse::<RefnoEnum>()
                .unwrap_or_else(|_| branch_refno.clone()),
            noun: bend.noun.clone(),
            anchor_point: Vec3::from_array(bend.work_point),
            face_center_1: bend.face_center_1.map(Vec3::from_array),
            face_center_2: bend.face_center_2.map(Vec3::from_array),
            angle: bend.angle,
            radius: bend.radius,
        });
    }

    let output = BranchMeasurementPlanner::new(query, &topology, &fitting_elements, &bends).build();
    let stats = MbdPipeStats {
        segments_count: output.segments.len(),
        dims_count: output.dims.len(),
        cut_tubis_count: output.cut_tubis.len(),
        welds_count: output.welds.len(),
        slopes_count: output.slopes.len(),
        fittings_count: output.fittings.len(),
        tags_count: output.tags.len(),
        bends_count: output.bends.len(),
    };

    let mut data = MbdPipeData {
        input_refno: branch_refno.to_string(),
        branch_refno: branch_refno.to_string(),
        branch_name,
        branch_attrs,
        segments: output.segments,
        dims: output.dims,
        cut_tubis: output.cut_tubis,
        welds: output.welds,
        slopes: output.slopes,
        fittings: output.fittings,
        tags: output.tags,
        bends: output.bends,
        stats,
        debug_info: query.debug.then_some(debug_info),
        layout_result: None,
    };

    if matches!(query.mode, MbdPipeMode::LayoutFirst) {
        data.layout_result = Some(compute_branch_layout_result(query, &data));
    }

    Ok(data)
}

/// 把后端的 `MbdPipeData` 子集喂给 `aios_core::mbd::BranchCalculator`，产出完整 `LayoutResult`。
///
/// 单位：所有输入/输出均为 **毫米（mm）**，与 `mbd_pipe_api` 的原始坐标空间一致。
fn compute_branch_layout_result(
    query: &ResolvedMbdPipeQuery,
    data: &MbdPipeData,
) -> aios_core::mbd::LayoutResult {
    use aios_core::mbd::iso_extras::{BendInput, SlopeInput, TagInput, WeldInput};
    use aios_core::mbd::iso_params::{BranchContext, IsoParams, SegmentInput};
    use aios_core::mbd::{BranchCalculator, LayoutRequest, SolveBranchInput};
    use glam::Vec3;

    let od_by_segment: HashMap<String, f32> = data
        .segments
        .iter()
        .filter_map(|s| s.outside_diameter.map(|od| (s.id.clone(), od)))
        .collect();
    // 若 segment 未携带 outside_diameter（B.2 之前普遍如此），用 DN200 常见 OD=229mm 作为兜底，
    // 以满足 PML `offset = od + 1.2*cheight*(n-1)` 第一层 offset 不会坍缩到 100mm。
    // B.2 补齐真实 OD 后，大多数段会直接命中 od_by_segment，此分支只在 surreal 查不到时触发。
    const DEFAULT_OD_MM: f32 = 229.0;
    let default_od = od_by_segment
        .values()
        .copied()
        .next()
        .unwrap_or(DEFAULT_OD_MM);

    let iso_params = IsoParams {
        min_slope: query.min_slope,
        max_slope: query.max_slope,
        consider_pre_next_dir: true,
        look_angle: 60.0,
        cheight: 100.0,
        em4_mode: true,
    };
    let context = BranchContext {
        branch_refno: data.branch_refno.clone(),
        bran_volume_center: estimate_bran_volume_center(&data.segments),
        dim_times: 1,
    };

    let linear_dims: Vec<SegmentInput> = data
        .dims
        .iter()
        .map(|d| {
            let start = Vec3::from_array(d.start);
            let end = Vec3::from_array(d.end);
            let owner_id = d
                .layout_hint
                .as_ref()
                .and_then(|h| h.owner_segment_id.clone())
                .unwrap_or_default();
            let od = od_by_segment.get(&owner_id).copied().unwrap_or(default_od);
            let pipe_dir = normalized_dir_or_x(end - start);
            SegmentInput {
                id: d.id.clone(),
                kind: serde_value_as_str(&serde_json::to_value(d.kind).ok())
                    .unwrap_or_else(|| "segment".to_string()),
                start,
                end,
                pipe_dir,
                od,
                text: d.text.clone(),
                isoline_index: d.seq.map(|s| s as usize),
            }
        })
        .collect();

    let cut_tubis: Vec<SegmentInput> = data
        .cut_tubis
        .iter()
        .map(|c| {
            let start = Vec3::from_array(c.start);
            let end = Vec3::from_array(c.end);
            let od = od_by_segment
                .get(&c.segment_id)
                .copied()
                .unwrap_or(default_od);
            let pipe_dir = normalized_dir_or_x(end - start);
            SegmentInput {
                id: c.id.clone(),
                kind: "cut_tubi".to_string(),
                start,
                end,
                pipe_dir,
                od,
                text: c.text.clone(),
                isoline_index: None,
            }
        })
        .collect();

    let slopes: Vec<SlopeInput> = data
        .slopes
        .iter()
        .map(|s| SlopeInput {
            id: s.id.clone(),
            tubi_start: Vec3::from_array(s.start),
            tubi_end: Vec3::from_array(s.end),
            slope: s.slope,
            od: default_od,
            text: s.text.clone(),
        })
        .collect();

    let welds: Vec<WeldInput> = data
        .welds
        .iter()
        .map(|w| WeldInput {
            id: w.id.clone(),
            position: Vec3::from_array(w.position),
            label: w.label.clone(),
            is_shop: w.is_shop,
            subtitle: None,
        })
        .collect();

    let tags: Vec<TagInput> = data
        .tags
        .iter()
        .map(|t| TagInput {
            id: t.id.clone(),
            position: Vec3::from_array(t.position),
            text: t.text.clone(),
        })
        .collect();

    let bends: Vec<BendInput> = data
        .bends
        .iter()
        .map(|b| BendInput {
            id: b.id.clone(),
            vertex: Vec3::from_array(b.work_point),
            face_center_1: b.face_center_1.map(Vec3::from_array),
            face_center_2: b.face_center_2.map(Vec3::from_array),
            angle_deg: b.angle,
            od: default_od,
            face_texts: [None, None],
            angle_text: format_bend_angle_text(b.angle),
        })
        .collect();

    let sections = BranchCalculator::solve_branch(SolveBranchInput {
        context: &context,
        params: &iso_params,
        linear_dims: &linear_dims,
        cut_tubis: &cut_tubis,
        slopes: &slopes,
        welds: &welds,
        tags: &tags,
        bends: &bends,
    });

    let request = LayoutRequest {
        mode: aios_core::mbd::BranchLayoutMode::LayoutFirst,
        include_chain_dims: query.include_chain_dims,
        include_overall_dim: query.include_overall_dim,
        include_port_dims: query.include_port_dims,
        include_welds: query.include_welds,
        include_slopes: query.include_slopes,
        include_bends: query.include_bends,
        include_cut_tubis: query.include_cut_tubis,
        include_tags: query.include_tags,
        include_fittings: query.include_fittings,
        look_angle: None,
        consider_pre_next_dir: true,
        ignore_line: false,
        auto_text_scale: true,
        min_text_scale: 0.75,
        allow_layer_split: true,
    };

    BranchCalculator::assemble_prelaid_out(&request, sections)
}

fn normalized_dir_or_x(v: glam::Vec3) -> glam::Vec3 {
    let d = v.normalize_or_zero();
    if d.length_squared() < 1e-6 {
        glam::Vec3::X
    } else {
        d
    }
}

fn serde_value_as_str(v: &Option<serde_json::Value>) -> Option<String> {
    v.as_ref()
        .and_then(|x| x.as_str())
        .map(|s| s.to_string())
}

fn format_bend_angle_text(angle: Option<f32>) -> String {
    angle
        .map(|a| format!("{a:.1}°"))
        .unwrap_or_default()
}

/// 粗略估计分支 volume 中心：取所有段 arrive/leave 的平均（忽略 None）。
/// 对应 PML `var !volume volume $!branname; !pos1.midpoint(!pos2)` 的简化版本。
fn estimate_bran_volume_center(segments: &[MbdPipeSegmentDto]) -> glam::Vec3 {
    use glam::Vec3;
    let mut sum = Vec3::ZERO;
    let mut count = 0f32;
    for s in segments {
        if let Some(a) = s.arrive {
            sum += Vec3::from_array(a);
            count += 1.0;
        }
        if let Some(l) = s.leave {
            sum += Vec3::from_array(l);
            count += 1.0;
        }
    }
    if count > 0.0 { sum / count } else { Vec3::ZERO }
}

/// 核心 MBD 数据生成逻辑（不依赖 axum，可被 API handler 和批量导出共用）
pub async fn generate_mbd_data(
    branch_refno: RefnoEnum,
    query: &MbdPipeQuery,
) -> anyhow::Result<MbdPipeData> {
    let query = query.resolve();
    let (segments, mut debug_info) =
        fetch_tubi_segments_from_surreal_with_debug(branch_refno.clone()).await?;

    let (branch_name, branch_attrs) = if query.include_branch_attrs {
        match try_fill_branch_name_and_attrs(branch_refno).await {
            Ok(v) => v,
            Err(e) => {
                debug_info
                    .notes
                    .push(format!("分支属性填充失败（已忽略）: {e}"));
                (branch_refno.to_string(), BranchAttrsDto::default())
            }
        }
    } else {
        (branch_refno.to_string(), BranchAttrsDto::default())
    };

    build_mbd_pipe_data_from_segments(
        branch_refno,
        &query,
        segments,
        branch_name,
        branch_attrs,
        debug_info,
    )
    .await
}

/// 生成单个 BRAN 的 MBD JSON 并写入磁盘
pub async fn export_mbd_json_for_bran(
    branch_refno: RefnoEnum,
    output_dir: &FsPath,
) -> anyhow::Result<(PathBuf, usize)> {
    let query = export_default_query();
    let data = generate_mbd_data(branch_refno.clone(), &query).await?;
    let seg_count = data.segments.len();

    let response = MbdPipeResponse {
        success: true,
        error_message: None,
        data: Some(data),
    };
    let json = serde_json::to_string_pretty(&response)?;

    std::fs::create_dir_all(output_dir)?;
    let file_name = format!("{}.json", branch_refno);
    let file_path = output_dir.join(&file_name);
    std::fs::write(&file_path, json.as_bytes())?;

    Ok((file_path, seg_count))
}

/// 根据 scope 收集 BRAN/HANG refno 列表
async fn collect_bran_refnos_for_scope(scope: &MbdExportScope) -> anyhow::Result<Vec<RefnoEnum>> {
    use aios_core::{SUL_DB, SurrealQueryExt};

    aios_core::init_surreal().await?;

    match scope {
        MbdExportScope::ByDbnum(dbnum) => {
            let sql = "SELECT value id FROM pe WHERE noun IN ['BRAN', 'HANG']";
            let all_refnos: Vec<RefnoEnum> = SUL_DB.query_take(sql, 0).await?;

            let db_meta = crate::data_interface::db_meta_manager::db_meta();
            db_meta.ensure_loaded()?;

            let filtered: Vec<RefnoEnum> = all_refnos
                .into_iter()
                .filter(|r| {
                    db_meta
                        .get_dbnum_by_refno(r.clone())
                        .map_or(false, |d| d == *dbnum)
                })
                .collect();
            Ok(filtered)
        }
        MbdExportScope::ByRefno(refno) => {
            let brans = aios_core::collect_descendant_filter_ids_with_self(
                &[refno.clone()],
                &["BRAN", "HANG"],
                None,
                true,
            )
            .await?;
            Ok(brans)
        }
        MbdExportScope::AllDbnums => {
            let sql = "SELECT value id FROM pe WHERE noun IN ['BRAN', 'HANG']";
            let all_refnos: Vec<RefnoEnum> = SUL_DB.query_take(sql, 0).await?;
            Ok(all_refnos)
        }
    }
}

/// 批量生成 MBD JSON
pub async fn export_mbd_json_batch(
    output_dir: &FsPath,
    scope: MbdExportScope,
) -> anyhow::Result<MbdExportStats> {
    let start = std::time::Instant::now();
    let bran_refnos = collect_bran_refnos_for_scope(&scope).await?;

    println!(
        "📋 MBD 预生成：共 {} 个 BRAN/HANG 待处理 → {}",
        bran_refnos.len(),
        output_dir.display()
    );

    let total = bran_refnos.len();
    let mut success = 0usize;
    let mut failed: Vec<MbdExportFailure> = Vec::new();
    let mut manifest_entries: Vec<MbdManifestEntry> = Vec::new();

    for (idx, refno) in bran_refnos.iter().enumerate() {
        match export_mbd_json_for_bran(refno.clone(), output_dir).await {
            Ok((_path, seg_count)) => {
                success += 1;
                manifest_entries.push(MbdManifestEntry {
                    refno: refno.to_string(),
                    file: format!("{}.json", refno),
                    segments: seg_count,
                });
                if (idx + 1) % 10 == 0 || idx + 1 == total {
                    println!(
                        "  [{}/{}] ✅ {} ({} segments)",
                        idx + 1,
                        total,
                        refno,
                        seg_count
                    );
                }
            }
            Err(e) => {
                failed.push(MbdExportFailure {
                    refno: refno.to_string(),
                    error: e.to_string(),
                });
                if (idx + 1) % 10 == 0 || idx + 1 == total {
                    println!("  [{}/{}] ❌ {} → {}", idx + 1, total, refno, e);
                }
            }
        }
    }

    // 写 manifest.json
    let project_name = aios_core::get_db_option().project_name.clone();
    let manifest = MbdManifest {
        project: project_name,
        generated_at: chrono::Utc::now().to_rfc3339(),
        count: manifest_entries.len(),
        branches: manifest_entries,
    };
    let manifest_path = output_dir.join("manifest.json");
    std::fs::create_dir_all(output_dir)?;
    std::fs::write(&manifest_path, serde_json::to_string_pretty(&manifest)?)?;

    let elapsed_ms = start.elapsed().as_millis() as u64;
    println!(
        "📊 MBD 预生成完成：{}/{} 成功，{} 失败，耗时 {}ms",
        success,
        total,
        failed.len(),
        elapsed_ms
    );

    Ok(MbdExportStats {
        total,
        success,
        failed,
        elapsed_ms,
        output_dir: output_dir.display().to_string(),
    })
}

/// 获取 MBD 输出目录
pub fn get_mbd_output_dir() -> PathBuf {
    let db_option = aios_core::get_db_option();
    let project_name = &db_option.project_name;
    if project_name.is_empty() {
        PathBuf::from("output/mbd")
    } else {
        PathBuf::from(format!("output/{project_name}/mbd"))
    }
}

/// POST /api/mbd/generate handler
async fn post_generate_mbd(Json(req): Json<MbdGenerateRequest>) -> impl IntoResponse {
    let output_dir = get_mbd_output_dir();

    let scope = if let Some(refno_str) = &req.refno {
        match refno_str.parse::<RefnoEnum>() {
            Ok(r) => MbdExportScope::ByRefno(r),
            Err(e) => {
                return json_utf8(serde_json::json!({
                    "success": false,
                    "error": format!("无效的 refno: {e}")
                }));
            }
        }
    } else if let Some(dbnum) = req.dbnum {
        MbdExportScope::ByDbnum(dbnum)
    } else {
        MbdExportScope::AllDbnums
    };

    match export_mbd_json_batch(&output_dir, scope).await {
        Ok(stats) => json_utf8(serde_json::json!({
            "success": true,
            "total": stats.total,
            "success_count": stats.success,
            "failed": stats.failed,
            "elapsed_ms": stats.elapsed_ms,
            "output_dir": stats.output_dir,
        })),
        Err(e) => json_utf8(serde_json::json!({
            "success": false,
            "error": e.to_string()
        })),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{Value, json};

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
            outside_diameter: None,
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
            outside_diameter: None,
        };

        let (a, b) = segment_port_points(&seg);
        assert_eq!(a.to_array(), [2.0, 0.0, 0.0]);
        assert_eq!(b.to_array(), [5.0, 0.0, 0.0]);
    }

    #[test]
    fn test_mbd_pipe_mode_defaults_to_construction() {
        let resolved = MbdPipeQuery::default().resolve();
        assert_eq!(resolved.mode, MbdPipeMode::Construction);
        assert!(resolved.include_dims);
        assert!(resolved.include_chain_dims);
        assert!(!resolved.include_overall_dim);
        assert!(!resolved.include_port_dims);
        assert!(resolved.include_welds);
        assert!(resolved.include_slopes);
        assert!(!resolved.include_bends);
        assert!(resolved.include_cut_tubis);
        assert!(resolved.include_fittings);
        assert!(resolved.include_tags);
        assert!(resolved.include_layout_hints);
    }

    #[test]
    fn test_mbd_pipe_mode_inspection_can_deserialize() {
        let mode: MbdPipeMode = serde_json::from_value(json!("inspection")).unwrap();
        assert_eq!(mode, MbdPipeMode::Inspection);
    }

    #[test]
    fn test_mbd_pipe_mode_layout_first_can_deserialize() {
        let mode: MbdPipeMode = serde_json::from_value(json!("layout_first")).unwrap();
        assert_eq!(mode, MbdPipeMode::LayoutFirst);
    }

    #[test]
    fn test_mbd_pipe_query_resolve_layout_first_defaults_match_construction() {
        let lf = MbdPipeQuery {
            mode: Some(MbdPipeMode::LayoutFirst),
            ..Default::default()
        }
        .resolve();
        let cons = MbdPipeQuery {
            mode: Some(MbdPipeMode::Construction),
            ..Default::default()
        }
        .resolve();
        assert_eq!(lf.mode, MbdPipeMode::LayoutFirst);
        assert_eq!(cons.mode, MbdPipeMode::Construction);
        assert_eq!(lf.include_dims, cons.include_dims);
        assert_eq!(lf.include_chain_dims, cons.include_chain_dims);
        assert_eq!(lf.include_welds, cons.include_welds);
    }

    #[test]
    fn test_mbd_pipe_query_resolve_inspection_defaults() {
        let resolved = MbdPipeQuery {
            mode: Some(MbdPipeMode::Inspection),
            ..Default::default()
        }
        .resolve();

        assert_eq!(resolved.mode, MbdPipeMode::Inspection);
        assert!(resolved.include_dims);
        assert!(!resolved.include_chain_dims);
        assert!(!resolved.include_overall_dim);
        assert!(resolved.include_port_dims);
        assert!(!resolved.include_welds);
        assert!(!resolved.include_slopes);
        assert!(!resolved.include_bends);
        assert!(!resolved.include_cut_tubis);
        assert!(!resolved.include_fittings);
        assert!(!resolved.include_tags);
        assert!(!resolved.include_layout_hints);
    }

    #[test]
    fn test_mbd_pipe_query_resolve_explicit_port_override_on_construction() {
        let resolved = MbdPipeQuery {
            include_port_dims: Some(true),
            ..Default::default()
        }
        .resolve();

        assert_eq!(resolved.mode, MbdPipeMode::Construction);
        assert!(resolved.include_port_dims);
    }

    #[test]
    fn test_mbd_pipe_query_resolve_explicit_chain_override_on_inspection() {
        let resolved = MbdPipeQuery {
            mode: Some(MbdPipeMode::Inspection),
            include_chain_dims: Some(true),
            ..Default::default()
        }
        .resolve();

        assert_eq!(resolved.mode, MbdPipeMode::Inspection);
        assert!(resolved.include_chain_dims);
        assert!(resolved.include_port_dims);
    }

    #[test]
    fn test_mbd_pipe_query_resolve_explicit_new_flags_override_mode_defaults() {
        let resolved = MbdPipeQuery {
            mode: Some(MbdPipeMode::Inspection),
            include_cut_tubis: Some(true),
            include_fittings: Some(true),
            include_tags: Some(true),
            include_layout_hints: Some(true),
            ..Default::default()
        }
        .resolve();

        assert!(resolved.include_cut_tubis);
        assert!(resolved.include_fittings);
        assert!(resolved.include_tags);
        assert!(resolved.include_layout_hints);
    }

    #[test]
    fn test_measurement_planner_separates_cut_tubis_from_dims() {
        let query = MbdPipeQuery::default().resolve();
        let topology = BranchTopologyBuilder::build(
            RefnoEnum::from("24381_145018"),
            vec![
                CacheTubiSeg {
                    refno: RefnoEnum::from("1_1"),
                    arrive_refno: None,
                    order: Some(0),
                    start: Vec3::new(0.0, 0.0, 0.0),
                    end: Vec3::new(100.0, 0.0, 0.0),
                    arrive_axis: None,
                    leave_axis: None,
                    outside_diameter: None,
                },
                CacheTubiSeg {
                    refno: RefnoEnum::from("1_2"),
                    arrive_refno: None,
                    order: Some(1),
                    start: Vec3::new(100.0, 0.0, 0.0),
                    end: Vec3::new(220.0, 0.0, 0.0),
                    arrive_axis: None,
                    leave_axis: None,
                    outside_diameter: None,
                },
            ],
        );

        let output = BranchMeasurementPlanner::new(&query, &topology, &[], &[]).build();

        assert_eq!(output.cut_tubis.len(), 2);
        assert!(output.dims.iter().all(|dim| {
            matches!(
                dim.kind,
                MbdDimKind::Segment | MbdDimKind::Chain | MbdDimKind::Overall | MbdDimKind::Port
            )
        }));
        assert!(output.dims.iter().all(|dim| dim.id.starts_with("dim:")));
        assert!(
            output
                .cut_tubis
                .iter()
                .all(|item| item.id.starts_with("cut_tubi:"))
        );
    }

    #[test]
    fn test_measurement_planner_generates_fittings_and_tags_with_layout_hints() {
        let query = MbdPipeQuery::default().resolve();
        let topology = BranchTopologyBuilder::build(
            RefnoEnum::from("24381_145018"),
            vec![CacheTubiSeg {
                refno: RefnoEnum::from("1_1"),
                arrive_refno: None,
                order: Some(0),
                start: Vec3::new(0.0, 0.0, 0.0),
                end: Vec3::new(100.0, 0.0, 0.0),
                arrive_axis: None,
                leave_axis: None,
                outside_diameter: None,
            }],
        );
        let fittings = vec![
            RawFittingElement {
                refno: RefnoEnum::from("2_1"),
                noun: "TEE".to_string(),
                anchor_point: Vec3::new(50.0, 0.0, 0.0),
                face_center_1: None,
                face_center_2: None,
                angle: None,
                radius: None,
            },
            RawFittingElement {
                refno: RefnoEnum::from("2_2"),
                noun: "FLAN".to_string(),
                anchor_point: Vec3::new(100.0, 0.0, 0.0),
                face_center_1: None,
                face_center_2: None,
                angle: None,
                radius: None,
            },
        ];

        let output = BranchMeasurementPlanner::new(&query, &topology, &fittings, &[]).build();

        assert_eq!(output.fittings.len(), 2);
        assert!(output.tags.iter().any(|tag| tag.noun == "TEE"));
        assert!(output.tags.iter().any(|tag| tag.noun == "FLAN"));
        assert!(output.fittings.iter().all(|item| {
            item.layout_hint
                .as_ref()
                .and_then(|hint| hint.owner_segment_id.clone())
                .is_some()
        }));
    }

    #[test]
    fn test_bend_query_uses_direct_branch_children() {
        let sql = build_branch_child_element_query("pe:`24381_145018`", &["BEND", "ELBO"]);

        assert!(sql.contains("FROM pe:`24381_145018`<-pe_owner"));
        assert!(!sql.contains("->inst_relate"));
        assert!(sql.contains("record::id(in) as refno"));
        assert!(sql.contains("in.world_trans as world_trans"));
        assert!(sql.contains("in.ptset[*].pt as ptset_pts"));
    }

    #[test]
    fn test_discrete_fitting_query_uses_direct_branch_children() {
        let sql =
            build_branch_child_element_query("pe:`24381_145018`", &["TEE", "OLET", "FLAN", "FLNG"]);

        assert!(sql.contains("FROM pe:`24381_145018`<-pe_owner"));
        assert!(!sql.contains("->inst_relate"));
        assert!(sql.contains("WHERE in.noun IN ['TEE', 'OLET', 'FLAN', 'FLNG']"));
    }

    #[test]
    fn test_stats_serialization_exposes_phase1_counts() {
        let stats = MbdPipeStats {
            segments_count: 1,
            dims_count: 2,
            cut_tubis_count: 3,
            welds_count: 4,
            slopes_count: 5,
            fittings_count: 6,
            tags_count: 7,
            bends_count: 8,
        };
        let json = serde_json::to_value(stats).expect("stats serialize");
        assert!(json.get("cut_tubis_count").is_some());
        assert!(json.get("fittings_count").is_some());
        assert!(json.get("tags_count").is_some());
    }

    #[test]
    fn test_data_serialization_exposes_phase1_collections() {
        let data = MbdPipeData {
            input_refno: "24381_145018".to_string(),
            branch_refno: "24381_145018".to_string(),
            branch_name: "demo".to_string(),
            branch_attrs: BranchAttrsDto::default(),
            segments: Vec::new(),
            dims: Vec::new(),
            cut_tubis: Vec::new(),
            welds: Vec::new(),
            slopes: Vec::new(),
            fittings: Vec::new(),
            tags: Vec::new(),
            bends: Vec::new(),
            stats: MbdPipeStats::default(),
            debug_info: Some(MbdPipeDebugInfo::default()),
            layout_result: None,
        };
        let json = serde_json::to_value(data).expect("data serialize");
        assert!(json.get("cut_tubis").is_some());
        assert!(json.get("fittings").is_some());
        assert!(json.get("tags").is_some());
    }

    #[test]
    fn test_debug_notes_use_phase1_stats_format() {
        let debug = MbdPipeDebugInfo {
            notes: vec![
                "stats: segs=1 dims=2 cut_tubis=3 welds=4 slopes=5 fittings=6 tags=7 bends=8"
                    .to_string(),
            ],
            ..Default::default()
        };
        let json = serde_json::to_value(debug).expect("debug serialize");
        let notes = json
            .get("notes")
            .and_then(Value::as_array)
            .expect("notes array");
        let last = notes
            .last()
            .and_then(Value::as_str)
            .expect("last note string");
        assert!(last.contains("cut_tubis="));
        assert!(last.contains("fittings="));
        assert!(last.contains("tags="));
    }
}
