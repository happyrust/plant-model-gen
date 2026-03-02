//! 布尔运算任务数据结构（内存驱动）
//!
//! 目标：
//! - 从 `ShapeInstancesData` 构建布尔任务，避免再反查 DB
//! - 支持跨批次汇总，防止“正实体/负实体分批到达”导致漏任务
//! - 对 `ngmr_neg_relate_map` 保留几何级精确映射（carrier_refno, ngmr_geom_refno）

use aios_core::geometry::{GeoBasicType, ShapeInstancesData};
use aios_core::parsed_data::geo_params_data::PdmsGeoParam;
use aios_core::{RefnoEnum, Transform};
use std::collections::{HashMap, HashSet};

/// 单个布尔运算任务
#[derive(Debug, Clone)]
pub struct BooleanTask {
    /// 目标实例参考号（被切割对象）
    pub refno: RefnoEnum,
    /// 目标实例 noun（用于内存侧 BRAN 过滤）
    pub noun: Option<String>,
    /// 任务类型
    pub task_type: BooleanTaskType,
}

/// 布尔任务类型
#[derive(Debug, Clone)]
pub enum BooleanTaskType {
    /// 元件库布尔：同一元件内的正/负几何体做差集
    CataNeg(CataNegBoolTask),
    /// 实例级布尔：外部负实体切割正实体
    InstNeg(InstNegBoolTask),
}

/// 元件库布尔任务数据（等价于 `CataNegGroup`，但从内存传递）
#[derive(Debug, Clone)]
pub struct CataNegBoolTask {
    /// inst_info 的 ID 字符串（用于结果回写）
    pub inst_info_id: String,
    /// 布尔分组：每组中 [0] 是正实体 geom_refno，[1..] 是负实体 geom_refno
    pub boolean_groups: Vec<Vec<RefnoEnum>>,
    /// 各 geom 的数据（以 geom_refno 为 key）
    pub geo_data_map: HashMap<RefnoEnum, CataGeoData>,
}

/// 元件库布尔中单个几何体的数据
#[derive(Debug, Clone)]
pub struct CataGeoData {
    /// 几何哈希值（即 mesh_id）
    pub geo_hash: u64,
    /// 几何参数
    pub param: PdmsGeoParam,
    /// 局部变换矩阵
    pub transform: Transform,
}

/// 实例级布尔任务数据（等价于 `ManiGeoTransQuery`，但从内存传递）
#[derive(Debug, Clone)]
pub struct InstNegBoolTask {
    /// 正实体的世界变换矩阵
    pub inst_world_transform: Transform,
    /// 正实体几何列表
    pub pos_geos: Vec<PosGeoData>,
    /// 负实体列表
    pub neg_entities: Vec<NegEntityData>,
}

/// 正实体几何数据
#[derive(Debug, Clone)]
pub struct PosGeoData {
    /// 几何哈希（mesh_id，用于从 .manifold 文件加载）
    pub geo_hash: String,
    /// 局部变换矩阵
    pub local_transform: Transform,
}

/// 负实体数据（一个 carrier 的几何集合）
#[derive(Debug, Clone)]
pub struct NegEntityData {
    /// 负载体参考号
    pub carrier_refno: RefnoEnum,
    /// 负载体的世界变换
    pub carrier_world_transform: Transform,
    /// NGMR 精确几何 refno（None 表示普通 neg_relate 关系，Some 表示 ngmr_relate 指定几何）
    pub ngmr_geom_refno: Option<RefnoEnum>,
    /// 负实体的几何列表
    pub neg_geos: Vec<NegGeoData>,
}

/// 单个负几何体数据
#[derive(Debug, Clone)]
pub struct NegGeoData {
    /// 几何哈希（mesh_id）
    pub geo_hash: String,
    /// 几何 refno（用于 NGMR 精确匹配）
    pub geom_refno: RefnoEnum,
    /// 几何类型字符串
    pub geo_type: String,
    /// 局部变换矩阵
    pub local_transform: Transform,
}

/// 跨批次布尔任务汇总器。
///
/// 在 insert_handle 收到多个 `ShapeInstancesData` 批次时，先 merge 到此汇总器，
/// 最终一次性构建任务，避免跨批次关系丢失。
#[derive(Debug, Default)]
pub struct BooleanTaskAccumulator {
    merged: ShapeInstancesData,
}

impl BooleanTaskAccumulator {
    /// 合并单个批次到累积状态
    pub fn merge_batch(&mut self, batch: &ShapeInstancesData) {
        // inst_info_map: key 是 RefnoEnum（唯一标识），同 refno 跨批次的 info 语义相同，覆盖安全
        for (refno, info) in &batch.inst_info_map {
            self.merged.inst_info_map.insert(*refno, info.clone());
        }
        // inst_geos_map: 复用 aios_core 的 insert_geos_data 合并语义，
        // 同 inst_key 跨批次的 insts 会 extend_from_slice 而非覆盖
        for (inst_key, geos) in &batch.inst_geos_map {
            self.merged
                .insert_geos_data(inst_key.clone(), geos.clone());
        }
        // inst_tubi_map: key 是 RefnoEnum（唯一标识），同 refno 跨批次的 tubi 语义相同，覆盖安全
        for (refno, tubi) in &batch.inst_tubi_map {
            self.merged.inst_tubi_map.insert(*refno, tubi.clone());
        }

        for (target, carriers) in &batch.neg_relate_map {
            let merged = self.merged.neg_relate_map.entry(*target).or_default();
            merged.extend(carriers.iter().copied());
            dedup_refnos(merged);
        }

        for (target, pairs) in &batch.ngmr_neg_relate_map {
            let merged = self.merged.ngmr_neg_relate_map.entry(*target).or_default();
            merged.extend(pairs.iter().copied());
            dedup_pairs(merged);
        }
    }

    /// 从已汇总数据构建完整布尔任务
    pub fn build_tasks(&self) -> Vec<BooleanTask> {
        build_boolean_tasks(&self.merged)
    }
}

fn dedup_refnos(values: &mut Vec<RefnoEnum>) {
    let mut seen: HashSet<RefnoEnum> = HashSet::new();
    values.retain(|v| seen.insert(*v));
}

fn dedup_pairs(values: &mut Vec<(RefnoEnum, RefnoEnum)>) {
    let mut seen: HashSet<(RefnoEnum, RefnoEnum)> = HashSet::new();
    values.retain(|v| seen.insert(*v));
}

fn build_noun_map(shape_insts: &ShapeInstancesData) -> HashMap<RefnoEnum, String> {
    let mut noun_map: HashMap<RefnoEnum, String> = HashMap::new();
    for (_, geos_data) in &shape_insts.inst_geos_map {
        if !geos_data.type_name.is_empty() {
            noun_map.insert(geos_data.refno, geos_data.type_name.to_uppercase());
        }
    }
    noun_map
}

/// 从 ShapeInstancesData 中提取元件库布尔任务
pub fn extract_cata_neg_tasks(shape_insts: &ShapeInstancesData) -> Vec<BooleanTask> {
    let noun_map = build_noun_map(shape_insts);
    let mut tasks = Vec::new();

    for (refno, info) in &shape_insts.inst_info_map {
        let refno = *refno;
        let noun = noun_map.get(&refno).cloned();
        if noun.as_deref() == Some("BRAN") {
            continue;
        }

        let inst_key = info.get_inst_key();
        let Some(geos_data) = shape_insts.inst_geos_map.get(&inst_key) else {
            continue;
        };

        let mut boolean_groups: Vec<Vec<RefnoEnum>> = Vec::new();
        let mut geo_data_map: HashMap<RefnoEnum, CataGeoData> = HashMap::new();

        for geo in &geos_data.insts {
            geo_data_map.insert(
                geo.refno,
                CataGeoData {
                    geo_hash: geo.geo_hash,
                    param: geo.geo_param.clone(),
                    transform: geo.geo_transform,
                },
            );

            if geo.geo_type == GeoBasicType::Pos && !geo.cata_neg_refnos.is_empty() {
                let mut group = vec![geo.refno];
                group.extend_from_slice(&geo.cata_neg_refnos);
                boolean_groups.push(group);
            }
        }

        if boolean_groups.is_empty() {
            continue;
        }

        tasks.push(BooleanTask {
            refno,
            noun,
            task_type: BooleanTaskType::CataNeg(CataNegBoolTask {
                inst_info_id: info.id_str(),
                boolean_groups,
                geo_data_map,
            }),
        });
    }

    tasks
}

/// 从 ShapeInstancesData 中提取实例级布尔任务
///
/// 规则：
/// - `neg_relate_map`: carrier 级关系（全部 Neg/CataCrossNeg 几何）
/// - `ngmr_neg_relate_map`: 几何级关系（仅应用指定 ngmr_geom_refno）
pub fn extract_inst_neg_tasks(shape_insts: &ShapeInstancesData) -> Vec<BooleanTask> {
    let noun_map = build_noun_map(shape_insts);
    let mut tasks = Vec::new();

    // target -> [(carrier_refno, ngmr_geom_refno?)]
    let mut target_neg_specs: HashMap<RefnoEnum, Vec<(RefnoEnum, Option<RefnoEnum>)>> =
        HashMap::new();

    for (target, carriers) in &shape_insts.neg_relate_map {
        let entry = target_neg_specs.entry(*target).or_default();
        entry.extend(carriers.iter().map(|carrier| (*carrier, None)));
    }

    for (target, pairs) in &shape_insts.ngmr_neg_relate_map {
        let entry = target_neg_specs.entry(*target).or_default();
        entry.extend(pairs.iter().map(|(carrier, ngmr_geom)| (*carrier, Some(*ngmr_geom))));
    }

    for (target_refno, raw_specs) in target_neg_specs {
        let noun = noun_map.get(&target_refno).cloned();
        if noun.as_deref() == Some("BRAN") {
            continue;
        }

        let Some(info) = shape_insts.inst_info_map.get(&target_refno) else {
            continue;
        };

        let inst_key = info.get_inst_key();
        let pos_geos: Vec<PosGeoData> = if let Some(geos_data) = shape_insts.inst_geos_map.get(&inst_key) {
            geos_data
                .insts
                .iter()
                .filter(|g| g.geo_type == GeoBasicType::Pos)
                .map(|g| PosGeoData {
                    geo_hash: g.geo_hash.to_string(),
                    local_transform: g.geo_transform,
                })
                .collect()
        } else {
            Vec::new()
        };

        if pos_geos.is_empty() {
            continue;
        }

        let mut uniq_specs: HashSet<(RefnoEnum, Option<RefnoEnum>)> = HashSet::new();
        let mut neg_entities = Vec::new();

        for (carrier_refno, ngmr_geom_refno) in raw_specs {
            if !uniq_specs.insert((carrier_refno, ngmr_geom_refno)) {
                continue;
            }

            let carrier_info = shape_insts.inst_info_map.get(&carrier_refno);
            let carrier_world_transform = carrier_info
                .as_ref()
                .map(|i| i.world_transform)
                .unwrap_or_default();
            let carrier_inst_key = carrier_info
                .as_ref()
                .map(|i| i.get_inst_key())
                .unwrap_or_default();

            let mut seen_neg_geo: HashSet<(RefnoEnum, String)> = HashSet::new();
            let mut neg_geos: Vec<NegGeoData> = Vec::new();
            if let Some(geos_data) = shape_insts.inst_geos_map.get(&carrier_inst_key) {
                for geo in &geos_data.insts {
                    if !matches!(geo.geo_type, GeoBasicType::Neg | GeoBasicType::CataCrossNeg) {
                        continue;
                    }
                    if let Some(expect_geom_refno) = ngmr_geom_refno {
                        if geo.refno != expect_geom_refno {
                            continue;
                        }
                    }
                    let geo_hash = geo.geo_hash.to_string();
                    if !seen_neg_geo.insert((carrier_refno, geo_hash.clone())) {
                        continue;
                    }
                    neg_geos.push(NegGeoData {
                        geo_hash,
                        geom_refno: geo.refno,
                        geo_type: format!("{}", geo.geo_type),
                        local_transform: geo.geo_transform,
                    });
                }
            }

            if neg_geos.is_empty() {
                continue;
            }

            neg_entities.push(NegEntityData {
                carrier_refno,
                carrier_world_transform,
                ngmr_geom_refno,
                neg_geos,
            });
        }

        if neg_entities.is_empty() {
            continue;
        }

        tasks.push(BooleanTask {
            refno: target_refno,
            noun,
            task_type: BooleanTaskType::InstNeg(InstNegBoolTask {
                inst_world_transform: info.world_transform,
                pos_geos,
                neg_entities,
            }),
        });
    }

    tasks
}

/// 从 ShapeInstancesData 构建完整的布尔任务集合
pub fn build_boolean_tasks(shape_insts: &ShapeInstancesData) -> Vec<BooleanTask> {
    let mut tasks = Vec::new();
    tasks.extend(extract_cata_neg_tasks(shape_insts));
    tasks.extend(extract_inst_neg_tasks(shape_insts));
    tasks
}
