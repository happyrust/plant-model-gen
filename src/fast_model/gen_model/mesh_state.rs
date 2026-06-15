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
    let mesh_dir = aios_core::get_db_option().get_meshes_path();
    mesh_file_exists_in_dir(&mesh_dir, geo_hash)
}

pub fn get_cached_or_local_aabb(geo_hash: u64) -> Option<Aabb> {
    let mesh_dir = aios_core::get_db_option().get_meshes_path();
    get_cached_or_local_aabb_in_dir(&mesh_dir, geo_hash)
}

pub fn get_cached_or_local_aabb_in_dir(mesh_dir: &Path, geo_hash: u64) -> Option<Aabb> {
    if !mesh_file_exists_in_dir(mesh_dir, geo_hash) {
        return None;
    }
    let key = geo_hash.to_string();
    let cached_aabb = EXIST_MESH_GEO_HASHES.get(&key)?;
    let cached = *cached_aabb;
    if is_valid_cached_aabb(&cached) {
        Some(cached)
    } else {
        None
    }
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

pub fn mesh_file_exists_in_dir(mesh_dir: &Path, geo_hash: u64) -> bool {
    if matches!(geo_hash, 1 | 2 | 3) {
        return true;
    }

    let hash = geo_hash.to_string();
    let default_lod = format!(
        "{:?}",
        aios_core::get_db_option().mesh_precision().default_lod
    );
    let base_dir = normalize_mesh_base_dir(mesh_dir);
    let lod_dir = base_dir.join(format!("lod_{}", default_lod));
    [
        lod_dir.join(format!("{}_{}.glb", hash, default_lod)),
        lod_dir.join(format!("{}.glb", hash)),
        base_dir.join(format!("{}.glb", hash)),
    ]
    .iter()
    .any(|p| p.exists())
}

fn normalize_mesh_base_dir(mesh_dir: &Path) -> PathBuf {
    let is_lod_dir = mesh_dir
        .file_name()
        .map(|n| n.to_string_lossy().starts_with("lod_"))
        .unwrap_or(false);
    if is_lod_dir {
        mesh_dir.parent().unwrap_or(mesh_dir).to_path_buf()
    } else {
        mesh_dir.to_path_buf()
    }
}

fn is_valid_cached_aabb(aabb: &Aabb) -> bool {
    let ext_mag = aabb.extents().magnitude();
    ext_mag > 1e-4 && ext_mag < f32::INFINITY
}
