fn main() {
    fn mock_build_path(base_dir: &std::path::Path, default_lod: &str, mesh_id: &str) -> std::path::PathBuf {
         let mut clean_base = base_dir.to_path_buf();
         while let Some(last_component) = clean_base.file_name().and_then(|n| n.to_str()) {
             if last_component.starts_with("lod_") {
                 clean_base.pop();
             } else {
                 break;
             }
         }
         let lod_dir_name = format!("lod_{}", default_lod);
         let lod_filename = format!("{}_{}.glb", mesh_id, default_lod);
         clean_base.join(lod_dir_name).join(lod_filename)
    }

    let mesh_id = "12345";

    // 测试 1: 正常基础路径
    let base1 = std::path::Path::new("./assets/meshes");
    println!("Test 1 (base): {:?} -> {:?}", base1, mock_build_path(base1, "L1", mesh_id));

    // 测试 2: 已经是某种 LOD 路径 (L1) 但目标是 L2
    let base2 = std::path::Path::new("./assets/meshes/lod_L1");
    println!("Test 2 (nested): {:?} -> {:?}", base2, mock_build_path(base2, "L2", mesh_id));

    // 测试 3: 多级嵌套路径 (模拟 Bug 产生的场景)
    let base3 = std::path::Path::new("./assets/meshes/lod_L1/lod_L2");
    println!("Test 3 (double nested): {:?} -> {:?}", base3, mock_build_path(base3, "L2", mesh_id));
}
