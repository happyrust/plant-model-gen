fn main() {
    // 模拟 meshes_path = "./assets/meshes/lod_L1" 这种情况
    let base_dir = std::path::Path::new("./assets/meshes/lod_L1");
    
    // aios-database/src/fast_model/manifold_bool.rs 中的逻辑
    let is_already_lod_dir = base_dir
        .file_name()
        .and_then(|n| n.to_str())
        .map(|s| s.starts_with("lod_"))
        .unwrap_or(false);

    println!("base_dir: {:?}", base_dir);
    println!("is_already_lod_dir: {}", is_already_lod_dir);

    let default_lod = "L2"; 
    let mesh_id = "12345";
    let lod_filename = format!("{}_{}.glb", mesh_id, default_lod);

    let path = if is_already_lod_dir {
        base_dir.join(lod_filename)
    } else {
        let lod_dir = base_dir.join(format!("lod_{}", default_lod));
        lod_dir.join(lod_filename)
    };
    println!("Resulting path from manifold_bool/build_lod_mesh_path: {:?}", path);

    // 接下来模拟嵌套调用的情况，即 base_dir 如果已经是 ./assets/meshes/lod_L1/lod_L2
    let nested_base_dir = std::path::Path::new("./assets/meshes/lod_L1/lod_L2");
    let is_already_lod_dir_nested = nested_base_dir
        .file_name()
        .and_then(|n| n.to_str())
        .map(|s| s.starts_with("lod_"))
        .unwrap_or(false);
    
    println!("nested_base_dir: {:?}", nested_base_dir);
    println!("is_already_lod_dir_nested: {}", is_already_lod_dir_nested);
}
