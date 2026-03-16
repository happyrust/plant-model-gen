use crate::fast_model::EXIST_MESH_GEO_HASHES;
use parry3d::bounding_volume::Aabb;
use std::path::{Path, PathBuf};

pub const MESH_STATE_SOURCE_ENV: &str = "MESH_STATE_SOURCE";
pub const MESH_STATE_SOURCE_FILE: &str = "file";

pub fn use_file_mesh_state() -> bool {
    matches!(
        std::env::var(MESH_STATE_SOURCE_ENV).ok().as_deref(),
        Some(MESH_STATE_SOURCE_FILE)
    )
}

pub fn flush_aabb_cache() {
    crate::fast_model::save_aabb_cache_to_disk();
}

pub fn mesh_exists(geo_hash: u64) -> bool {
    find_existing_mesh_path(&aios_core::get_db_option().get_meshes_path(), geo_hash).is_some()
}

pub fn get_cached_or_local_aabb(geo_hash: u64) -> Option<Aabb> {
    let mesh_dir = aios_core::get_db_option().get_meshes_path();
    get_cached_or_local_aabb_in_dir(&mesh_dir, geo_hash)
}

pub fn get_cached_or_local_aabb_in_dir(mesh_dir: &Path, geo_hash: u64) -> Option<Aabb> {
    let mesh_path = find_existing_mesh_path(mesh_dir, geo_hash)?;
    let key = geo_hash.to_string();
    if let Some(cached_aabb) = EXIST_MESH_GEO_HASHES.get(&key) {
        let cached = *cached_aabb;
        if is_valid_cached_aabb(&cached) {
            return Some(cached);
        }
    }

    let computed = load_local_mesh_aabb_from_glb(&mesh_path)?;
    EXIST_MESH_GEO_HASHES.insert(key, computed);
    Some(computed)
}

pub fn prime_cached_aabb_for_mesh_ids<'a>(mesh_ids: impl IntoIterator<Item = &'a str>) {
    if !use_file_mesh_state() {
        return;
    }

    for mesh_id in mesh_ids {
        let Some(geo_hash) = mesh_id.parse::<u64>().ok() else {
            continue;
        };
        if matches!(geo_hash, 1 | 2 | 3) {
            continue;
        }
        let _ = get_cached_or_local_aabb(geo_hash);
    }
}

fn find_existing_mesh_path(mesh_dir: &Path, geo_hash: u64) -> Option<PathBuf> {
    if !mesh_dir.exists() {
        return None;
    }

    for lod in ["L0", "L1", "L2", "L3", "L4"] {
        let path = mesh_dir.join(format!("lod_{lod}/{geo_hash}_{lod}.glb"));
        if path.exists() {
            return Some(path);
        }
    }
    None
}

fn is_valid_cached_aabb(aabb: &Aabb) -> bool {
    let ext_mag = aabb.extents().magnitude();
    ext_mag > 1e-4 && ext_mag < f32::INFINITY
}

fn load_local_mesh_aabb_from_glb(path: &Path) -> Option<Aabb> {
    let mesh = crate::fast_model::export_model::import_glb::import_glb_to_mesh(path).ok()?;
    let mut iter = mesh.vertices.iter();
    let first = *iter.next()?;
    let mut min = first;
    let mut max = first;
    for v in iter {
        min = min.min(*v);
        max = max.max(*v);
    }
    Some(Aabb::new(
        parry3d::math::Point::new(min.x, min.y, min.z),
        parry3d::math::Point::new(max.x, max.y, max.z),
    ))
}
