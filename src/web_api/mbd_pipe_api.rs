//! MBD 管道标注 API（首期：管道分支 BRAN/HANG）
//!
//! 目标：为 plant3d-web 提供“管道 MBD 标注”所需的结构化数据（段/尺寸/焊缝/坡度）。
//! 说明：本接口采用“后端提供语义点位 + 前端做屏幕布局/避让”的分层方式，便于渐进式对齐 MBD(PML)。

use std::collections::HashMap;
use std::path::{Path as FsPath, PathBuf};

use aios_core::RefnoEnum;
use axum::{
    Router,
    extract::{Path, Query},
    response::IntoResponse,
    routing::get,
    Json,
};
use glam::Vec3;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct MbdPipeQuery {
    /// dbno（可选；若不传则尝试从 output/scene_tree/db_meta_info.json 推导）
    pub dbno: Option<u32>,
    /// foyer instance_cache 的 batch_id（可选；若不传则默认按 latest）
    pub batch_id: Option<String>,
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
            dbno: None,
            batch_id: None,
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

    // cache-only 约定：当前接口以“输入即 BRAN/HANG refno”为前提，不回退 SurrealDB 做祖先解析。
    // plant3d-web 的测试路由与面板逻辑也是以分支 refno 为输入。
    let branch_refno = input_refno_enum.clone();

    let segments = match fetch_tubi_segments_from_cache(
        branch_refno.clone(),
        query.dbno,
        query.batch_id.as_deref(),
    )
    .await
    {
        Ok(v) => v,
        Err(e) => {
            return Json(MbdPipeResponse {
                success: false,
                error_message: Some(format!("从 foyer cache 读取分支管段失败: {e}")),
                data: None,
            });
        }
    };

    let branch_name = branch_refno.to_string();
    let branch_attrs = BranchAttrsDto::default();

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
                start: [seg.start.x, seg.start.y, seg.start.z],
                end: [seg.end.x, seg.end.y, seg.end.z],
                length,
                text: format!("{:.0}", length),
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
            if seg1.end.distance(seg2.start) >= query.weld_merge_threshold {
                continue;
            }

            let weld_type = determine_weld_type(Some("TUBI"), Some("TUBI"));

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
                position: [seg1.end.x, seg1.end.y, seg1.end.z],
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

async fn fetch_tubi_segments_from_cache(
    branch_refno: RefnoEnum,
    dbno: Option<u32>,
    batch_id: Option<&str>,
) -> anyhow::Result<Vec<CacheTubiSeg>> {
    use crate::data_interface::db_meta_manager::db_meta;
    use crate::fast_model::instance_cache::InstanceCacheManager;

    let dbnum = if let Some(dbno) = dbno {
        dbno
    } else {
        db_meta().ensure_loaded()?;
        db_meta().get_dbnum_by_refno(branch_refno).unwrap_or(0)
    };
    if dbnum == 0 {
        anyhow::bail!("无法推导 dbno（请传 dbno 或先生成 output/scene_tree/db_meta_info.json）");
    }

    // 运行时约定：
    // - 若 FOYER_CACHE_DIR 指定，则优先使用
    // - 否则优先尝试项目内默认输出目录（AvevaMarineSample），再回退到 output/instance_cache
    let cache_dir = std::env::var("FOYER_CACHE_DIR")
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

    let cache = InstanceCacheManager::new(&cache_dir).await?;
    let branch_u64 = branch_refno.refno();
    let mut active_dbnum = dbnum;
    let mut batch_ids = cache.list_batches(active_dbnum);
    if batch_ids.is_empty() {
        // 兼容：前端传入的 dbno 可能是“db_meta 的 dbnum”（例如 7997），
        // 但 instance_cache 的 key 可能是“本次解析/缓存生成的 db 文件编号”（例如 1112）。
        // 因此当指定 dbno 无批次时，尝试回退到 cache 里实际存在的 dbnum。
        let candidates = cache.list_dbnums();
        if candidates.len() == 1 {
            active_dbnum = candidates[0];
            batch_ids = cache.list_batches(active_dbnum);
        } else {
            'outer: for cand in candidates {
                let ids = cache.list_batches(cand);
                if ids.is_empty() {
                    continue;
                }
                // 仅探测最新少量 batch，避免全量扫描
                for id in ids.iter().rev().take(3) {
                    let Some(batch) = cache.get(cand, id).await else { continue };
                    if batch
                        .inst_tubi_map
                        .values()
                        .any(|info| info.owner_refno.refno() == branch_u64)
                    {
                        active_dbnum = cand;
                        batch_ids = ids;
                        break 'outer;
                    }
                }
            }
        }
    }
    if batch_ids.is_empty() {
        anyhow::bail!(
            "instance_cache 无批次数据：dbno={} dir={}（且回退失败）",
            dbnum,
            cache_dir.display()
        );
    }

    fn parse_seq(id: &str) -> Option<u64> {
        id.rsplit('_').next()?.parse().ok()
    }

    // 以“截至 batch_id 的快照”语义读取（与 ptset_api 一致）
    let target_seq = batch_id.and_then(parse_seq);
    let mut ids_with_seq: Vec<(u64, String)> = batch_ids
        .drain(..)
        .filter_map(|id| Some((parse_seq(&id)?, id)))
        .collect();
    ids_with_seq.sort_by_key(|(seq, _)| *seq);
    if let Some(t) = target_seq {
        ids_with_seq.retain(|(seq, _)| *seq <= t);
    }
    if ids_with_seq.is_empty() {
        anyhow::bail!("未找到可用 batch（dbno={} batch_id={:?}）", dbnum, batch_id);
    }

    // 合并快照：后来的 batch 覆盖较早的段
    let mut merged: HashMap<RefnoEnum, CacheTubiSeg> = HashMap::new();
    for (_, id) in ids_with_seq {
        let Some(batch) = cache.get(active_dbnum, &id).await else { continue };
        for (leave_refno, info) in &batch.inst_tubi_map {
            if info.owner_refno.refno() != branch_u64 {
                continue;
            }
            // cache 里 tubi_start_pt/tubi_end_pt 可能未写入（或被裁剪），此时用 tubi 的 world_transform
            // 将 unit cylinder 的端点 (0,0,0)-(0,0,1) 变换到世界坐标，作为稳定兜底。
            let (start, end) = match (info.tubi_start_pt, info.tubi_end_pt) {
                (Some(s), Some(e)) => (s, e),
                _ => {
                    let wt = info.get_ele_world_transform();
                    let m = wt.to_matrix();
                    (
                        info.tubi_start_pt
                            .unwrap_or_else(|| m.transform_point3(Vec3::new(0.0, 0.0, 0.0))),
                        info.tubi_end_pt
                            .unwrap_or_else(|| m.transform_point3(Vec3::new(0.0, 0.0, 1.0))),
                    )
                }
            };
            merged.insert(
                *leave_refno,
                CacheTubiSeg {
                    refno: *leave_refno,
                    arrive_refno: info.tubi_arrive_refno,
                    order: info.tubi_index,
                    start,
                    end,
                },
            );
        }
    }

    let mut segs: Vec<CacheTubiSeg> = merged.into_values().collect();
    segs.sort_by(|a, b| {
        let ao = a.order.unwrap_or(u32::MAX);
        let bo = b.order.unwrap_or(u32::MAX);
        ao.cmp(&bo).then_with(|| a.refno.to_string().cmp(&b.refno.to_string()))
    });
    Ok(segs)
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

