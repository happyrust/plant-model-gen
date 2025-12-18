//! 调试 Manifold 转换问题
//! 
//! 分析为什么 mesh 转 manifold 后会丢失三角形

use aios_core::shape::pdms_shape::PlantMesh;
use aios_core::csg::manifold::ManifoldRust;
use glam::DMat4;

fn main() -> anyhow::Result<()> {
    // 测试加载 17496_106028 的正实体 mesh
    let mesh_id = "11815795513166638042";
    let mesh_path = format!(
        "/Volumes/DPC/work/plant-code/rs-plant3-d/assets/meshes/lod_L1/{}_L1.mesh",
        mesh_id
    );
    
    println!("=== 加载 mesh 文件 ===");
    let mesh = PlantMesh::des_mesh_file(&mesh_path)?;
    println!("✅ 加载成功:");
    println!("  - 顶点数: {}", mesh.vertices.len());
    println!("  - 索引数: {}", mesh.indices.len());
    println!("  - 三角形数: {}", mesh.indices.len() / 3);
    
    // 打印前几个顶点和索引
    println!("\n前5个顶点:");
    for (i, v) in mesh.vertices.iter().take(5).enumerate() {
        println!("  v{}: {:?}", i, v);
    }
    
    println!("\n前15个索引 (5个三角形):");
    for i in (0..mesh.indices.len().min(15)).step_by(3) {
        println!("  tri{}: [{}, {}, {}]", 
            i/3, 
            mesh.indices[i], 
            mesh.indices[i+1], 
            mesh.indices[i+2]
        );
    }
    
    // 测试不同的变换矩阵
    println!("\n=== 测试1: 单位矩阵 (more_precision=false) ===");
    test_conversion(&mesh, DMat4::IDENTITY, false)?;
    
    println!("\n=== 测试2: 单位矩阵 (more_precision=true) ===");
    test_conversion(&mesh, DMat4::IDENTITY, true)?;
    
    // 测试一个实际的变换矩阵（缩放）
    println!("\n=== 测试3: 缩放矩阵 0.001 (mm->m) ===");
    let scale_mat = DMat4::from_scale(glam::DVec3::splat(0.001));
    test_conversion(&mesh, scale_mat, false)?;
    
    println!("\n=== 测试4: 缩放矩阵 0.001 (more_precision=true) ===");
    test_conversion(&mesh, scale_mat, true)?;
    
    Ok(())
}

fn test_conversion(
    mesh: &PlantMesh, 
    mat: DMat4, 
    more_precision: bool
) -> anyhow::Result<()> {
    let manifold = ManifoldRust::convert_to_manifold(
        mesh.clone(), 
        mat, 
        more_precision
    );
    
    let result_mesh = manifold.get_mesh();
    println!("转换后 Manifold mesh:");
    println!("  - 顶点数: {}", result_mesh.vertices.len() / 3);
    println!("  - 索引数: {}", result_mesh.indices.len());
    println!("  - 三角形数: {}", result_mesh.indices.len() / 3);
    
    if result_mesh.indices.is_empty() {
        println!("  ❌ 失败: 没有三角形!");
    } else {
        println!("  ✅ 成功");
        
        // 打印前几个转换后的顶点
        println!("\n  前3个顶点:");
        for i in 0..3.min(result_mesh.vertices.len() / 3) {
            let idx = i * 3;
            println!("    v{}: [{:.6}, {:.6}, {:.6}]", 
                i,
                result_mesh.vertices[idx],
                result_mesh.vertices[idx + 1],
                result_mesh.vertices[idx + 2]
            );
        }
    }
    
    Ok(())
}
