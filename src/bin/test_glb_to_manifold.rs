use aios_core::csg::manifold::ManifoldRust;
use aios_database::fast_model::export_model::import_glb::import_glb_to_mesh;
use glam::DMat4;
use std::path::Path;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let args: Vec<String> = std::env::args().collect();
    let regen_model = args.contains(&"--regen-model".to_string());
    let path_arg = args
        .iter()
        .position(|x| x == "--path")
        .and_then(|i| args.get(i + 1))
        .map(|s| s.as_str());

    println!("=== GLB to Manifold 直接转换测试 ===\n");

    let glb_path = Path::new(
        path_arg.unwrap_or("/Volumes/DPC/work/plant-code/gen-model-fork/assets/meshes/lod_L1/1_L1.glb"),
    );
    if !glb_path.exists() {
        anyhow::bail!("测试文件不存在: {:?}", glb_path);
    }

    println!("正在加载并转换文件: {:?}", glb_path);

    // 先用 GLB -> PlantMesh 做输入体检
    match import_glb_to_mesh(glb_path) {
        Ok(mesh) => {
            println!("\n[输入 PlantMesh]");
            println!("  vertices: {}", mesh.vertices.len());
            println!("  normals: {}", mesh.normals.len());
            println!("  indices: {}", mesh.indices.len());
            if mesh.indices.len() % 3 != 0 {
                println!("  ⚠️ indices 不是 3 的倍数：{}", mesh.indices.len());
            }
            if let (Some(&min), Some(&max)) = (mesh.indices.iter().min(), mesh.indices.iter().max()) {
                println!("  索引范围: [{}..={}]", min, max);
            }
        }
        Err(e) => {
            println!("\n[输入 PlantMesh] ❌ 解析失败: {}", e);
        }
    }
    
    // 使用单位矩阵，不进行额外变换
    let mat = DMat4::IDENTITY;
    let more_precision = false;

    let run_once = |more_precision: bool| -> anyhow::Result<()> {
        println!("\n[Manifold] more_precision={}", more_precision);
        match ManifoldRust::import_glb_to_manifold(glb_path, mat, more_precision) {
            Ok(manifold) => {
                let mesh = manifold.get_mesh();
                let vert_count = mesh.vertices.len() / 3;
                let tri_count = mesh.indices.len() / 3;
                
                println!("✅ 转换成功!");
                println!("  输出顶点数: {}", vert_count);
                println!("  输出三角形数: {}", tri_count);
                
                if tri_count == 0 {
                    println!("  ⚠️ 警告: 输出三角形数量为 0，转换可能失败。");
                } else {
                    println!("  ✨ 转换结果正常。");
                }

                if regen_model {
                    println!("\n🔄 开始测试布尔模型生成 (--regen-model)...");
                    // 创建一个小的位移矩阵作为“负实体”进行剪裁测试
                    let neg_mat = DMat4::from_translation(glam::DVec3::new(0.5, 0.5, 0.5));
                    let neg_manifold =
                        ManifoldRust::import_glb_to_manifold(glb_path, neg_mat, more_precision)?;
                    
                    println!("  正在执行布尔减法...");
                    let final_manifold = manifold.batch_boolean_subtract(&[neg_manifold]);
                    
                    let out_mesh_id = "test_boolean_result";
                    let output_path = Path::new("test_output").join(format!("{}.glb", out_mesh_id));
                    
                    if let Some(parent) = output_path.parent() {
                        std::fs::create_dir_all(parent)?;
                    }

                    println!("  正在导出结果到: {:?}", output_path);
                    final_manifold.export_to_glb(&output_path)?;
                    
                    let result_mesh = final_manifold.get_mesh();
                    println!("  ✅ 布尔生成成功!");
                    println!("    结果顶点数: {}", result_mesh.vertices.len() / 3);
                    println!("    结果三角形数: {}", result_mesh.indices.len() / 3);
                    println!("  ✨ 请检查 test_output/test_boolean_result.glb 是否正确。");
                }
                Ok(())
            }
            Err(e) => {
                println!("❌ 转换失败: {}", e);
                Ok(())
            }
        }
    };

    run_once(false)?;
    run_once(true)?;

    // 保留旧逻辑的输出结构（避免破坏脚本使用习惯）
    match ManifoldRust::import_glb_to_manifold(glb_path, mat, more_precision) {
        Ok(manifold) => {
            let mesh = manifold.get_mesh();
            let vert_count = mesh.vertices.len() / 3;
            let tri_count = mesh.indices.len() / 3;
            
            println!("\n✅ 转换成功!");
            println!("  输出顶点数: {}", vert_count);
            println!("  输出三角形数: {}", tri_count);
            
            if tri_count == 0 {
                println!("  ⚠️ 警告: 输出三角形数量为 0，转换可能失败。");
            } else {
                println!("  ✨ 转换结果正常。");
            }

            if regen_model {
                println!("\n🔄 开始测试布尔模型生成 (--regen-model)...");
                // 创建一个小的位移矩阵作为“负实体”进行剪裁测试
                let neg_mat = DMat4::from_translation(glam::DVec3::new(0.5, 0.5, 0.5));
                let neg_manifold = ManifoldRust::import_glb_to_manifold(glb_path, neg_mat, more_precision)?;
                
                println!("  正在执行布尔减法...");
                let final_manifold = manifold.batch_boolean_subtract(&[neg_manifold]);
                
                let out_mesh_id = "test_boolean_result";
                let output_path = Path::new("test_output").join(format!("{}.glb", out_mesh_id));
                
                if let Some(parent) = output_path.parent() {
                    std::fs::create_dir_all(parent)?;
                }

                println!("  正在导出结果到: {:?}", output_path);
                final_manifold.export_to_glb(&output_path)?;
                
                let result_mesh = final_manifold.get_mesh();
                println!("  ✅ 布尔生成成功!");
                println!("    结果顶点数: {}", result_mesh.vertices.len() / 3);
                println!("    结果三角形数: {}", result_mesh.indices.len() / 3);
                println!("  ✨ 请检查 test_output/test_boolean_result.glb 是否正确。");
            }
        }
        Err(e) => {
            println!("\n❌ 转换失败: {}", e);
        }
    }

    Ok(())
}
