use aios_database::fast_model::export_model::import_glb::import_glb_to_mesh;
use std::collections::HashMap;
use std::path::Path;

fn main() -> anyhow::Result<()> {
    let args: Vec<String> = std::env::args().collect();
    let glb_path = args
        .iter()
        .position(|x| x == "--path")
        .and_then(|i| args.get(i + 1))
        .map(|s| s.as_str())
        .ok_or_else(|| anyhow::anyhow!("用法: cargo run --bin check_glb_manifold -- --path <file.glb>"))?;

    let glb_path = Path::new(glb_path);
    let mesh = import_glb_to_mesh(glb_path)?;

    let v = mesh.vertices.len() as u32;
    let idx = &mesh.indices;
    let tri = idx.len() / 3;

    println!("[mesh]");
    println!("  path: {}", glb_path.display());
    println!("  vertices: {}", mesh.vertices.len());
    println!("  indices: {}", mesh.indices.len());
    println!("  triangles: {}", tri);
    if idx.len() % 3 != 0 {
        println!("  ⚠️ indices.len % 3 != 0");
    }
    if let (Some(&min), Some(&max)) = (idx.iter().min(), idx.iter().max()) {
        println!("  index_range: [{}..={}], vertex_count={}", min, max, v);
    }

    let mut edge_counts: HashMap<(u32, u32), u32> = HashMap::new();
    let mut degenerate_triangles = 0usize;
    let mut out_of_range_indices = 0usize;

    for t in 0..tri {
        let a = idx[t * 3];
        let b = idx[t * 3 + 1];
        let c = idx[t * 3 + 2];

        if a >= v || b >= v || c >= v {
            out_of_range_indices += 1;
            continue;
        }
        if a == b || b == c || c == a {
            degenerate_triangles += 1;
            continue;
        }

        let e1 = if a < b { (a, b) } else { (b, a) };
        let e2 = if b < c { (b, c) } else { (c, b) };
        let e3 = if c < a { (c, a) } else { (a, c) };
        *edge_counts.entry(e1).or_insert(0) += 1;
        *edge_counts.entry(e2).or_insert(0) += 1;
        *edge_counts.entry(e3).or_insert(0) += 1;
    }

    let mut boundary_edges = 0usize;
    let mut nonmanifold_edges = 0usize;
    let mut boundary_list: Vec<(u32, u32)> = Vec::new();
    for (_, c) in edge_counts.iter() {
        if *c == 1 {
            boundary_edges += 1;
        } else if *c > 2 {
            nonmanifold_edges += 1;
        }
    }

    if boundary_edges > 0 && boundary_edges <= 64 {
        for (&e, &c) in edge_counts.iter() {
            if c == 1 {
                boundary_list.push(e);
            }
        }
        boundary_list.sort_unstable();
    }

    println!("\n[topology]");
    println!("  degenerate_triangles: {}", degenerate_triangles);
    println!("  out_of_range_triangles: {}", out_of_range_indices);
    println!("  unique_edges: {}", edge_counts.len());
    println!("  boundary_edges(count==1): {}", boundary_edges);
    println!("  nonmanifold_edges(count>2): {}", nonmanifold_edges);
    println!(
        "  watertight_heuristic: {}",
        boundary_edges == 0 && nonmanifold_edges == 0 && degenerate_triangles == 0 && out_of_range_indices == 0
    );

    if !boundary_list.is_empty() {
        println!("\n[boundary_edges_sample]");
        for (i, (a, b)) in boundary_list.iter().take(20).enumerate() {
            let va = mesh.vertices[*a as usize];
            let vb = mesh.vertices[*b as usize];
            println!(
                "  {:02}: ({:>4},{:>4})  A=({:>8.3},{:>8.3},{:>8.3})  B=({:>8.3},{:>8.3},{:>8.3})",
                i,
                a,
                b,
                va.x,
                va.y,
                va.z,
                vb.x,
                vb.y,
                vb.z
            );
        }
    }

    Ok(())
}
