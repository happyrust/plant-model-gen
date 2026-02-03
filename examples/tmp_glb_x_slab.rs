//! 临时 GLB x-slab 统计：用于判断布尔后是否仍存在 x≈0 的顶点（例如“切片未贯穿”）。
//!
//! 用法：
//!   cargo run --example tmp_glb_x_slab -- path/to.glb

use aios_database::fast_model::export_model::import_glb::import_glb_to_mesh;
use std::path::Path;

fn main() -> anyhow::Result<()> {
    let path = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "assets/meshes/lod_L1/24381_131092_L1.glb".to_string());
    let mesh = import_glb_to_mesh(Path::new(&path))?;

    anyhow::ensure!(!mesh.vertices.is_empty(), "mesh.vertices 为空: {path}");

    fn axis_stats(mesh: &aios_core::shape::pdms_shape::PlantMesh, axis: char) -> (f32, f32, f32) {
        let get = |v: &glam::Vec3| match axis {
            'x' => v.x,
            'y' => v.y,
            'z' => v.z,
            _ => v.x,
        };
        let mut min_v = get(&mesh.vertices[0]);
        let mut max_v = get(&mesh.vertices[0]);
        for v in &mesh.vertices[1..] {
            let a = get(v);
            min_v = min_v.min(a);
            max_v = max_v.max(a);
        }
        let center = (min_v + max_v) / 2.0;
        (min_v, max_v, center)
    }

    println!("glb: {path}");
    println!("verts={} tris={}", mesh.vertices.len(), mesh.indices.len() / 3);

    for axis in ['x', 'y'] {
        let (min_v, max_v, center) = axis_stats(&mesh, axis);
        println!(
            "{axis}_min={:.6} {axis}_max={:.6} {axis}_center={:.6}",
            min_v, max_v, center
        );

        for w in [0.1_f32, 0.5, 1.0, 2.0, 5.0, 10.0, 15.0] {
            let cnt = mesh
                .vertices
                .iter()
                .filter(|v| {
                    let a = match axis {
                        'x' => v.x,
                        'y' => v.y,
                        _ => v.x,
                    };
                    (a - center).abs() <= w
                })
                .count();
            println!("  {axis}_slab_half_width={:.2} count={}", w, cnt);
        }

        let min_abs = mesh
            .vertices
            .iter()
            .map(|v| {
                let a = match axis {
                    'x' => v.x,
                    'y' => v.y,
                    _ => v.x,
                };
                (a - center).abs()
            })
            .fold(f32::INFINITY, |a, b| a.min(b));
        println!("  {axis}_min_abs_offset={:.9}", min_abs);
    }

    // 额外：看 x≈0 的顶点是否只出现在某个 z 范围（用于判断“只切了一半高度”）。
    let x_eps = 0.5_f32;
    let mut z_min = f32::INFINITY;
    let mut z_max = -f32::INFINITY;
    let mut z_cnt = 0usize;
    for v in &mesh.vertices {
        if v.x.abs() <= x_eps {
            z_min = z_min.min(v.z);
            z_max = z_max.max(v.z);
            z_cnt += 1;
        }
    }
    if z_cnt > 0 {
        println!(
            "x_abs<= {:.3}: cnt={} z_min={:.6} z_max={:.6}",
            x_eps, z_cnt, z_min, z_max
        );
    } else {
        println!("x_abs<= {:.3}: cnt=0", x_eps);
    }

    Ok(())
}
