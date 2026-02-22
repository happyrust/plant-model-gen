use std::path::Path;

use aios_core::csg::manifold::ManifoldRust;
use glam::DMat4;

fn main() {
    let path = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "assets/meshes/2.glb".to_string());

    let p = Path::new(&path);
    println!("GLB: {}", p.display());

    for &more_precision in &[false, true] {
        match ManifoldRust::import_glb_to_manifold(p, DMat4::IDENTITY, more_precision) {
            Ok(m) => {
                let mesh = m.get_mesh();
                println!(
                    "  more_precision={}: verts={} tris={}",
                    more_precision,
                    mesh.vertices.len(),
                    mesh.indices.len() / 3
                );
            }
            Err(e) => {
                println!("  more_precision={}: ERROR: {}", more_precision, e);
            }
        }
    }
}
