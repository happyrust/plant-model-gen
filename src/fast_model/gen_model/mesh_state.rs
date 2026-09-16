use crate::fast_model::EXIST_MESH_GEO_HASHES;
use parry3d::bounding_volume::Aabb;
use std::path::{Path, PathBuf};

pub const MESH_STATE_SOURCE_ENV: &str = "MESH_STATE_SOURCE";
pub const MESH_STATE_SOURCE_FILE: &str = "file";

/// 跨 run 的 mesh 存在性只由内容寻址资产文件决定，不能依赖模型数据库状态。
#[derive(Debug, Clone)]
pub struct MeshAssetStore {
    base_dir: PathBuf,
    default_lod: String,
}

impl MeshAssetStore {
    pub fn new(mesh_dir: impl AsRef<Path>) -> Self {
        Self {
            base_dir: normalize_mesh_base_dir(mesh_dir.as_ref()),
            default_lod: format!(
                "{:?}",
                aios_core::get_db_option().mesh_precision().default_lod
            ),
        }
    }

    pub fn contains(&self, geo_hash: u64) -> bool {
        if matches!(geo_hash, 1 | 2 | 3) {
            return true;
        }
        self.path(geo_hash).is_some()
    }

    fn path(&self, geo_hash: u64) -> Option<PathBuf> {
        let hash = geo_hash.to_string();
        let lod_dir = self.base_dir.join(format!("lod_{}", self.default_lod));
        [
            lod_dir.join(format!("{}_{}.glb", hash, self.default_lod)),
            lod_dir.join(format!("{}.glb", hash)),
            self.base_dir.join(format!("{}.glb", hash)),
        ]
        .into_iter()
        .find(|path| path.is_file())
    }
}

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
    MeshAssetStore::new(mesh_dir).contains(geo_hash)
}

pub fn get_cached_or_local_aabb(geo_hash: u64) -> Option<Aabb> {
    let mesh_dir = aios_core::get_db_option().get_meshes_path();
    get_cached_or_local_aabb_in_dir(&mesh_dir, geo_hash)
}

pub fn get_cached_or_local_aabb_in_dir(mesh_dir: &Path, geo_hash: u64) -> Option<Aabb> {
    let store = MeshAssetStore::new(mesh_dir);
    if !store.contains(geo_hash) {
        return None;
    }
    let key = geo_hash.to_string();
    if let Some(cached_aabb) = EXIST_MESH_GEO_HASHES.get(&key) {
        let cached = *cached_aabb;
        if is_valid_cached_aabb(&cached) {
            return Some(cached);
        }
    }

    let mesh = crate::fast_model::export_model::import_glb::import_glb_to_mesh(
        &store.path(geo_hash)?,
    )
    .ok()?;
    let mut aabb = Aabb::new_invalid();
    for vertex in mesh.vertices {
        aabb.take_point(vertex.into());
    }
    if !is_valid_cached_aabb(&aabb) {
        return None;
    }
    EXIST_MESH_GEO_HASHES.insert(key, aabb);
    Some(aabb)
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
    MeshAssetStore::new(mesh_dir).contains(geo_hash)
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
