use aios_database::fast_model::export_model::import_glb::import_glb_to_mesh;
use std::path::Path;

fn main() {
    let path = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "assets/meshes/2.glb".to_string());
    let mesh = import_glb_to_mesh(Path::new(&path)).unwrap();

    println!("几何体: {path}");
    println!("  vertices: {}", mesh.vertices.len());
    println!("  normals: {}", mesh.normals.len());
    println!("  indices: {}", mesh.indices.len());
    if let (Some(&min), Some(&max)) = (mesh.indices.iter().min(), mesh.indices.iter().max()) {
        println!("  索引范围: [{}..={}]", min, max);
    }
    println!(
        "  前10个索引: {:?}",
        &mesh.indices[..mesh.indices.len().min(10)]
    );

    // 顶点范围（快速判断是否出现“尺寸被意外放大/缩小”）
    if !mesh.vertices.is_empty() {
        let mut min = mesh.vertices[0];
        let mut max = mesh.vertices[0];
        for v in &mesh.vertices[1..] {
            min.x = min.x.min(v.x);
            min.y = min.y.min(v.y);
            min.z = min.z.min(v.z);
            max.x = max.x.max(v.x);
            max.y = max.y.max(v.y);
            max.z = max.z.max(v.z);
        }
        println!(
            "  aabb(min/max): ({:.6},{:.6},{:.6}) / ({:.6},{:.6},{:.6})",
            min.x, min.y, min.z, max.x, max.y, max.z
        );
    }

    // GLB 常见“法线不对”原因有两类：
    // 1) normals 自身无效（0/NaN/未归一），或与面法线整体反向；
    // 2) 绕序整体翻面（体积符号为负），此时 normals 可能“自洽”但整体朝内，渲染会发黑/背面。
    if mesh.normals.len() == mesh.vertices.len() && !mesh.normals.is_empty() {
        // --- 法线数值统计 ---
        let mut zero_cnt = 0usize;
        let mut non_finite_cnt = 0usize;
        let mut non_finite_indices: Vec<usize> = Vec::new();
        let mut min_len = f32::INFINITY;
        let mut max_len = 0.0f32;
        let mut sum_len = 0.0f64;
        for n in &mesh.normals {
            if !(n.x.is_finite() && n.y.is_finite() && n.z.is_finite()) {
                non_finite_cnt += 1;
                continue;
            }
            let len = n.length();
            if len <= f32::EPSILON {
                zero_cnt += 1;
            }
            min_len = min_len.min(len);
            max_len = max_len.max(len);
            sum_len += len as f64;
        }
        let mean_len = sum_len / (mesh.normals.len() as f64);
        println!("  normals 统计:");
        println!("    - zero: {}", zero_cnt);
        println!("    - non_finite: {}", non_finite_cnt);
        println!(
            "    - len(min/mean/max): {:.6}/{:.6}/{:.6}",
            min_len, mean_len, max_len
        );

        // 打印 non-finite 的具体法线与关联信息，方便定位上游生成问题。
        for (i, n) in mesh.normals.iter().enumerate() {
            if !(n.x.is_finite() && n.y.is_finite() && n.z.is_finite()) {
                non_finite_indices.push(i);
            }
        }
        if !non_finite_indices.is_empty() {
            println!("  non_finite normals 顶点索引: {:?}", non_finite_indices);
            for &i in non_finite_indices.iter().take(8) {
                let v = mesh.vertices.get(i).copied().unwrap_or_default();
                println!(
                    "    - i={i} v=({:.6},{:.6},{:.6}) n=({},{},{})",
                    v.x, v.y, v.z, mesh.normals[i].x, mesh.normals[i].y, mesh.normals[i].z
                );
            }

            // 这些顶点被多少个三角形引用？
            for &i in non_finite_indices.iter().take(8) {
                let mut ref_cnt = 0u32;
                for &idx in &mesh.indices {
                    if idx as usize == i {
                        ref_cnt += 1;
                    }
                }
                println!("    - i={i} 被 indices 引用次数: {ref_cnt}");
            }

            // 列出包含这些顶点的三角形（最多前 10 个）
            let mut listed = 0u32;
            for (t, tri) in mesh.indices.chunks(3).enumerate() {
                if tri.len() < 3 {
                    continue;
                }
                let a = tri[0] as usize;
                let b = tri[1] as usize;
                let c = tri[2] as usize;
                if non_finite_indices.contains(&a)
                    || non_finite_indices.contains(&b)
                    || non_finite_indices.contains(&c)
                {
                    println!("    - tri#{t}: [{a}, {b}, {c}]");
                    listed += 1;
                    if listed >= 10 {
                        break;
                    }
                }
            }
        }

        // --- 以中心近似判断“整体朝向” ---
        let mut center = glam::Vec3::ZERO;
        for &v in &mesh.vertices {
            center += v;
        }
        center /= mesh.vertices.len().max(1) as f32;

        let mut dir_checked = 0u32;
        let mut dir_negative = 0u32;
        let mut dir_sum = 0.0f64;
        for (v, n) in mesh.vertices.iter().zip(mesh.normals.iter()) {
            if !(n.x.is_finite() && n.y.is_finite() && n.z.is_finite()) {
                continue;
            }
            if n.length_squared() <= f32::EPSILON {
                continue;
            }
            let to_v = *v - center;
            if to_v.length_squared() <= f32::EPSILON {
                continue;
            }
            let d = n.normalize().dot(to_v.normalize());
            dir_checked += 1;
            dir_sum += d as f64;
            if d < 0.0 {
                dir_negative += 1;
            }
        }
        if dir_checked > 0 {
            println!(
                "  朝向估计(法线·(v-center))<0: {}/{} (avg_dot={:.4})",
                dir_negative,
                dir_checked,
                dir_sum / (dir_checked as f64)
            );
        }

        // --- 面法线一致性（用于检测“面法线与顶点法线相反”）---
        let mut checked = 0u32;
        let mut opposite = 0u32;
        let mut dot_sum = 0.0f64;
        let mut dot_min = 1.0f32;
        let mut dot_max = -1.0f32;
        for tri in mesh.indices.chunks(3) {
            if tri.len() < 3 {
                continue;
            }
            let i0 = tri[0] as usize;
            let i1 = tri[1] as usize;
            let i2 = tri[2] as usize;
            if i0 >= mesh.vertices.len() || i1 >= mesh.vertices.len() || i2 >= mesh.vertices.len() {
                continue;
            }
            let v0 = mesh.vertices[i0];
            let v1 = mesh.vertices[i1];
            let v2 = mesh.vertices[i2];
            let face_n = (v1 - v0).cross(v2 - v0);
            if face_n.length_squared() <= f32::EPSILON {
                continue;
            }
            let face_n = face_n.normalize();

            let avg_vn = (mesh.normals[i0] + mesh.normals[i1] + mesh.normals[i2]) / 3.0;
            if avg_vn.length_squared() <= f32::EPSILON {
                continue;
            }
            let avg_vn = avg_vn.normalize();

            let d = face_n.dot(avg_vn);
            checked += 1;
            dot_sum += d as f64;
            dot_min = dot_min.min(d);
            dot_max = dot_max.max(d);
            if d < 0.0 {
                opposite += 1;
            }
        }
        if checked > 0 {
            let ratio = (opposite as f64) * 100.0 / (checked as f64);
            println!(
                "  面法线一致性(面法线·顶点法线<0): {opposite}/{checked} ({ratio:.2}%), dot(min/avg/max)={:.4}/{:.4}/{:.4}",
                dot_min,
                dot_sum / (checked as f64),
                dot_max
            );
        }

        // --- 近似有符号体积（闭合体：负值通常意味着“绕序整体翻面”） ---
        let mut vol6 = 0.0f64;
        for tri in mesh.indices.chunks(3) {
            if tri.len() < 3 {
                continue;
            }
            let i0 = tri[0] as usize;
            let i1 = tri[1] as usize;
            let i2 = tri[2] as usize;
            if i0 >= mesh.vertices.len() || i1 >= mesh.vertices.len() || i2 >= mesh.vertices.len() {
                continue;
            }
            let v0 = mesh.vertices[i0];
            let v1 = mesh.vertices[i1];
            let v2 = mesh.vertices[i2];
            vol6 += (v0.dot(v1.cross(v2))) as f64;
        }
        println!(
            "  近似有符号体积(volume): {:.6} (sign={})",
            vol6 / 6.0,
            if vol6 >= 0.0 { "+" } else { "-" }
        );
    } else {
        println!("  顶点法线缺失或数量不匹配：跳过检查");
    }
}
