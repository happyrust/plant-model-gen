use aios_database::fast_model::export_model::import_glb::import_glb_to_mesh;
use std::path::Path;

fn main() {
    let mesh = import_glb_to_mesh(Path::new("assets/meshes/2.glb")).unwrap();
    println!("几何体 2:");
    println!("  vertices: {}", mesh.vertices.len());
    println!("  normals: {}", mesh.normals.len());
    println!("  indices: {}", mesh.indices.len());
    if let (Some(&min), Some(&max)) = (mesh.indices.iter().min(), mesh.indices.iter().max()) {
        println!("  索引范围: [{}..={}]", min, max);
    }
    println!("  前10个索引: {:?}", &mesh.indices[..mesh.indices.len().min(10)]);
}

