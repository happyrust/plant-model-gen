use aios_database::fast_model::export_model::import_glb::import_glb_to_mesh;
use glam::Vec3;
use std::collections::HashMap;
use std::path::Path;

fn main() {
    let path = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "assets/meshes/2.glb".to_string());
    let mesh = import_glb_to_mesh(Path::new(&path)).unwrap();

    println!("几何体: {path}");
    println!("  vertices: {}", mesh.vertices.len());
    println!("  indices: {}", mesh.indices.len());
    println!("  tris: {}", mesh.indices.len() / 3);

    // 1) 三角形退化检查（重复顶点/零面积）
    let mut degenerate_index = 0usize;
    let mut degenerate_area = 0usize;
    let mut min_area = f32::INFINITY;
    let mut max_area = 0.0f32;
    let mut sum_area = 0.0f64;

    // 2) 边统计：无向边计数（用于 watertight 检查）
    let mut undirected: HashMap<(u32, u32), u32> = HashMap::new();
    // 3) 有向边计数（用于方向一致性检查）
    let mut directed: HashMap<(u32, u32), u32> = HashMap::new();

    for tri in mesh.indices.chunks(3) {
        if tri.len() < 3 {
            continue;
        }
        let a = tri[0];
        let b = tri[1];
        let c = tri[2];

        if a == b || b == c || c == a {
            degenerate_index += 1;
            continue;
        }

        let (va, vb, vc) = (
            mesh.vertices[a as usize],
            mesh.vertices[b as usize],
            mesh.vertices[c as usize],
        );
        let area2 = (vb - va).cross(vc - va).length(); // 2*area
        if !area2.is_finite() || area2 <= 1e-8 {
            degenerate_area += 1;
        } else {
            min_area = min_area.min(area2);
            max_area = max_area.max(area2);
            sum_area += area2 as f64;
        }

        // 边：三条
        let edges = [(a, b), (b, c), (c, a)];
        for (u, v) in edges {
            *directed.entry((u, v)).or_insert(0) += 1;
            let (m0, m1) = if u < v { (u, v) } else { (v, u) };
            *undirected.entry((m0, m1)).or_insert(0) += 1;
        }
    }

    let tri_cnt = mesh.indices.len() / 3;
    let valid_area_cnt = tri_cnt.saturating_sub(degenerate_index) - degenerate_area;
    let mean_area = if valid_area_cnt > 0 {
        sum_area / (valid_area_cnt as f64)
    } else {
        0.0
    };

    println!("  degenerate(tri index重复): {}", degenerate_index);
    println!("  degenerate(tri 零面积/非有限): {}", degenerate_area);
    if valid_area_cnt > 0 {
        println!(
            "  area2(min/mean/max): {:.6}/{:.6}/{:.6}",
            min_area, mean_area, max_area
        );
    }

    // 无向边分布：理想闭合体应全部为 2
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
    println!("  undirected edges: {}", undirected.len());
    println!("    - count=1 (boundary): {}", e1);
    println!("    - count=2 (manifold): {}", e2);
    println!("    - count>2 (non-manifold): {}", e_gt2);

    // 有向边方向一致性：对于闭合定向流形，(u->v) 与 (v->u) 应各出现 1 次
    let mut dir_bad = 0usize;
    for (&(u, v), &cnt_uv) in &directed {
        let cnt_vu = directed.get(&(v, u)).copied().unwrap_or(0);
        if cnt_uv != 1 || cnt_vu != 1 {
            // 只统计一次，避免重复：约束 u < v
            if u < v {
                dir_bad += 1;
            }
        }
    }
    println!("  directed edge pairs with (uv!=1 or vu!=1): {}", dir_bad);

    // 额外：估一个中心方向，便于快速 sanity
    if !mesh.vertices.is_empty() {
        let mut center = Vec3::ZERO;
        for &v in &mesh.vertices {
            center += v;
        }
        center /= mesh.vertices.len() as f32;
        println!(
            "  center approx: ({:.3},{:.3},{:.3})",
            center.x, center.y, center.z
        );
    }
}
