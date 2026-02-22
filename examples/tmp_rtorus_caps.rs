//! 临时诊断：生成一个 PrimRTorus，并检查是否包含起止端面（角度 < 360° 时应闭合）。
//!
//! 用法：
//!   cargo run --example tmp_rtorus_caps
//!
//! 该文件仅用于排查 RTOR 的 Manifold 导入失败问题；确认后可删除。

use aios_core::RefnoEnum;
use aios_core::geometry::csg::generate_csg_mesh;
use aios_core::mesh_precision::LodMeshSettings;
use aios_core::parsed_data::geo_params_data::PdmsGeoParam;
use aios_core::prim_geo::rtorus::RTorus;

fn main() {
    // 来自问题现场：PrimRTorus(RTorus { rins: 147.0, rout: 157.0, height: 5.0, angle: 60.0 })
    let rt = RTorus {
        rins: 147.0,
        rout: 157.0,
        height: 5.0,
        angle: 60.0,
    };
    let param = PdmsGeoParam::PrimRTorus(rt);
    let mut settings = LodMeshSettings::default();
    // 现场 mesh（915397...）推测 major_segments_base≈30（60° -> ceil(base*1/6)=5），这里模拟一下。
    settings.radial_segments = 30;

    for manifold in [false, true] {
        let Some(g) = generate_csg_mesh(
            &param,
            &settings,
            false,
            manifold,
            Some(RefnoEnum::default()),
        ) else {
            println!("manifold={}: generate_csg_mesh -> None", manifold);
            continue;
        };
        let v = g.mesh.vertices.len();
        let i = g.mesh.indices.len();
        let t = i / 3;
        println!(
            "manifold={}: verts={} indices={} tris={}",
            manifold, v, i, t
        );
    }
}
