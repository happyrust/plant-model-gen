//! [foyer-removal] 桩模块：geom_input_cache 已移除。

use crate::options::DbOptionExt;
use aios_core::RefnoEnum;
use aios_core::Transform;
use aios_core::geometry::EleGeosInfo;
use aios_core::parsed_data::CateAxisParam;
use aios_core::types::NamedAttrMap;
use std::collections::HashMap;

/// 缓存运行模式（桩）
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CacheRunMode {
    Direct,
    PrefetchThenGenerate,
}

/// Loop 输入（桩）
#[derive(Clone, Debug)]
pub struct LoopInput {
    pub refno: RefnoEnum,
    pub attmap: NamedAttrMap,
    pub owner_refno: RefnoEnum,
    pub owner_type: String,
    pub world_transform: Transform,
    pub visible: bool,
    pub neg_refnos: Vec<RefnoEnum>,
    pub cmpf_neg_refnos: Vec<RefnoEnum>,
    pub loops: Vec<glam::Vec3>,
    pub height: f32,
}

/// Prim 输入（桩）
#[derive(Clone, Debug)]
pub struct PrimInput {
    pub refno: RefnoEnum,
    pub attmap: NamedAttrMap,
    pub owner_refno: RefnoEnum,
    pub owner_type: String,
    pub world_transform: Transform,
    pub visible: bool,
    pub neg_refnos: Vec<RefnoEnum>,
    pub poly_extra: Option<PrimPolyExtra>,
}

#[derive(Clone, Debug)]
pub struct PrimPolyExtra {
    pub polygons: Vec<PrimPolygonData>,
    pub is_polyhe: bool,
}

#[derive(Clone, Debug)]
pub struct PrimPolygonData {
    pub loops: Vec<Vec<glam::Vec3>>,
}

/// Cate 输入（桩）
#[derive(Clone, Debug)]
pub struct CateInput {
    pub refno: RefnoEnum,
    pub attmap: NamedAttrMap,
    pub owner_refno: RefnoEnum,
    pub owner_type: String,
    pub world_transform: Transform,
    pub visible: bool,
}

pub fn init_global_geom_input_cache() {}

pub async fn prefetch_all_geom_inputs(
    _db_option: &DbOptionExt,
    _loop_refs: &[RefnoEnum],
    _prim_refs: &[RefnoEnum],
    _cate_refs: &[RefnoEnum],
) -> anyhow::Result<()> {
    Ok(())
}

pub fn ensure_geom_inputs_present_for_refnos_from_global(
    _loop_refs: &[RefnoEnum],
    _prim_refs: &[RefnoEnum],
    _cate_refs: &[RefnoEnum],
) -> anyhow::Result<()> {
    Ok(())
}

pub fn load_cate_inputs_for_refnos_from_global(
    _refnos: &[RefnoEnum],
) -> anyhow::Result<HashMap<RefnoEnum, CateInput>> {
    Ok(HashMap::new())
}
