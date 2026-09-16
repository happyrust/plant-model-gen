//! Reuse Unit 几何体复用辅助函数
//!
//! 本模块提供统一的单位几何体复用判断和 transform 处理逻辑，
//! 消除 prim_model/loop_model/cata_model/primitive_builder 中的重复代码。

use aios_core::Transform;
use glam::Vec3;

/// 判断 geo_hash 是否为内置单位几何体 (1/2/3)
///
/// 内置单位几何体：
/// - 1: unit_box_mesh
/// - 2: unit_cylinder_mesh
/// - 3: unit_sphere_mesh
#[inline]
pub fn is_builtin_unit_geo_hash(geo_hash: u64) -> bool {
    matches!(geo_hash, 1 | 2 | 3)
}

/// 统一处理 transform.scale 清零逻辑
///
/// 约定：当 unit_flag=false 且 geo_hash 不是内置 unit mesh(1/2/3) 时，
/// 本仓库生成出来的 mesh 顶点已包含"真实尺寸"（由 geo_param 决定），
/// 因此实例层不应再携带非 1 的 scale；否则导出/布尔会把尺寸再乘一次，导致平方级放大。
#[inline]
pub fn normalize_transform_scale(transform: &mut Transform, unit_flag: bool, geo_hash: u64) {
    if !unit_flag && !is_builtin_unit_geo_hash(geo_hash) {
        transform.scale = Vec3::ONE;
    }
}
