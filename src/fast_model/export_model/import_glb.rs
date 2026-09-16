use aios_core::shape::pdms_shape::PlantMesh;
use anyhow::{Context, Result, anyhow};
use glam::{Vec2, Vec3};
use std::path::Path;

pub fn import_glb_to_mesh(path: &Path) -> Result<PlantMesh> {
    let (document, buffers, _) =
        gltf::import(path).with_context(|| format!("Failed to import GLB from {:?}", path))?;

    use gltf::Semantic;

    // 正确实现：支持多个 primitive，且提前检查 accessor count，避免 gltf crate 在 count=0 时 panic。
    let mut all_vertices = Vec::new();
    let mut all_normals = Vec::new();
    let mut all_indices = Vec::new();
    let mut all_uvs = Vec::new();

    for mesh in document.meshes() {
        for primitive in mesh.primitives() {
            let Some(pos_accessor) = primitive.get(&Semantic::Positions) else {
                return Err(anyhow!("GLB 缺少 POSITION accessor: {:?}", path));
            };
            if pos_accessor.count() == 0 {
                return Err(anyhow!("GLB POSITION accessor 为空(count=0): {:?}", path));
            }
            if let Some(idx_accessor) = primitive.indices() {
                if idx_accessor.count() == 0 {
                    return Err(anyhow!("GLB INDICES accessor 为空(count=0): {:?}", path));
                }
            } else {
                return Err(anyhow!("GLB 缺少 INDICES accessor: {:?}", path));
            }

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

    if all_vertices.is_empty() || all_indices.is_empty() {
        return Err(anyhow!(
            "GLB 解析结果为空：vertices={} indices={} path={:?}",
            all_vertices.len(),
            all_indices.len(),
            path
        ));
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
