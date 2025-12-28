//! 管道标注 API 模块
//!
//! 提供管道分支（BRAN）的工程标注数据接口

use aios_core::{
    RefnoEnum,
    rs_surreal::pipeline::PipelineQueryService,
};
use axum::{
    Router,
    extract::Path,
    response::IntoResponse,
    routing::get,
    Json,
};
use serde::{Deserialize, Serialize};

/// 标注命令类型
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum AnnotationCommand {
    /// 尺寸线
    DimensionLine {
        start: [f32; 3],
        end: [f32; 3],
        offset: f32,
        text: String,
    },
    /// 文字标签
    TextLabel {
        position: [f32; 3],
        text: String,
        leader_end: Option<[f32; 3]>,
    },
    /// 焊缝符号
    WeldSymbol {
        position: [f32; 3],
        weld_type: u8,
    },
    /// 支吊架符号
    SupportSymbol {
        position: [f32; 3],
        support_type: String,
    },
    /// 坡度标注
    SlopeAnnotation {
        start: [f32; 3],
        end: [f32; 3],
        slope_value: f32,
    },
}

/// 标注数据响应
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AnnotationResponse {
    pub success: bool,
    pub error_message: Option<String>,
    pub data: Option<AnnotationData>,
}

/// 标注数据
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AnnotationData {
    pub refno: String,
    pub name: String,
    pub segments_count: usize,
    pub welds_count: usize,
    pub slopes_count: usize,
    pub commands: Vec<AnnotationCommand>,
}

/// 创建管道标注路由
pub fn create_pipeline_annotation_routes() -> Router {
    Router::new()
        .route("/annotation/{refno}", get(get_pipeline_annotation))
}

/// 获取管道分支的标注数据
async fn get_pipeline_annotation(
    Path(refno): Path<String>,
) -> impl IntoResponse {
    // 解析 refno
    let refno_enum = match refno.parse::<RefnoEnum>() {
        Ok(r) => r,
        Err(e) => {
            return Json(AnnotationResponse {
                success: false,
                error_message: Some(format!("无效的 refno: {}", e)),
                data: None,
            });
        }
    };

    // 查询管段数据
    let segments = match PipelineQueryService::fetch_branch_segments(refno_enum.clone()).await {
        Ok(s) => s,
        Err(e) => {
            return Json(AnnotationResponse {
                success: false,
                error_message: Some(format!("查询失败: {}", e)),
                data: None,
            });
        }
    };

    // 获取分支名称
    let name = segments.first()
        .and_then(|s| s.attrs.get("NAME"))
        .and_then(|v| {
            if let aios_core::NamedAttrValue::StringType(s) = v {
                Some(s.clone())
            } else {
                None
            }
        })
        .unwrap_or_else(|| refno.clone());

    // 生成标注命令
    let mut commands = Vec::new();
    let mut welds_count = 0;
    let mut slopes_count = 0;

    // 1. 为每个等轴测线段生成尺寸线
    for seg in &segments {
        if let (Some(arrive), Some(leave)) = (seg.arrive, seg.leave) {
            let length = seg.length;
            if length > 1.0 {
                commands.push(AnnotationCommand::DimensionLine {
                    start: [arrive.world_pos.x, arrive.world_pos.y, arrive.world_pos.z],
                    end: [leave.world_pos.x, leave.world_pos.y, leave.world_pos.z],
                    offset: 50.0,
                    text: format!("{:.0}", length),
                });
            }
        }
    }

    // 2. 识别焊缝
    for i in 0..segments.len().saturating_sub(1) {
        let seg1 = &segments[i];
        let seg2 = &segments[i + 1];

        if let (Some(leave1), Some(arrive2)) = (seg1.leave, seg2.arrive) {
            if leave1.world_pos.distance(arrive2.world_pos) < 1.0 {
                let weld_type = determine_weld_type(seg1, seg2);
                commands.push(AnnotationCommand::WeldSymbol {
                    position: [leave1.world_pos.x, leave1.world_pos.y, leave1.world_pos.z],
                    weld_type,
                });
                welds_count += 1;
            }
        }
    }

    // 3. 计算坡度
    for seg in &segments {
        if let (Some(arrive), Some(leave)) = (seg.arrive, seg.leave) {
            let dx = leave.world_pos.x - arrive.world_pos.x;
            let dy = leave.world_pos.y - arrive.world_pos.y;
            let dz = leave.world_pos.z - arrive.world_pos.z;
            let horizontal_dist = (dx * dx + dy * dy).sqrt();

            if horizontal_dist > 10.0 {
                let slope = dz.abs() / horizontal_dist;
                if slope >= 0.005 && slope < 1.0 {
                    commands.push(AnnotationCommand::SlopeAnnotation {
                        start: [arrive.world_pos.x, arrive.world_pos.y, arrive.world_pos.z],
                        end: [leave.world_pos.x, leave.world_pos.y, leave.world_pos.z],
                        slope_value: slope,
                    });
                    slopes_count += 1;
                }
            }
        }
    }

    Json(AnnotationResponse {
        success: true,
        error_message: None,
        data: Some(AnnotationData {
            refno,
            name,
            segments_count: segments.len(),
            welds_count,
            slopes_count,
            commands,
        }),
    })
}

/// 判断焊缝类型
fn determine_weld_type(seg1: &aios_core::rs_surreal::pipeline::PipelineSegmentRecord, seg2: &aios_core::rs_surreal::pipeline::PipelineSegmentRecord) -> u8 {
    let noun1 = seg1.noun_raw.as_deref().unwrap_or("");
    let noun2 = seg2.noun_raw.as_deref().unwrap_or("");

    // 承插焊
    if noun1.contains("SW") || noun2.contains("SW") {
        return 2;
    }

    // 角焊 (法兰)
    if noun1.contains("FLAN") || noun2.contains("FLAN") {
        return 1;
    }

    // 默认对接焊
    0
}
