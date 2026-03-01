use std::collections::{HashMap, HashSet};

use aios_core::{RefnoEnum, SurrealQueryExt, model_primary_db};
use aios_core::Transform;
use dashmap::DashMap;
use dashmap::mapref::entry::Entry;
use serde::Deserialize;
use surrealdb::types::SurrealValue;
use std::sync::OnceLock;

use crate::data_interface::db_meta_manager::db_meta;
use crate::options::DbOptionExt;

/// 纯内存 transform 缓存：用于"模型生成阶段"读取/写入 world_transform。
///
/// 约定：
/// - 只要缓存存在即可（不要求全量命中）。
/// - miss 时，允许走旧路径按需计算/查询，然后回写到内存缓存。

pub struct TransformCacheManager {
    cache: DashMap<(u32, RefnoEnum), Transform>,
    pins: DashMap<(u32, RefnoEnum), u32>,
}

impl TransformCacheManager {
    pub fn new() -> Self {
        Self {
            cache: DashMap::new(),
            pins: DashMap::new(),
        }
    }

    pub fn get_world_transform(&self, dbnum: u32, refno: RefnoEnum) -> Option<Transform> {
        self.cache.get(&(dbnum, refno)).map(|v| v.clone())
    }

    pub fn remove(&self, dbnum: u32, refno: RefnoEnum) {
        self.cache.remove(&(dbnum, refno));
    }

    pub fn insert_world_transform(&self, dbnum: u32, refno: RefnoEnum, world: Transform) {
        self.cache.insert((dbnum, refno), world);
    }

    /// 清空所有缓存条目，释放内存（分批生成时在批次间调用）。
    pub fn clear(&self) -> usize {
        let count = self.cache.len();
        self.cache.clear();
        count
    }

    /// 按 refno 定向清理 world_transform 缓存，避免并发任务互相清空全局缓存。
    pub fn clear_refnos(&self, refnos: &[RefnoEnum]) -> usize {
        if refnos.is_empty() {
            return 0;
        }
        let keys = Self::map_refnos_to_keys(refnos);
        let mut count = 0usize;
        for (dbnum, refno) in keys {
            if self.cache.remove(&(dbnum, refno)).is_some() {
                count += 1;
            }
        }
        count
    }

    /// 为一组 refno 增加缓存租约（pin），用于并发任务隔离。
    pub fn pin_refnos(&self, refnos: &[RefnoEnum]) -> usize {
        if refnos.is_empty() {
            return 0;
        }
        let keys = Self::map_refnos_to_keys(refnos);
        self.pin_keys(&keys)
    }

    /// 释放一组 refno 的缓存租约（不清理缓存条目）。
    pub fn unpin_refnos(&self, refnos: &[RefnoEnum]) -> usize {
        if refnos.is_empty() {
            return 0;
        }
        let keys = Self::map_refnos_to_keys(refnos);
        self.unpin_keys(&keys)
    }

    /// 释放一组 refno 的缓存租约，并在无人持有时清理缓存条目。
    pub fn release_refnos_and_clear(&self, refnos: &[RefnoEnum]) -> usize {
        if refnos.is_empty() {
            return 0;
        }
        let keys = Self::map_refnos_to_keys(refnos);
        self.release_keys_and_clear(&keys)
    }

    fn map_refnos_to_keys(refnos: &[RefnoEnum]) -> Vec<(u32, RefnoEnum)> {
        let _ = db_meta().ensure_loaded();

        let mut keys = Vec::with_capacity(refnos.len());
        for &refno in refnos {
            let Some(dbnum) = db_meta().get_dbnum_by_refno(refno) else {
                continue;
            };
            keys.push((dbnum, refno));
        }
        keys
    }

    fn pin_keys(&self, keys: &[(u32, RefnoEnum)]) -> usize {
        let mut pinned = 0usize;
        for &key in keys {
            match self.pins.entry(key) {
                Entry::Occupied(mut occ) => {
                    *occ.get_mut() += 1;
                }
                Entry::Vacant(vac) => {
                    vac.insert(1);
                }
            }
            pinned += 1;
        }
        pinned
    }

    fn unpin_keys(&self, keys: &[(u32, RefnoEnum)]) -> usize {
        let mut released = 0usize;
        for &key in keys {
            match self.pins.entry(key) {
                Entry::Occupied(mut occ) => {
                    let cnt = occ.get_mut();
                    if *cnt > 1 {
                        *cnt -= 1;
                    } else {
                        occ.remove();
                    }
                    released += 1;
                }
                Entry::Vacant(_) => {}
            }
        }
        released
    }

    fn release_keys_and_clear(&self, keys: &[(u32, RefnoEnum)]) -> usize {
        let mut removed = 0usize;
        for &key in keys {
            let mut can_clear = true;

            match self.pins.entry(key) {
                Entry::Occupied(mut occ) => {
                    let cnt = occ.get_mut();
                    if *cnt > 1 {
                        *cnt -= 1;
                        can_clear = false;
                    } else {
                        occ.remove();
                    }
                }
                Entry::Vacant(_) => {
                    // 未被 pin 的历史调用，维持兼容：允许直接清理。
                }
            }

            if can_clear && self.cache.remove(&key).is_some() {
                removed += 1;
            }
        }
        removed
    }
}

static GLOBAL_TRANSFORM_CACHE: OnceLock<TransformCacheManager> = OnceLock::new();

pub fn init_global_transform_cache() {
    let _ = GLOBAL_TRANSFORM_CACHE.get_or_init(|| TransformCacheManager::new());
}

/// 清空全局 transform 缓存（分批生成时在批次间调用）。
pub fn clear_global_transform_cache() -> usize {
    GLOBAL_TRANSFORM_CACHE.get()
        .map(|mgr| mgr.clear())
        .unwrap_or(0)
}

/// 按 refno 清理全局 transform 缓存（用于分批任务隔离）。
pub fn clear_global_transform_cache_for_refnos(refnos: &[RefnoEnum]) -> usize {
    GLOBAL_TRANSFORM_CACHE
        .get()
        .map(|mgr| mgr.release_refnos_and_clear(refnos))
        .unwrap_or(0)
}

/// 为一组 refno 增加全局 transform 缓存租约（pin）。
pub fn pin_global_transform_cache_for_refnos(refnos: &[RefnoEnum]) -> usize {
    GLOBAL_TRANSFORM_CACHE
        .get()
        .map(|mgr| mgr.pin_refnos(refnos))
        .unwrap_or(0)
}

/// 释放一组 refno 的全局 transform 缓存租约（不清理条目）。
pub fn release_global_transform_cache_for_refnos(refnos: &[RefnoEnum]) -> usize {
    GLOBAL_TRANSFORM_CACHE
        .get()
        .map(|mgr| mgr.unpin_refnos(refnos))
        .unwrap_or(0)
}

fn get_global_cache() -> Option<&'static TransformCacheManager> {
    // 若尚未初始化则自动初始化（纯内存，无副作用）
    if GLOBAL_TRANSFORM_CACHE.get().is_none() {
        init_global_transform_cache();
    }
    GLOBAL_TRANSFORM_CACHE.get()
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

/// 模型生成专用：从内存 transform cache 读取 world_transform；miss 时按需生成并回写缓存。
pub async fn get_world_transform_cache_first(
    db_option: Option<&DbOptionExt>,
    refno: RefnoEnum,
) -> anyhow::Result<Option<Transform>> {
    let dbnum = resolve_dbnum(refno);
    let use_cache = db_option.map(|x| x.use_cache).unwrap_or(true) && dbnum.is_some();

    if use_cache {
        let dbnum = dbnum.unwrap();
        if let Some(cache) = get_global_cache() {
            if let Some(hit) = cache.get_world_transform(dbnum, refno) {
                return Ok(Some(hit));
            }
        }
    }

    // miss：走旧计算路径（策略/惰性计算）。
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

/// strict cache-only：只从内存 transform_cache 读取 world_transform。
/// miss 直接返回 Err（离线 Generate 阶段 miss 代表 Prefetch 不完整）。
pub async fn get_world_transforms_cache_only_batch(
    _db_option: &DbOptionExt,
    refnos: &[RefnoEnum],
) -> anyhow::Result<HashMap<RefnoEnum, Transform>> {
    if refnos.is_empty() {
        return Ok(HashMap::new());
    }

    db_meta().ensure_loaded()?;
    init_global_transform_cache();

    let Some(cache) = GLOBAL_TRANSFORM_CACHE.get() else {
        anyhow::bail!("global transform_cache 未初始化");
    };

    let mut out: HashMap<RefnoEnum, Transform> = HashMap::new();
    let mut missing: Vec<RefnoEnum> = Vec::new();

    for &r in refnos {
        let Some(dbnum) = db_meta().get_dbnum_by_refno(r) else {
            anyhow::bail!("缺少 ref0->dbnum 映射: refno={}", r);
        };
        if dbnum == 0 {
            anyhow::bail!("无效 dbnum=0（缺少 ref0->dbnum 映射或元数据不完整）: refno={}", r);
        }
        if let Some(hit) = cache.get_world_transform(dbnum, r) {
            out.insert(r, hit);
        } else {
            missing.push(r);
        }
    }

    if !missing.is_empty() {
        missing.sort_by_key(|r| r.refno());
        const SAMPLE_LIMIT: usize = 16;
        let sample = missing
            .iter()
            .take(SAMPLE_LIMIT)
            .map(|r| r.to_string())
            .collect::<Vec<_>>()
            .join(", ");
        anyhow::bail!(
            "transform_cache miss（cache-only 不允许回源 DB）：missing={}, sample=[{}]",
            missing.len(),
            sample
        );
    }

    Ok(out)
}

/// strict cache-only：读取单个 world_transform；miss 直接 Err。
pub async fn get_world_transform_cache_only(
    db_option: &DbOptionExt,
    refno: RefnoEnum,
) -> anyhow::Result<Transform> {
    let hm = get_world_transforms_cache_only_batch(db_option, &[refno]).await?;
    hm.get(&refno)
        .cloned()
        .ok_or_else(|| anyhow::anyhow!("transform_cache miss: refno={}", refno))
}

/// strict cache-only：确保给定 refnos 的 transform 均已存在于缓存；缺失直接 Err。
pub async fn ensure_world_transforms_present(
    db_option: &DbOptionExt,
    refnos: &[RefnoEnum],
) -> anyhow::Result<()> {
    let _ = get_world_transforms_cache_only_batch(db_option, refnos).await?;
    Ok(())
}

fn transform_from_matrix_cols(cols: &[f64; 16]) -> Option<Transform> {
    let m = glam::DMat4::from_cols_array(cols);
    let (scale, rot, trans) = m.to_scale_rotation_translation();
    Some(Transform {
        translation: glam::Vec3::new(trans.x as f32, trans.y as f32, trans.z as f32),
        rotation: glam::Quat::from_xyzw(rot.x as f32, rot.y as f32, rot.z as f32, rot.w as f32),
        scale: glam::Vec3::new(scale.x as f32, scale.y as f32, scale.z as f32),
    })
}

/// 批量版 cache-first world_transform 获取：
/// - 先读内存 transform_cache；
/// - miss 的 refno 再通过 SurrealDB pe_transform 批量查询 matrix；
/// - 仍 miss 的少量 refno 兜底走旧计算路径（aios_core::get_world_transform）。
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
        if let Some(cache) = get_global_cache() {
            for &r in refnos {
                let dbnum = *dbnum_map.get(&r).unwrap_or(&0);
                if let Some(hit) = cache.get_world_transform(dbnum, r) {
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
    crate::fast_model::utils::ensure_surreal_init().await?;

    #[derive(Debug, Deserialize, SurrealValue)]
    struct PeWorldMatrixRow {
        #[serde(default)]
        refno: Option<RefnoEnum>,
        #[serde(default)]
        matrix: Option<Vec<f64>>,
    }

    const CHUNK: usize = 200;
    let mut still_missing: HashSet<RefnoEnum> = misses.iter().copied().collect();

    for chunk in misses.chunks(CHUNK) {
        let ids = chunk
            .iter()
            .map(|r| r.to_pe_key())
            .collect::<Vec<_>>()
            .join(",");

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

        let rows: Vec<PeWorldMatrixRow> = model_primary_db().query_take(&sql, 0).await?;
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
        let dot = t.rotation.dot(glam::Quat::from_xyzw(
            rot.x as f32,
            rot.y as f32,
            rot.z as f32,
            rot.w as f32,
        ));
        assert!(dot.abs() > 1.0 - 1e-3);
    }

    #[test]
    fn test_transform_cache_pin_release_two_tasks_same_refno() {
        let mgr = TransformCacheManager::new();
        let refno: RefnoEnum = "24381/36716".into();
        let dbnum = 1112u32;

        mgr.insert_world_transform(dbnum, refno.clone(), Transform::IDENTITY);
        let keys = vec![(dbnum, refno.clone())];

        assert_eq!(mgr.pin_keys(&keys), 1);
        assert_eq!(mgr.pin_keys(&keys), 1);

        let removed_first = mgr.release_keys_and_clear(&keys);
        assert_eq!(removed_first, 0, "仍有并发租约时不应清理缓存");
        assert!(mgr.get_world_transform(dbnum, refno.clone()).is_some());

        let removed_second = mgr.release_keys_and_clear(&keys);
        assert_eq!(removed_second, 1, "最后一个租约释放后应清理对应缓存条目");
        assert!(mgr.get_world_transform(dbnum, refno).is_none());
    }
}
