//! 分析 mesh 几何特征，找出为什么 Manifold 认为它无效

use aios_core::shape::pdms_shape::PlantMesh;
use glam::Vec3;
use std::collections::HashSet;

fn main() -> anyhow::Result<()> {
    let mesh_id = "11815795513166638042";
    let mesh_path = format!(
        "/Volumes/DPC/work/plant-code/rs-plant3-d/assets/meshes/lod_L1/{}_L1.mesh",
        mesh_id
    );
    
    println!("=== 加载并分析 mesh ===");
    let mesh = PlantMesh::des_mesh_file(&mesh_path)?;
    
    println!("\n基本信息:");
    println!("  顶点数: {}", mesh.vertices.len());
    println!("  索引数: {}", mesh.indices.len());
    println!("  三角形数: {}", mesh.indices.len() / 3);
    
    // 检查索引范围
    println!("\n索引范围检查:");
    let max_index = mesh.indices.iter().max().copied().unwrap_or(0);
    let min_index = mesh.indices.iter().min().copied().unwrap_or(0);
    println!("  最小索引: {}", min_index);
    println!("  最大索引: {}", max_index);
    println!("  顶点数-1: {}", mesh.vertices.len() - 1);
    
    if max_index >= mesh.vertices.len() as u32 {
        println!("  ❌ 错误: 索引超出顶点范围!");
    } else {
        println!("  ✅ 索引范围正常");
    }
    
    // 检查退化三角形
    println!("\n退化三角形检查:");
    let mut degenerate_count = 0;
    for i in (0..mesh.indices.len()).step_by(3) {
        let i0 = mesh.indices[i];
        let i1 = mesh.indices[i + 1];
        let i2 = mesh.indices[i + 2];
        
        if i0 == i1 || i1 == i2 || i0 == i2 {
            println!("  退化三角形 {}: [{}, {}, {}]", i/3, i0, i1, i2);
            degenerate_count += 1;
        }
    }
    
    if degenerate_count > 0 {
        println!("  ❌ 发现 {} 个退化三角形", degenerate_count);
    } else {
        println!("  ✅ 无退化三角形");
    }
    
    // 检查零面积三角形
    println!("\n零面积/共线三角形检查:");
    let mut zero_area_count = 0;
    for i in (0..mesh.indices.len()).step_by(3) {
        let v0 = mesh.vertices[mesh.indices[i] as usize];
        let v1 = mesh.vertices[mesh.indices[i + 1] as usize];
        let v2 = mesh.vertices[mesh.indices[i + 2] as usize];
        
        let edge1 = v1 - v0;
        let edge2 = v2 - v0;
        let cross = edge1.cross(edge2);
        let area = cross.length() * 0.5;
        
        if area < 1e-6 {
            println!("  零面积三角形 {}: area={:.10}, vertices:", i/3, area);
            println!("    v0: {:?}", v0);
            println!("    v1: {:?}", v1);
            println!("    v2: {:?}", v2);
            zero_area_count += 1;
        }
    }
    
    if zero_area_count > 0 {
        println!("  ❌ 发现 {} 个零面积三角形", zero_area_count);
    } else {
        println!("  ✅ 无零面积三角形");
    }
    
    // 检查顶点重复
    println!("\n顶点重复检查:");
    let unique_verts: HashSet<_> = mesh.vertices.iter()
        .map(|v| (
            (v.x * 1000.0) as i32,
            (v.y * 1000.0) as i32,
            (v.z * 1000.0) as i32,
        ))
        .collect();
    println!("  总顶点数: {}", mesh.vertices.len());
    println!("  唯一顶点数: {}", unique_verts.len());
    
    if unique_verts.len() < mesh.vertices.len() {
        println!("  ⚠️  有 {} 个重复顶点", mesh.vertices.len() - unique_verts.len());
    }
    
    // 计算 AABB
    println!("\n包围盒 (AABB):");
    let mut min = Vec3::splat(f32::MAX);
    let mut max = Vec3::splat(f32::MIN);
    for v in &mesh.vertices {
        min = min.min(*v);
        max = max.max(*v);
    }
    println!("  Min: {:?}", min);
    println!("  Max: {:?}", max);
    println!("  Size: {:?}", max - min);
    
    let size = max - min;
    if size.x < 1e-6 || size.y < 1e-6 || size.z < 1e-6 {
        println!("  ❌ 警告: 某个维度尺寸过小 (可能是平面几何)");
    }
    
    // 打印所有顶点
    println!("\n所有顶点:");
    for (i, v) in mesh.vertices.iter().enumerate() {
        println!("  v{}: {:?}", i, v);
    }
    
    // 打印所有三角形
    println!("\n所有三角形:");
    for i in (0..mesh.indices.len()).step_by(3) {
        let i0 = mesh.indices[i] as usize;
        let i1 = mesh.indices[i + 1] as usize;
        let i2 = mesh.indices[i + 2] as usize;
        println!("  tri{}: [{}, {}, {}] -> {:?}, {:?}, {:?}", 
            i/3, i0, i1, i2,
            mesh.vertices[i0],
            mesh.vertices[i1],
            mesh.vertices[i2]
        );
    }
    
    Ok(())
}
