use std::collections::{HashMap, HashSet};
use std::hash::BuildHasherDefault;
use std::path::{Path, PathBuf};

use aios_core::{init_surreal, RefnoEnum, SUL_DB, SurrealQueryExt};
use bevy_transform::prelude::Transform;
use foyer::{DirectFsDeviceOptionsBuilder, HybridCache, HybridCacheBuilder};
use serde::{Deserialize, Serialize};
use surrealdb::types::SurrealValue;
use tokio::sync::OnceCell;
use twox_hash::XxHash64;

use crate::data_interface::db_meta_manager::db_meta;
use crate::options::DbOptionExt;

/// foyer transform 缓存：用于“模型生成阶段”读取/写入 world_transform，避免依赖 SurrealDB 的 pe_transform 预热。
///
/// 约定：
/// - 只要缓存存在即可（不要求全量命中）。
/// - miss 时，允许走旧路径按需计算/查询，然后回写到本地缓存。
/// - 与旧逻辑区分：旧逻辑走 aios_core 内部的 pe_transform / 惰性计算；新逻辑优先读本地 foyer。

#[derive(Clone, Hash, Eq, PartialEq, Serialize, Deserialize)]
pub struct TransformCacheKey {
    pub dbnum: u32,
    pub refno: RefnoEnum,
}

#[derive(Clone, Serialize, Deserialize)]
pub struct TransformCacheValue {
    pub payload: Vec<u8>,
}

#[derive(Clone, Serialize, Deserialize)]
pub struct CachedWorldTransform {
    pub refno: RefnoEnum,
    pub world: Transform,
    pub created_at: i64,
}

pub struct TransformCacheManager {
    cache: HybridCache<TransformCacheKey, TransformCacheValue, BuildHasherDefault<XxHash64>>,
    cache_dir: PathBuf,
}

impl TransformCacheManager {
    pub async fn new(cache_dir: &Path) -> anyhow::Result<Self> {
        if !cache_dir.exists() {
            std::fs::create_dir_all(cache_dir)?;
        }

        // 变换缓存通常比 instance_cache 小很多，先给一个中等容量即可。
        let device_config = DirectFsDeviceOptionsBuilder::new(cache_dir)
            .with_capacity(512 * 1024 * 1024)
            .build();

        let cache = HybridCacheBuilder::new()
            .memory(64 * 1024 * 1024)
            .with_hash_builder(BuildHasherDefault::<XxHash64>::default())
            .storage()
            .with_device_config(device_config)
            .build()
            .await?;

        Ok(Self {
            cache,
            cache_dir: cache_dir.to_path_buf(),
        })
    }

    pub fn cache_dir(&self) -> &Path {
        &self.cache_dir
    }

    pub async fn get_world_transform(&self, dbnum: u32, refno: RefnoEnum) -> Option<Transform> {
        let key = TransformCacheKey { dbnum, refno };
        match self.cache.get(&key).await {
            Ok(Some(entry)) => {
                let payload = &entry.value().payload;
                serde_json::from_slice::<CachedWorldTransform>(payload)
                    .ok()
                    .map(|v| v.world)
            }
            _ => None,
        }
    }

    /// 删除指定 key 的 transform 缓存
    pub fn remove(&self, dbnum: u32, refno: RefnoEnum) {
        let key = TransformCacheKey { dbnum, refno };
        self.cache.remove(&key);
    }

    pub fn insert_world_transform(&self, dbnum: u32, refno: RefnoEnum, world: Transform) {
        let key = TransformCacheKey { dbnum, refno };
        let item = CachedWorldTransform {
            refno,
            world,
            created_at: chrono::Utc::now().timestamp_millis(),
        };
        let payload = match serde_json::to_vec(&item) {
            Ok(v) => v,
            Err(e) => {
                eprintln!(
                    "[transform_cache] 序列化失败，跳过写入: dbnum={}, refno={}, err={}",
                    dbnum, refno, e
                );
                return;
            }
        };
        self.cache.insert(key, TransformCacheValue { payload });
    }
}

static GLOBAL_TRANSFORM_CACHE: OnceCell<TransformCacheManager> = OnceCell::const_new();

pub fn transform_cache_dir_for_option(db_option: &DbOptionExt) -> PathBuf {
    // 与 instance_cache 同根目录，但使用子目录隔离，避免多个 foyer cache 共享同一 device 目录。
    db_option.get_foyer_cache_dir().join("transform_cache")
}

pub fn default_transform_cache_dir() -> PathBuf {
    // 运行时约定：若未提供 DbOptionExt，则按环境变量 FOYER_CACHE_DIR 或默认 output/instance_cache 推导。
    let base = std::env::var("FOYER_CACHE_DIR")
        .ok()
        .filter(|s| !s.trim().is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("output/instance_cache"));
    base.join("transform_cache")
}

pub fn ensure_transform_cache_dir(db_option: &DbOptionExt) -> anyhow::Result<PathBuf> {
    let dir = transform_cache_dir_for_option(db_option);
    if !dir.exists() {
        std::fs::create_dir_all(&dir)?;
    }
    Ok(dir)
}

pub async fn init_global_transform_cache(db_option: &DbOptionExt) -> anyhow::Result<()> {
    let dir = ensure_transform_cache_dir(db_option)?;
    let _ = GLOBAL_TRANSFORM_CACHE
        .get_or_try_init(|| async move { TransformCacheManager::new(&dir).await })
        .await?;
    Ok(())
}

async fn get_global_cache(db_option: Option<&DbOptionExt>) -> anyhow::Result<Option<&'static TransformCacheManager>> {
    if let Some(db_option) = db_option {
        init_global_transform_cache(db_option).await?;
        return Ok(GLOBAL_TRANSFORM_CACHE.get());
    }

    // 未传 DbOptionExt：尝试用默认路径初始化一次，保证“无配置上下文”的调用点也能 cache-first。
    if GLOBAL_TRANSFORM_CACHE.get().is_none() {
        let dir = default_transform_cache_dir();
        if !dir.exists() {
            let _ = std::fs::create_dir_all(&dir);
        }
        let _ = GLOBAL_TRANSFORM_CACHE
            .get_or_try_init(|| async move { TransformCacheManager::new(&dir).await })
            .await?;
    }
    Ok(GLOBAL_TRANSFORM_CACHE.get())
}

fn resolve_dbnum(refno: RefnoEnum) -> Option<u32> {
    if db_meta().ensure_loaded().is_ok() {
        if let Some(dbnum) = db_meta().get_dbnum_by_refno(refno) {
            return Some(dbnum);
        }
    }
    // ref0 != dbnum，禁止回退用 ref0 当 dbnum。
    log::warn!(
        "[transform_cache] 缺少 ref0->dbnum 映射: refno={}",
        refno
    );
    None
}

/// 模型生成专用：从 foyer transform cache 读取 world_transform；miss 时按需生成并回写缓存。
///
/// 与旧逻辑区分：旧逻辑直接调用 `aios_core::get_world_transform`（会优先查 pe_transform 表）。
pub async fn get_world_transform_cache_first(
    db_option: Option<&DbOptionExt>,
    refno: RefnoEnum,
) -> anyhow::Result<Option<Transform>> {
    let dbnum = resolve_dbnum(refno);
    let use_cache = db_option.map(|x| x.use_cache).unwrap_or(true) && dbnum.is_some();

    if use_cache {
        let dbnum = dbnum.unwrap();
        if let Some(cache) = get_global_cache(db_option).await? {
            if let Some(hit) = cache.get_world_transform(dbnum, refno).await {
                return Ok(Some(hit));
            }
        }
    }

    // miss：先用"直接读 pe.world_trans"的轻量路径（不依赖 pe_transform 预热）。
    if let Ok(Some(world)) = aios_core::rs_surreal::query_pe_world_trans(refno).await {
        if let (true, Some(dbnum)) = (use_cache, dbnum) {
            if let Some(cache) = GLOBAL_TRANSFORM_CACHE.get() {
                cache.insert_world_transform(dbnum, refno, world.clone());
            }
        }
        return Ok(Some(world));
    }

    // 再兜底走旧计算路径（策略/惰性计算）。
    let computed = aios_core::get_world_transform(refno).await?;
    if let Some(world) = computed.clone() {
        if let (true, Some(dbnum)) = (use_cache, dbnum) {
            if let Some(cache) = GLOBAL_TRANSFORM_CACHE.get() {
                cache.insert_world_transform(dbnum, refno, world);
            }
        }
    }
    Ok(computed)
}

fn transform_from_matrix_cols(cols: &[f64; 16]) -> Option<Transform> {
    // SurrealDB 存储的矩阵为列主序（与 glam::DMat4::from_cols_array 对齐）。
    let m = glam::DMat4::from_cols_array(cols);
    let (scale, rot, trans) = m.to_scale_rotation_translation();
    Some(Transform {
        translation: glam::Vec3::new(trans.x as f32, trans.y as f32, trans.z as f32),
        rotation: glam::Quat::from_xyzw(rot.x as f32, rot.y as f32, rot.z as f32, rot.w as f32),
        scale: glam::Vec3::new(scale.x as f32, scale.y as f32, scale.z as f32),
    })
}

/// 批量版 cache-first world_transform 获取：
/// - 先读 foyer transform_cache；
/// - miss 的 refno 再通过 SurrealDB pe_transform 批量查询 matrix；
/// - 仍 miss 的少量 refno 兜底走旧计算路径（aios_core::get_world_transform）。
///
/// 说明：该接口用于“预取阶段”的批量提速；生成阶段仍应尽量 cache-only。
pub async fn get_world_transforms_cache_first_batch(
    db_option: Option<&DbOptionExt>,
    refnos: &[RefnoEnum],
) -> anyhow::Result<HashMap<RefnoEnum, Transform>> {
    if refnos.is_empty() {
        return Ok(HashMap::new());
    }

    let use_cache = db_option.map(|x| x.use_cache).unwrap_or(true);
    let mut out: HashMap<RefnoEnum, Transform> = HashMap::new();

    // 记录每个 refno 的 dbnum（用于写回缓存；ref0 != dbnum，必须通过 db_meta 映射）
    let mut dbnum_map: HashMap<RefnoEnum, u32> = HashMap::new();
    for &r in refnos {
        if let Some(d) = resolve_dbnum(r) {
            dbnum_map.insert(r, d);
        }
    }

    // 1) cache hits
    let mut misses: Vec<RefnoEnum> = Vec::new();
    if use_cache {
        if let Some(cache) = get_global_cache(db_option).await? {
            // 这里不做过度并发：foyer HybridCache 本身有内存层，单次 get 较轻；且 refnos 通常为 batch_size 级别。
            for &r in refnos {
                let dbnum = *dbnum_map.get(&r).unwrap_or(&0);
                if let Some(hit) = cache.get_world_transform(dbnum, r).await {
                    out.insert(r, hit);
                } else {
                    misses.push(r);
                }
            }
        } else {
            misses.extend_from_slice(refnos);
        }
    } else {
        misses.extend_from_slice(refnos);
    }

    if misses.is_empty() {
        return Ok(out);
    }

    // 2) SurrealDB batch query pe_transform.world_trans.d.matrix
    //    注意：只对 misses 进行查询，减少无谓传输。
    init_surreal().await?;

    #[derive(Debug, Deserialize, SurrealValue)]
    struct PeWorldMatrixRow {
        #[serde(default)]
        refno: Option<RefnoEnum>,
        #[serde(default)]
        matrix: Option<Vec<f64>>,
    }

    // 避免 SQL 过大：按固定块查询
    const CHUNK: usize = 200;
    let mut still_missing: HashSet<RefnoEnum> = misses.iter().copied().collect();

    for chunk in misses.chunks(CHUNK) {
        let ids = chunk
            .iter()
            .map(|r| r.to_pe_key())
            .collect::<Vec<_>>()
            .join(",");

        // 说明：
        // - record::id(id) 取 pe 的 id 部分（如 24381_103385），RefnoEnum 可直接反序列化。
        // - pe_transform 的主键形制：pe_transform:<pe_id>，用 type::record 构造。
        let sql = format!(
            r#"
            SELECT
                record::id(id) as refno,
                (
                    SELECT VALUE world_trans.d.matrix
                    FROM pe_transform
                    WHERE id = type::record('pe_transform', record::id(id))
                    LIMIT 1
                )[0] as matrix
            FROM [{ids}];
            "#
        );

        let rows: Vec<PeWorldMatrixRow> = SUL_DB.query_take(&sql, 0).await?;
        for row in rows {
            let Some(r) = row.refno else { continue };
            let Some(m) = row.matrix else { continue };
            if m.len() != 16 {
                continue;
            }
            let mut cols = [0.0f64; 16];
            cols.copy_from_slice(&m[..16]);
            if let Some(t) = transform_from_matrix_cols(&cols) {
                out.insert(r, t.clone());
                still_missing.remove(&r);
                if use_cache {
                    if let Some(cache) = GLOBAL_TRANSFORM_CACHE.get() {
                        if let Some(&dbnum) = dbnum_map.get(&r) {
                            cache.insert_world_transform(dbnum, r, t);
                        }
                    }
                }
            }
        }
    }

    if still_missing.is_empty() {
        return Ok(out);
    }

    // 3) 兜底：少量 miss 走旧计算路径（并回写缓存）
    //    说明：此分支理论上应该很少触发；若频繁触发，说明 pe_transform 不完整或查询口径需要调整。
    for r in still_missing {
        if let Ok(Some(t)) = aios_core::get_world_transform(r).await {
            out.insert(r, t.clone());
            if use_cache {
                if let Some(cache) = GLOBAL_TRANSFORM_CACHE.get() {
                    if let Some(&dbnum) = dbnum_map.get(&r) {
                        cache.insert_world_transform(dbnum, r, t);
                    }
                }
            }
        }
    }

    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_transform_from_matrix_cols_roundtrip() {
        let scale = glam::DVec3::new(2.0, 3.0, 4.0);
        let rot = glam::DQuat::from_rotation_y(1.2345);
        let trans = glam::DVec3::new(10.0, -20.0, 30.0);
        let m = glam::DMat4::from_scale_rotation_translation(scale, rot, trans);
        let cols = m.to_cols_array();

        let t = transform_from_matrix_cols(&cols).expect("must decompose");
        let eps = 1e-4;
        assert!((t.translation.x - trans.x as f32).abs() < eps);
        assert!((t.translation.y - trans.y as f32).abs() < eps);
        assert!((t.translation.z - trans.z as f32).abs() < eps);
        assert!((t.scale.x - scale.x as f32).abs() < eps);
        assert!((t.scale.y - scale.y as f32).abs() < eps);
        assert!((t.scale.z - scale.z as f32).abs() < eps);
        // rotation 只做近似检查：q 与 -q 等价，这里用 dot 取绝对值。
        let dot = t.rotation.dot(glam::Quat::from_xyzw(
            rot.x as f32,
            rot.y as f32,
            rot.z as f32,
            rot.w as f32,
        ));
        assert!(dot.abs() > 1.0 - 1e-3);
    }
}
