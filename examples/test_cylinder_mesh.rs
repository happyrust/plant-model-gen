use std::fs::File;
use std::io::Write;
use std::path::PathBuf;

use aios_core::geometry::csg::{build_csg_mesh, unit_cylinder_mesh, unit_cylinder_mesh_standard};
use aios_core::mesh_precision::LodMeshSettings;
use aios_core::parsed_data::geo_params_data::PdmsGeoParam;
use aios_core::prim_geo::{SBox, Sphere};
use aios_core::shape::pdms_shape::PlantMesh;
use aios_core::types::RefnoEnum;
use glam::Vec3;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let output_dir = if args.len() > 1 {
        PathBuf::from(&args[1])
    } else {
        PathBuf::from("output/mesh_comparison")
    };

    std::fs::create_dir_all(&output_dir).unwrap();

    let settings = LodMeshSettings::default();

    println!("{}", "=".repeat(60));
    println!("基本体 Mesh 对比测试");
    println!("{}", "=".repeat(60));

    // ========== 1. 圆柱体 (SCylinder) ==========
    println!("\n【1】圆柱体 (SCylinder)");
    println!("{}", "-".repeat(40));

    let standard_mesh = unit_cylinder_mesh_standard(&settings, false);
    let manifold_mesh = unit_cylinder_mesh(&settings, false);

    export_and_compare(&standard_mesh, &manifold_mesh, &output_dir, "cylinder");

    // ========== 2. 盒子 (Box) ==========
    println!("\n【2】盒子 (Box)");
    println!("{}", "-".repeat(40));

    let sbox = SBox {
        center: Vec3::ZERO,
        size: Vec3::new(1.0, 1.0, 1.0),
    };
    let box_param = PdmsGeoParam::PrimBox(sbox.clone());

    let box_standard = build_csg_mesh(&box_param, &settings, false, false, RefnoEnum::default());
    let box_manifold = build_csg_mesh(&box_param, &settings, false, true, RefnoEnum::default());

    if let (Some(std), Some(mfd)) = (box_standard, box_manifold) {
        export_and_compare(&std.mesh, &mfd.mesh, &output_dir, "box");
    }

    // ========== 3. 球体 (Sphere) ==========
    println!("\n【3】球体 (Sphere)");
    println!("{}", "-".repeat(40));

    let sphere = Sphere {
        center: Vec3::ZERO,
        radius: 0.5,
    };
    let sphere_param = PdmsGeoParam::PrimSphere(sphere.clone());

    let sphere_standard =
        build_csg_mesh(&sphere_param, &settings, false, false, RefnoEnum::default());
    let sphere_manifold =
        build_csg_mesh(&sphere_param, &settings, false, true, RefnoEnum::default());

    if let (Some(std), Some(mfd)) = (sphere_standard, sphere_manifold) {
        export_and_compare(&std.mesh, &mfd.mesh, &output_dir, "sphere");
    }

    println!("\n{}", "=".repeat(60));
    println!("测试完成！文件已导出到: {:?}", output_dir);
    println!("{}", "=".repeat(60));
}

fn export_and_compare(
    standard: &PlantMesh,
    manifold: &PlantMesh,
    output_dir: &PathBuf,
    name: &str,
) {
    // 导出 OBJ 文件
    let std_path = output_dir.join(format!("{}_standard.obj", name));
    let mfd_path = output_dir.join(format!("{}_manifold.obj", name));

    export_obj(standard, &std_path);
    export_obj(manifold, &mfd_path);

    // 统计信息
    let std_shared = count_shared_vertices(standard);
    let mfd_shared = count_shared_vertices(manifold);

    println!("普通版本:");
    println!("  顶点数: {}", standard.vertices.len());
    println!("  三角形数: {}", standard.indices.len() / 3);
    println!("  重复顶点数: {}", std_shared);
    println!("  文件: {:?}", std_path);

    println!("Manifold 版本:");
    println!("  顶点数: {}", manifold.vertices.len());
    println!("  三角形数: {}", manifold.indices.len() / 3);
    println!("  重复顶点数: {}", mfd_shared);
    println!("  文件: {:?}", mfd_path);

    let reduction = if standard.vertices.len() > manifold.vertices.len() {
        (standard.vertices.len() - manifold.vertices.len()) as f32 / standard.vertices.len() as f32
            * 100.0
    } else {
        0.0
    };
    println!(
        "顶点减少: {} ({:.1}%)",
        standard
            .vertices
            .len()
            .saturating_sub(manifold.vertices.len()),
        reduction
    );
}

fn export_obj(mesh: &PlantMesh, path: &PathBuf) {
    let mut content = String::new();
    content.push_str("# OBJ file exported from AIOS\n");
    content.push_str(&format!("# Vertices: {}\n", mesh.vertices.len()));
    content.push_str(&format!("# Faces: {}\n\n", mesh.indices.len() / 3));

    for v in &mesh.vertices {
        content.push_str(&format!("v {:.6} {:.6} {:.6}\n", v.x, v.y, v.z));
    }

    content.push_str("\n");

    for tri in mesh.indices.chunks(3) {
        if tri.len() == 3 {
            content.push_str(&format!("f {} {} {}\n", tri[0] + 1, tri[1] + 1, tri[2] + 1));
        }
    }

    let mut file = File::create(path).unwrap();
    file.write_all(content.as_bytes()).unwrap();
}

fn count_shared_vertices(mesh: &PlantMesh) -> usize {
    let precision = 1000.0;
    let mut vertex_map = std::collections::HashMap::new();
    let mut shared_count = 0;

    for v in &mesh.vertices {
        let key = (
            (v.x * precision).round() as i64,
            (v.y * precision).round() as i64,
            (v.z * precision).round() as i64,
        );
        let count = vertex_map.entry(key).or_insert(0);
        *count += 1;
    }

    for (_, count) in &vertex_map {
        if *count > 1 {
            shared_count += count - 1;
        }
    }

    shared_count
}
