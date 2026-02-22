use aios_database::fast_model::export_model::import_glb::import_glb_to_mesh;
use glam::Vec3;
use std::collections::HashMap;
use std::path::Path;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
struct QKey(i64, i64, i64);

fn quantize(v: Vec3, eps: f64) -> QKey {
    let inv = 1.0f64 / eps;
    QKey(
        (v.x as f64 * inv).round() as i64,
        (v.y as f64 * inv).round() as i64,
        (v.z as f64 * inv).round() as i64,
    )
}

fn main() {
    let path = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "assets/meshes/2.glb".to_string());
    let eps: f64 = std::env::args()
        .nth(2)
        .and_then(|s| s.parse::<f64>().ok())
        .unwrap_or(1e-6);

    let mesh = import_glb_to_mesh(Path::new(&path)).unwrap();
    println!("几何体: {path}");
    println!("  eps: {eps:.3e}");
    println!(
        "  raw: verts={} tris={}",
        mesh.vertices.len(),
        mesh.indices.len() / 3
    );

    // 量化焊接：顶点 -> canonical index
    let mut map: HashMap<QKey, u32> = HashMap::new();
    let mut welded_vertices: Vec<Vec3> = Vec::new();
    let mut remap: Vec<u32> = Vec::with_capacity(mesh.vertices.len());

    for &v in &mesh.vertices {
        let k = quantize(v, eps);
        if let Some(&idx) = map.get(&k) {
            remap.push(idx);
        } else {
            let idx = welded_vertices.len() as u32;
            welded_vertices.push(v);
            map.insert(k, idx);
            remap.push(idx);
        }
    }

    // remap indices
    let mut welded_indices: Vec<u32> = Vec::with_capacity(mesh.indices.len());
    for &i in &mesh.indices {
        welded_indices.push(remap[i as usize]);
    }

    // 统计边（无向）
    let mut undirected: HashMap<(u32, u32), u32> = HashMap::new();
    let mut degenerate = 0usize;
    for tri in welded_indices.chunks(3) {
        if tri.len() < 3 {
            continue;
        }
        let a = tri[0];
        let b = tri[1];
        let c = tri[2];
        if a == b || b == c || c == a {
            degenerate += 1;
            continue;
        }
        for (u, v) in [(a, b), (b, c), (c, a)] {
            let (m0, m1) = if u < v { (u, v) } else { (v, u) };
            *undirected.entry((m0, m1)).or_insert(0) += 1;
        }
    }

    let mut e1 = 0usize;
    let mut e2 = 0usize;
    let mut e_gt2 = 0usize;
    for &cnt in undirected.values() {
        if cnt == 1 {
            e1 += 1;
        } else if cnt == 2 {
            e2 += 1;
        } else if cnt > 2 {
            e_gt2 += 1;
        }
    }

    println!(
        "  welded: verts={} tris={} degenerate={}",
        welded_vertices.len(),
        welded_indices.len() / 3,
        degenerate
    );
    println!("  undirected edges: {}", undirected.len());
    println!("    - count=1 (boundary): {}", e1);
    println!("    - count=2 (manifold): {}", e2);
    println!("    - count>2 (non-manifold): {}", e_gt2);
}
