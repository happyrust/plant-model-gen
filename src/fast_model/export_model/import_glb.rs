use std::path::Path;
use anyhow::{Result, Context, anyhow};
use aios_core::shape::pdms_shape::PlantMesh;
use glam::{Vec3, Vec2};

pub fn import_glb_to_mesh(path: &Path) -> Result<PlantMesh> {
    let (document, buffers, _) = gltf::import(path)
        .with_context(|| format!("Failed to import GLB from {:?}", path))?;

    let mut vertices = Vec::new();
    let mut normals = Vec::new();
    let mut indices = Vec::new();
    // 目前暂不处理 UV，如需要可添加
    let mut uv = Vec::new();

    for mesh in document.meshes() {
        for primitive in mesh.primitives() {
            let reader = primitive.reader(|buffer| Some(&buffers[buffer.index()]));

            if let Some(iter) = reader.read_positions() {
                for v in iter {
                    vertices.push(Vec3::new(v[0], v[1], v[2]));
                }
            }

            if let Some(iter) = reader.read_normals() {
                for v in iter {
                    normals.push(Vec3::new(v[0], v[1], v[2]));
                }
            }

            if let Some(iter) = reader.read_indices() {
                indices.extend(iter.into_u32());
            }
            
             // 简单的 UV 读取逻辑（如果存在）
            if let Some(iter) = reader.read_tex_coords(0) {
                 for v in iter.into_f32() {
                    uv.push(Vec2::new(v[0], v[1]));
                }
            }
            
            // 注意：如果多个 primitive，需要处理 vertices 偏移，这里 simplified 假设单个 mesh/primitive 或者直接合并
            // 如果 glTF indices 是相对于 primitive 0 的，则 merge 时需要 offset indices。
            // reader.read_indices() return indices relative to current primitive vertices context?
            // "The indices are relative to the vertices of the current primitive."
            // So if we merge vertices directly, we MUST update indices by `vertices.len()` offset BEFORE pushing new vertices.
            // BUT here we interpret one GLB as one PlantMesh.
            // Our generator produces one primitive per GLB usually. 
            // If complex GLB, we need careful merging.
            // Assuming simplified case: 1 primitive or simple appends.
            // Wait, if I iterate primitives:
            // P1: V[0..10], I[0..5]
            // P2: V[0..10], I[0..5]
            // Output V needs to append P2's V. Output I needs to offset P2's I by P1's V count.
            
            // Re-implement correctly:
        }
    }
    
    // Correct implementation treating multiple primitives
    let mut all_vertices = Vec::new();
    let mut all_normals = Vec::new();
    let mut all_indices = Vec::new();
    let mut all_uvs = Vec::new();
    
    for mesh in document.meshes() {
        for primitive in mesh.primitives() {
            let reader = primitive.reader(|buffer| Some(&buffers[buffer.index()]));
            
            let vertex_offset = all_vertices.len() as u32;
            
            if let Some(iter) = reader.read_positions() {
                for v in iter {
                    all_vertices.push(Vec3::new(v[0], v[1], v[2]));
                }
            }
            
            if let Some(iter) = reader.read_normals() {
                 for v in iter {
                    all_normals.push(Vec3::new(v[0], v[1], v[2]));
                }
            }
            
             if let Some(iter) = reader.read_tex_coords(0) {
                 for v in iter.into_f32() {
                    all_uvs.push(Vec2::new(v[0], v[1]));
                }
            }
            
            if let Some(iter) = reader.read_indices() {
                for idx in iter.into_u32() {
                    all_indices.push(idx + vertex_offset);
                }
            }
        }
    }

    Ok(PlantMesh {
        vertices: all_vertices,
        normals: all_normals,
        indices: all_indices,
        uvs: all_uvs.iter().map(|v| v.to_array()).collect(),
        aabb: None, // 调用者可能重新计算
        edges: Vec::new(),
        wire_vertices: Vec::new(),
    })
}
