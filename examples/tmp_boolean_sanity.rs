//! 临时布尔健全性检查：unit_cylinder_mesh 与 unit_box_mesh 做差集。
//!
//! 用途：快速验证 Manifold difference 在“同坐标系、简单变换”下是否会产生切割结果，
//! 以辅助定位“负实体未减去”到底是输入变换/关系问题，还是布尔库/网格问题。
//!
//! 运行：
//!   cargo run --example tmp_boolean_sanity

use aios_core::csg::manifold::ManifoldRust;
use aios_core::geometry::csg::{unit_box_mesh, unit_cylinder_mesh};
use aios_core::mesh_precision::LodMeshSettings;
use glam::{DMat4, DVec3};
use std::path::Path;

fn main() -> anyhow::Result<()> {
    let settings = LodMeshSettings::default();

    // pos: unit_cylinder_mesh 以 z=[0..1] 为高度（见 rs-core/src/geometry/csg.rs），
    // 这里用与 24381_131092 类似的尺寸：x/y 直径 66，z 高 190。
    let pos_scale = DVec3::new(66.0, 66.0, 190.0);
    let pos_mat = DMat4::from_scale(pos_scale);

    // neg: unit_box_mesh 为中心在原点的立方体 [-0.5..0.5]^3，
    // 这里做一块薄切片：x 宽 30，y/z 足够大覆盖 pos（经验上用于切成两段）。
    let neg_scale = DVec3::new(30.0, 262.0, 262.0);
    let neg_mat_centered = DMat4::from_scale(neg_scale);
    // “底对齐”版本：unit_box_mesh 当前是中心在原点的 [-0.5..0.5]，
    // 若业务期望 box 像 cylinder 一样以 z=0 为底面，则需上移 scale.z/2。
    let neg_mat_z0 = DMat4::from_translation(DVec3::new(0.0, 0.0, neg_scale.z / 2.0))
        * DMat4::from_scale(neg_scale);

    let pos = {
        let m = unit_cylinder_mesh(&settings, false);
        let vertices: Vec<glam::Vec3> = m
            .vertices
            .iter()
            .map(|v| glam::Vec3::new(v.x, v.y, v.z))
            .collect();
        ManifoldRust::from_vertices_indices(&vertices, &m.indices, pos_mat, false)
    };

    let neg_centered = {
        let m = unit_box_mesh();
        let vertices: Vec<glam::Vec3> = m
            .vertices
            .iter()
            .map(|v| glam::Vec3::new(v.x, v.y, v.z))
            .collect();
        ManifoldRust::from_vertices_indices(&vertices, &m.indices, neg_mat_centered, true)
    };

    let neg_z0 = {
        let m = unit_box_mesh();
        let vertices: Vec<glam::Vec3> = m
            .vertices
            .iter()
            .map(|v| glam::Vec3::new(v.x, v.y, v.z))
            .collect();
        ManifoldRust::from_vertices_indices(&vertices, &m.indices, neg_mat_z0, true)
    };

    let mut diff_centered = pos.clone();
    diff_centered.inner = diff_centered.inner.difference(&neg_centered.inner);

    let mut diff_z0 = pos.clone();
    diff_z0.inner = diff_z0.inner.difference(&neg_z0.inner);

    let out_glb_centered = "output/tmp_boolean_sanity_centered.glb";
    let out_glb_z0 = "output/tmp_boolean_sanity_z0.glb";
    std::fs::create_dir_all("output")?;
    diff_centered.export_to_glb(Path::new(out_glb_centered))?;
    diff_z0.export_to_glb(Path::new(out_glb_z0))?;

    println!("✅ 写出: {out_glb_centered}");
    println!("✅ 写出: {out_glb_z0}");
    println!(
        "pos: v={} tri={}",
        pos.get_mesh().vertices.len(),
        pos.get_mesh().indices.len() / 3
    );
    println!(
        "neg_centered: v={} tri={}",
        neg_centered.get_mesh().vertices.len(),
        neg_centered.get_mesh().indices.len() / 3
    );
    println!(
        "diff_centered: v={} tri={}",
        diff_centered.get_mesh().vertices.len(),
        diff_centered.get_mesh().indices.len() / 3
    );
    println!(
        "neg_z0: v={} tri={}",
        neg_z0.get_mesh().vertices.len(),
        neg_z0.get_mesh().indices.len() / 3
    );
    println!(
        "diff_z0: v={} tri={}",
        diff_z0.get_mesh().vertices.len(),
        diff_z0.get_mesh().indices.len() / 3
    );

    Ok(())
}
