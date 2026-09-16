use aios_database::fast_model::export_model::import_glb::import_glb_to_mesh;
use std::path::Path;

fn main() {
    let path = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "assets/meshes/2.glb".to_string());
    let mesh = import_glb_to_mesh(Path::new(&path)).unwrap();

    println!("几何体: {path}");
    println!("  vertices: {}", mesh.vertices.len());
    println!("  normals: {}", mesh.normals.len());
    println!("  indices: {}", mesh.indices.len());
    if let (Some(&min), Some(&max)) = (mesh.indices.iter().min(), mesh.indices.iter().max()) {
        println!("  索引范围: [{}..={}]", min, max);
    }
    println!("  前10个索引: {:?}", &mesh.indices[..mesh.indices.len().min(10)]);

    // 简单检查：顶点法线与面法线是否大面积反向（用于快速定位“整体翻转/绕序问题”）
    if mesh.normals.len() == mesh.vertices.len() && !mesh.normals.is_empty() {
        let mut checked = 0u32;
        let mut opposite = 0u32;
        for tri in mesh.indices.chunks(3) {
            if tri.len() < 3 {
                continue;
            }
            let i0 = tri[0] as usize;
            let i1 = tri[1] as usize;
            let i2 = tri[2] as usize;
            if i0 >= mesh.vertices.len() || i1 >= mesh.vertices.len() || i2 >= mesh.vertices.len() {
                continue;
            }
            let v0 = mesh.vertices[i0];
            let v1 = mesh.vertices[i1];
            let v2 = mesh.vertices[i2];
            let face_n = (v1 - v0).cross(v2 - v0);
            if face_n.length_squared() <= f32::EPSILON {
                continue;
            }
            let face_n = face_n.normalize();

            let avg_vn = (mesh.normals[i0] + mesh.normals[i1] + mesh.normals[i2]) / 3.0;
            if avg_vn.length_squared() <= f32::EPSILON {
                continue;
            }
            let avg_vn = avg_vn.normalize();

            checked += 1;
            if face_n.dot(avg_vn) < 0.0 {
                opposite += 1;
            }
        }
        if checked > 0 {
            let ratio = (opposite as f64) * 100.0 / (checked as f64);
            println!("  法线反向比例(面法线·顶点法线<0): {opposite}/{checked} ({ratio:.2}%)");
        }
    } else {
        println!("  顶点法线缺失或数量不匹配：跳过面法线一致性检查");
    }
}

