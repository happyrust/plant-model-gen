//! 检查 mesh 的法向方向和拓扑结构

use aios_core::shape::pdms_shape::PlantMesh;
use std::collections::HashMap;

fn main() -> anyhow::Result<()> {
    let mesh_id = "11815795513166638042";
    let mesh_path = format!(
        "/Volumes/DPC/work/plant-code/rs-plant3-d/assets/meshes/lod_L1/{}_L1.mesh",
        mesh_id
    );
    
    let mesh = PlantMesh::des_mesh_file(&mesh_path)?;
    
    println!("=== 三角形法向检查 ===\n");
    
    let mut face_normals = Vec::new();
    
    for i in (0..mesh.indices.len()).step_by(3) {
        let i0 = mesh.indices[i] as usize;
        let i1 = mesh.indices[i + 1] as usize;
        let i2 = mesh.indices[i + 2] as usize;
        
        let v0 = mesh.vertices[i0];
        let v1 = mesh.vertices[i1];
        let v2 = mesh.vertices[i2];
        
        let edge1 = v1 - v0;
        let edge2 = v2 - v0;
        let normal = edge1.cross(edge2).normalize();
        let area = edge1.cross(edge2).length() * 0.5;
        
        face_normals.push(normal);
        
        println!("三角形 {}:", i / 3);
        println!("  顶点索引: [{}, {}, {}]", i0, i1, i2);
        println!("  顶点坐标:");
        println!("    v{}: {:?}", i0, v0);
        println!("    v{}: {:?}", i1, v1);
        println!("    v{}: {:?}", i2, v2);
        println!("  法向: {:?}", normal);
        println!("  面积: {:.2}", area);
        println!();
    }
    
    // 检查边的共享情况
    println!("\n=== 边共享检查 ===\n");
    
    let mut edges: HashMap<(u32, u32), Vec<usize>> = HashMap::new();
    
    for i in (0..mesh.indices.len()).step_by(3) {
        let tri_idx = i / 3;
        let i0 = mesh.indices[i];
        let i1 = mesh.indices[i + 1];
        let i2 = mesh.indices[i + 2];
        
        // 添加三条边（使用排序后的顶点对作为key）
        let e1 = if i0 < i1 { (i0, i1) } else { (i1, i0) };
        let e2 = if i1 < i2 { (i1, i2) } else { (i2, i1) };
        let e3 = if i2 < i0 { (i2, i0) } else { (i0, i2) };
        
        edges.entry(e1).or_default().push(tri_idx);
        edges.entry(e2).or_default().push(tri_idx);
        edges.entry(e3).or_default().push(tri_idx);
    }
    
    let mut boundary_edges = 0;
    let mut shared_edges = 0;
    let mut over_shared = 0;
    
    for (edge, tris) in &edges {
        match tris.len() {
            1 => {
                boundary_edges += 1;
                println!("边界边 {:?}: 只被三角形 {} 使用", edge, tris[0]);
            }
            2 => {
                shared_edges += 1;
            }
            n => {
                over_shared += 1;
                println!("过度共享边 {:?}: 被 {} 个三角形使用: {:?}", edge, n, tris);
            }
        }
    }
    
    println!("\n边统计:");
    println!("  边界边（只被1个三角形使用）: {}", boundary_edges);
    println!("  共享边（被2个三角形使用）: {}", shared_edges);
    println!("  过度共享边（被>2个三角形使用）: {}", over_shared);
    
    if boundary_edges > 0 {
        println!("\n  ⚠️  警告: 存在边界边，这不是一个封闭的流形!");
    }
    
    // 检查是否是封闭体积
    println!("\n=== 流形检查 ===");
    let total_edges = edges.len();
    let expected_edges_for_closed = (mesh.indices.len() / 3) * 3 / 2; // 每个三角形3条边，每条边被2个三角形共享
    
    println!("  总边数: {}", total_edges);
    println!("  封闭流形期望边数: {}", expected_edges_for_closed);
    
    if boundary_edges == 0 && over_shared == 0 {
        println!("  ✅ 这是一个封闭的流形");
    } else {
        println!("  ❌ 这不是一个有效的封闭流形");
    }
    
    // 尝试用 aios_core 的 Manifold 创建
    println!("\n=== 尝试创建 Manifold ===");
    
    use aios_core::csg::manifold::ManifoldRust;
    use glam::DMat4;
    
    println!("使用单位矩阵转换...");
    let manifold = ManifoldRust::convert_to_manifold(mesh.clone(), DMat4::IDENTITY, false);
    let result = manifold.get_mesh();
    
    println!("\nManifold 转换结果:");
    println!("  输出顶点数: {}", result.vertices.len() / 3);
    println!("  输出索引数: {}", result.indices.len());
    println!("  输出三角形数: {}", result.indices.len() / 3);
    
    if result.indices.is_empty() {
        println!("  ❌ Manifold 拒绝了这个几何体!");
        println!("\n可能的原因:");
        println!("  1. 几何体不是封闭的流形（有边界边）");
        println!("  2. 三角形法向不一致");
        println!("  3. 几何体自相交");
        println!("  4. 几何体退化（体积为0或厚度过薄）");
        println!("  5. 精度截断导致顶点重合");
    } else {
        println!("  ✅ Manifold 接受了这个几何体");
    }
    
    Ok(())
}
