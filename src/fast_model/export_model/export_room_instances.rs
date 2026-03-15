//! 房间实例导出模块
//!
//! 导出房间计算结果为 JSON 格式：
//! - `room_relations.json`: 房间号 → 构件列表的简单映射
//! - `room_geometries.json`: 房间 AABB + 面板几何实例
//!
//! ## 使用方式
//!
//! ```bash
//! cargo run -- export-room-instances --output ./output --verbose
//! ```

use std::collections::HashMap;
use std::path::Path;

use aios_core::{RefnoEnum, SurrealQueryExt, model_primary_db};
use anyhow::{Context, Result};
use chrono::{SecondsFormat, Utc};
use parry3d::bounding_volume::Aabb;
use parry3d::math::Point;
use serde::{Deserialize, Serialize};
use serde_json::json;
use surrealdb::types::{self as surrealdb_types, SurrealValue};
use tracing::{debug, info, warn};

/// Shared JSON fixture contract for post-compute room validation.
#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
pub struct RoomComputeValidationFixture {
    pub description: String,
    pub test_cases: Vec<RoomComputeValidationCase>,
}

/// One room validation case from `verification/room/compute/room_compute_validation.json`.
#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
pub struct RoomComputeValidationCase {
    pub case_id: String,
    pub description: String,
    pub room_number: String,
    pub panel_refno: String,
    pub expected_components: Vec<String>,
    #[serde(default)]
    pub notes: String,
}

impl RoomComputeValidationFixture {
    pub fn load_from_path(path: &Path) -> Result<Self> {
        let content = std::fs::read_to_string(path)
            .with_context(|| format!("读取验证 fixture 失败: {}", path.display()))?;
        serde_json::from_str(&content)
            .with_context(|| format!("解析验证 fixture JSON 失败: {}", path.display()))
    }
}

// ============================================================================
// 数据结构定义
// ============================================================================

/// AABB JSON 格式
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AabbJson {
    pub min: [f64; 3],
    pub max: [f64; 3],
}

impl From<Aabb> for AabbJson {
    fn from(aabb: Aabb) -> Self {
        Self {
            min: [aabb.mins.x as f64, aabb.mins.y as f64, aabb.mins.z as f64],
            max: [aabb.maxs.x as f64, aabb.maxs.y as f64, aabb.maxs.z as f64],
        }
    }
}

impl AabbJson {
    /// 合并两个 AABB
    pub fn merge(&self, other: &AabbJson) -> AabbJson {
        AabbJson {
            min: [
                self.min[0].min(other.min[0]),
                self.min[1].min(other.min[1]),
                self.min[2].min(other.min[2]),
            ],
            max: [
                self.max[0].max(other.max[0]),
                self.max[1].max(other.max[1]),
                self.max[2].max(other.max[2]),
            ],
        }
    }
}

/// 房间关系数据（简单映射）
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RoomRelationsData {
    pub version: u32,
    pub generated_at: String,
    /// 房间号 → 构件 refno 列表
    pub rooms: HashMap<String, Vec<String>>,
}

/// 房间几何数据
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RoomGeometriesData {
    pub version: u32,
    pub generated_at: String,
    pub rooms: Vec<RoomGeometryGroup>,
}

/// 单个房间的几何数据
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RoomGeometryGroup {
    pub room_num: String,
    pub room_refno: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub aabb: Option<AabbJson>,
    pub panels: Vec<RoomPanel>,
}

/// 房间面板
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RoomPanel {
    pub refno: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub aabb: Option<AabbJson>,
    pub instances: Vec<PanelInstance>,
}

/// 面板几何实例
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PanelInstance {
    pub geo_hash: String,
    pub geo_transform: Vec<f32>,
}

/// 导出统计信息
#[derive(Debug, Clone, Default)]
pub struct RoomExportStats {
    pub total_rooms: usize,
    pub total_panels: usize,
    pub total_components: usize,
    pub export_time_ms: u64,
}

// ============================================================================
// 数据库查询结构
// ============================================================================

/// room_relate 查询结果
#[derive(Debug, Clone, Deserialize, SurrealValue)]
pub struct RoomRelateRecord {
    /// 面板 refno (in)
    pub panel_refno: RefnoEnum,
    /// 构件 refno (out)
    pub component_refno: RefnoEnum,
    /// 房间号
    pub room_num: String,
}

/// room_panel_relate 查询结果
#[derive(Debug, Clone, Deserialize, SurrealValue)]
pub struct RoomPanelRecord {
    /// 房间 refno (in)
    pub room_refno: RefnoEnum,
    /// 面板 refno (out)
    pub panel_refno: RefnoEnum,
    /// 房间号
    pub room_num: String,
}

/// 面板几何实例查询结果
#[derive(Debug, Clone, Deserialize, SurrealValue)]
struct PanelGeomQuery {
    refno: RefnoEnum,
    geo_hash: String,
    world_trans: Option<aios_core::PlantTransform>,
    world_aabb: Option<aios_core::types::PlantAabb>,
}

// ============================================================================
// 查询函数
// ============================================================================

fn build_query_room_relations_sql() -> &'static str {
    r#"
        SELECT
            in as panel_refno,
            out as component_refno,
            room_num
        FROM room_relate
    "#
}

fn build_query_room_panel_relations_sql() -> &'static str {
    r#"
        SELECT
            in as room_refno,
            out as panel_refno,
            room_num
        FROM room_panel_relate
    "#
}

/// 查询所有 room_relate 关系
async fn query_room_relations() -> Result<Vec<RoomRelateRecord>> {
    let records: Vec<RoomRelateRecord> = model_primary_db()
        .query_take(build_query_room_relations_sql(), 0)
        .await
        .context("查询 room_relate 失败")?;

    Ok(records)
}

/// 查询所有 room_panel_relate 关系
async fn query_room_panel_relations() -> Result<Vec<RoomPanelRecord>> {
    let records: Vec<RoomPanelRecord> = model_primary_db()
        .query_take(build_query_room_panel_relations_sql(), 0)
        .await
        .context("查询 room_panel_relate 失败")?;

    Ok(records)
}

pub async fn query_room_relations_for_verify() -> Result<Vec<RoomRelateRecord>> {
    query_room_relations().await
}

pub async fn query_room_panel_relations_for_verify() -> Result<Vec<RoomPanelRecord>> {
    query_room_panel_relations().await
}

/// 查询面板的几何实例
async fn query_panel_geometries(panel_refnos: &[RefnoEnum]) -> Result<Vec<PanelGeomQuery>> {
    if panel_refnos.is_empty() {
        return Ok(Vec::new());
    }

    let pe_keys: Vec<String> = panel_refnos.iter().map(|r| r.to_pe_key()).collect();
    let pe_list = pe_keys.join(",");

    let sql = format!(
        r#"
        SELECT
            in as refno,
            out.insts[0].geo_hash as geo_hash,
            (
                SELECT VALUE world_trans.d
                FROM pe_transform
                WHERE id = type::record('pe_transform', record::id(in))
                LIMIT 1
            )[0] as world_trans,
            type::record('inst_relate_aabb', record::id(in)).aabb_id.d as world_aabb
        FROM [{}]->inst_relate
        "#,
        pe_list
    );

    let records: Vec<PanelGeomQuery> = model_primary_db()
        .query_take(&sql, 0)
        .await
        .unwrap_or_default();

    Ok(records)
}

// ============================================================================
// 导出函数
// ============================================================================

/// 导出房间关系数据 (room_relations.json)
///
/// 输出格式：
/// ```json
/// {
///   "version": 1,
///   "generated_at": "...",
///   "rooms": {
///     "A123": ["17496_170848", "17496_170849", ...],
///     "B456": ["17496_170850", ...]
///   }
/// }
/// ```
pub async fn export_room_relations(output_path: &Path, verbose: bool) -> Result<RoomExportStats> {
    let start_time = std::time::Instant::now();

    if verbose {
        info!("🚀 开始导出房间关系数据...");
    }

    // 1. 查询 room_relate 关系
    let relations = query_room_relations().await?;

    if verbose {
        info!("   - 查询到 {} 条 room_relate 记录", relations.len());
    }

    // 2. 按房间号分组
    let mut rooms: HashMap<String, Vec<String>> = HashMap::new();

    for record in &relations {
        let refno_str = record.component_refno.to_string();
        rooms
            .entry(record.room_num.clone())
            .or_default()
            .push(refno_str);
    }

    // 3. 去重
    for refnos in rooms.values_mut() {
        refnos.sort();
        refnos.dedup();
    }

    let total_rooms = rooms.len();
    let total_components: usize = rooms.values().map(|v| v.len()).sum();

    if verbose {
        info!(
            "   - 共 {} 个房间, {} 个构件",
            total_rooms, total_components
        );
    }

    // 4. 构建输出数据
    let data = RoomRelationsData {
        version: 1,
        generated_at: Utc::now().to_rfc3339_opts(SecondsFormat::Millis, true),
        rooms,
    };

    // 5. 写入文件
    let json_content =
        serde_json::to_string_pretty(&data).context("序列化 room_relations.json 失败")?;

    std::fs::write(output_path, &json_content)
        .with_context(|| format!("写入文件失败: {}", output_path.display()))?;

    let export_time_ms = start_time.elapsed().as_millis() as u64;

    if verbose {
        info!("✅ 房间关系导出完成: {}", output_path.display());
        info!("   - 耗时: {} ms", export_time_ms);
    }

    Ok(RoomExportStats {
        total_rooms,
        total_panels: 0,
        total_components,
        export_time_ms,
    })
}

/// 导出房间几何数据 (room_geometries.json)
pub async fn export_room_geometries(output_path: &Path, verbose: bool) -> Result<RoomExportStats> {
    let start_time = std::time::Instant::now();

    if verbose {
        info!("🚀 开始导出房间几何数据...");
    }

    // 1. 查询 room_panel_relate 关系
    let panel_relations = query_room_panel_relations().await?;

    if verbose {
        info!(
            "   - 查询到 {} 条 room_panel_relate 记录",
            panel_relations.len()
        );
    }

    // 2. 按房间号分组面板
    let mut room_panels: HashMap<String, Vec<(RefnoEnum, RefnoEnum)>> = HashMap::new();
    for record in &panel_relations {
        room_panels
            .entry(record.room_num.clone())
            .or_default()
            .push((record.room_refno, record.panel_refno));
    }

    // 3. 收集所有面板 refno
    let all_panel_refnos: Vec<RefnoEnum> = panel_relations.iter().map(|r| r.panel_refno).collect();

    // 4. 查询面板几何数据
    let panel_geoms = query_panel_geometries(&all_panel_refnos).await?;

    if verbose {
        info!("   - 查询到 {} 条面板几何记录", panel_geoms.len());
    }

    // 5. 构建面板几何映射
    let panel_geom_map: HashMap<RefnoEnum, Vec<&PanelGeomQuery>> = {
        let mut map: HashMap<RefnoEnum, Vec<&PanelGeomQuery>> = HashMap::new();
        for geom in &panel_geoms {
            map.entry(geom.refno).or_default().push(geom);
        }
        map
    };

    // 6. 构建房间几何数据
    let mut rooms: Vec<RoomGeometryGroup> = Vec::new();
    let mut total_panels = 0;

    for (room_num, panels) in &room_panels {
        let room_refno = panels
            .first()
            .map(|(r, _)| r.to_string())
            .unwrap_or_default();

        let mut room_panels_data: Vec<RoomPanel> = Vec::new();
        let mut room_aabb: Option<AabbJson> = None;

        for (_, panel_refno) in panels {
            let panel_instances = build_panel_instances(*panel_refno, &panel_geom_map);
            let panel_aabb = panel_geom_map.get(panel_refno).and_then(|geoms| {
                geoms.iter().find_map(|geom| geom.world_aabb.as_ref()).map(|plant_aabb| {
                    let inner_aabb = &plant_aabb.0;
                    AabbJson {
                        min: [
                            inner_aabb.mins.x as f64,
                            inner_aabb.mins.y as f64,
                            inner_aabb.mins.z as f64,
                        ],
                        max: [
                            inner_aabb.maxs.x as f64,
                            inner_aabb.maxs.y as f64,
                            inner_aabb.maxs.z as f64,
                        ],
                    }
                })
            });

            // 合并到房间 AABB
            if let Some(ref p_aabb) = panel_aabb {
                room_aabb = Some(match room_aabb {
                    Some(r_aabb) => r_aabb.merge(p_aabb),
                    None => p_aabb.clone(),
                });
            }

            room_panels_data.push(RoomPanel {
                refno: panel_refno.to_string(),
                aabb: panel_aabb,
                instances: panel_instances,
            });

            total_panels += 1;
        }

        rooms.push(RoomGeometryGroup {
            room_num: room_num.clone(),
            room_refno,
            aabb: room_aabb,
            panels: room_panels_data,
        });
    }

    let total_rooms = rooms.len();

    if verbose {
        info!("   - 共 {} 个房间, {} 个面板", total_rooms, total_panels);
    }

    // 7. 构建输出数据
    let data = RoomGeometriesData {
        version: 1,
        generated_at: Utc::now().to_rfc3339_opts(SecondsFormat::Millis, true),
        rooms,
    };

    // 8. 写入文件
    let json_content =
        serde_json::to_string_pretty(&data).context("序列化 room_geometries.json 失败")?;

    std::fs::write(output_path, &json_content)
        .with_context(|| format!("写入文件失败: {}", output_path.display()))?;

    let export_time_ms = start_time.elapsed().as_millis() as u64;

    if verbose {
        info!("✅ 房间几何导出完成: {}", output_path.display());
        info!("   - 耗时: {} ms", export_time_ms);
    }

    Ok(RoomExportStats {
        total_rooms,
        total_panels,
        total_components: 0,
        export_time_ms,
    })
}

// ============================================================================
// 辅助函数
// ============================================================================

/// 构建面板的几何实例列表
fn build_panel_instances(
    panel_refno: RefnoEnum,
    panel_geom_map: &HashMap<RefnoEnum, Vec<&PanelGeomQuery>>,
) -> Vec<PanelInstance> {
    let Some(geoms) = panel_geom_map.get(&panel_refno) else {
        return Vec::new();
    };

    geoms
        .iter()
        .filter_map(|geom| {
            let geo_transform = geom.world_trans.as_ref()?.to_matrix();
            let matrix_vec: Vec<f32> = geo_transform
                .to_cols_array()
                .iter()
                .map(|&v| v as f32)
                .collect();

            Some(PanelInstance {
                geo_hash: geom.geo_hash.clone(),
                geo_transform: matrix_vec,
            })
        })
        .collect()
}



/// 统一导出入口：同时导出 room_relations.json 和 room_geometries.json
pub async fn export_room_instances(
    output_dir: &Path,
    verbose: bool,
) -> Result<(RoomExportStats, RoomExportStats)> {
    // 确保输出目录存在
    std::fs::create_dir_all(output_dir)
        .with_context(|| format!("创建输出目录失败: {}", output_dir.display()))?;

    let relations_path = output_dir.join("room_relations.json");
    let geometries_path = output_dir.join("room_geometries.json");

    // 导出关系数据
    let relations_stats = export_room_relations(&relations_path, verbose).await?;

    // 导出几何数据
    let geometries_stats = export_room_geometries(&geometries_path, verbose).await?;

    if verbose {
        info!("🎉 房间数据导出完成!");
        info!(
            "   - room_relations.json: {} 个房间",
            relations_stats.total_rooms
        );
        info!(
            "   - room_geometries.json: {} 个面板",
            geometries_stats.total_panels
        );
    }

    Ok((relations_stats, geometries_stats))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn test_room_relation_queries_use_correct_direction() {
        let room_sql = build_query_room_relations_sql();
        assert!(room_sql.contains("in as panel_refno"));
        assert!(room_sql.contains("out as component_refno"));

        let room_panel_sql = build_query_room_panel_relations_sql();
        assert!(room_panel_sql.contains("in as room_refno"));
        assert!(room_panel_sql.contains("out as panel_refno"));
    }

    #[test]
    fn test_load_room_compute_validation_fixture() {
        let fixture_path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("verification/room/compute/room_compute_validation.json");

        let fixture = RoomComputeValidationFixture::load_from_path(&fixture_path)
            .expect("fixture should load");

        assert_eq!(fixture.description, "房间计算验证数据集");
        assert_eq!(fixture.test_cases.len(), 1);
        let case = &fixture.test_cases[0];
        assert_eq!(case.case_id, "room_540_panel_validation");
        assert_eq!(case.room_number, "R540");
        assert_eq!(case.panel_refno, "24381/35798");
        assert_eq!(case.expected_components, vec!["24381/145019"]);
    }
}
