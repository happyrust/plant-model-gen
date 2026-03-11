use std::collections::{HashMap, HashSet};
use std::sync::{Arc, OnceLock};

use aios_core::{RefnoEnum, Transform};
use dashmap::mapref::entry::Entry;
use dashmap::{DashMap, DashSet};
use futures::StreamExt;
use tokio::sync::Mutex;

use crate::data_interface::db_meta_manager::db_meta;
use crate::options::DbOptionExt;

use super::transform_rkyv_cache::{self, LoadedTransformDbnum};

/// 模型生成阶段的 transform 缓存：
/// - world/local 均放在内存 DashMap 中，按 (dbnum, refno) 索引；
/// - dbnum 首次访问时按需从 rkyv 文件加载；
/// - cache-first 路径 miss 时允许回退到 SurrealDB/旧计算路径。
pub struct TransformCacheManager {
    world_cache: DashMap<(u32, RefnoEnum), Transform>,
    local_cache: DashMap<(u32, RefnoEnum), Transform>,
    pins: DashMap<(u32, RefnoEnum), u32>,
    loaded_dbnums: DashSet<u32>,
}

impl TransformCacheManager {
    pub fn new() -> Self {
        Self {
            world_cache: DashMap::new(),
            local_cache: DashMap::new(),
            pins: DashMap::new(),
            loaded_dbnums: DashSet::new(),
        }
    }

    pub fn get_world_transform(&self, dbnum: u32, refno: RefnoEnum) -> Option<Transform> {
        self.world_cache.get(&(dbnum, refno)).map(|v| v.clone())
    }

    pub fn get_local_transform(&self, dbnum: u32, refno: RefnoEnum) -> Option<Transform> {
        self.local_cache.get(&(dbnum, refno)).map(|v| v.clone())
    }

    pub fn remove(&self, dbnum: u32, refno: RefnoEnum) {
        self.world_cache.remove(&(dbnum, refno));
        self.local_cache.remove(&(dbnum, refno));
        self.loaded_dbnums.remove(&dbnum);
    }

    pub fn insert_world_transform(&self, dbnum: u32, refno: RefnoEnum, world: Transform) {
        self.world_cache.insert((dbnum, refno), world);
    }

    pub fn insert_local_transform(&self, dbnum: u32, refno: RefnoEnum, local: Transform) {
        self.local_cache.insert((dbnum, refno), local);
    }

    pub fn is_dbnum_loaded(&self, dbnum: u32) -> bool {
        self.loaded_dbnums.contains(&dbnum)
    }

    pub fn load_dbnum_snapshot(&self, dbnum: u32, snapshot: LoadedTransformDbnum) {
        for (refno, world) in snapshot.world {
            self.world_cache.insert((dbnum, refno), world);
        }
        for (refno, local) in snapshot.local {
            self.local_cache.insert((dbnum, refno), local);
        }
        self.loaded_dbnums.insert(dbnum);
    }

    /// 清空所有缓存条目，释放内存（分批生成时在批次间调用）。
    pub fn clear(&self) -> usize {
        let count = self.world_cache.len();
        self.world_cache.clear();
        self.local_cache.clear();
        self.pins.clear();
        self.loaded_dbnums.clear();
        count
    }

    /// 按 refno 定向清理 transform 缓存，避免并发任务互相清空全局缓存。
    pub fn clear_refnos(&self, refnos: &[RefnoEnum]) -> usize {
        if refnos.is_empty() {
            return 0;
        }
        let keys = Self::map_refnos_to_keys(refnos);
        let mut count = 0usize;
        let mut stale_dbnums = HashSet::new();
        for (dbnum, refno) in keys {
            let removed_world = self.world_cache.remove(&(dbnum, refno)).is_some();
            let removed_local = self.local_cache.remove(&(dbnum, refno)).is_some();
            if removed_world || removed_local {
                count += 1;
                stale_dbnums.insert(dbnum);
            }
        }
        for dbnum in stale_dbnums {
            self.loaded_dbnums.remove(&dbnum);
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
        let mut stale_dbnums = HashSet::new();

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

            if can_clear {
                let removed_world = self.world_cache.remove(&key).is_some();
                let removed_local = self.local_cache.remove(&key).is_some();
                if removed_world || removed_local {
                    removed += 1;
                    stale_dbnums.insert(key.0);
                }
            }
        }

        for dbnum in stale_dbnums {
            self.loaded_dbnums.remove(&dbnum);
        }

        removed
    }
}

static GLOBAL_TRANSFORM_CACHE: OnceLock<TransformCacheManager> = OnceLock::new();
static DBNUM_LOAD_LOCKS: OnceLock<DashMap<u32, Arc<Mutex<()>>>> = OnceLock::new();

pub fn init_global_transform_cache() {
    let _ = GLOBAL_TRANSFORM_CACHE.get_or_init(TransformCacheManager::new);
}

fn dbnum_load_locks() -> &'static DashMap<u32, Arc<Mutex<()>>> {
    DBNUM_LOAD_LOCKS.get_or_init(DashMap::new)
}

fn dbnum_load_lock(dbnum: u32) -> Arc<Mutex<()>> {
    dbnum_load_locks()
        .entry(dbnum)
        .or_insert_with(|| Arc::new(Mutex::new(())))
        .clone()
}

/// 清空全局 transform 缓存（分批生成时在批次间调用）。
pub fn clear_global_transform_cache() -> usize {
    GLOBAL_TRANSFORM_CACHE
        .get()
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
    log::warn!("[transform_cache] 缺少 ref0->dbnum 映射: refno={}", refno);
    None
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

fn transform_from_dmat4(m: glam::DMat4) -> Option<Transform> {
    transform_from_matrix_cols(&m.to_cols_array())
}

async fn ensure_dbnum_loaded(
    db_option: Option<&DbOptionExt>,
    dbnum: u32,
    allow_build: bool,
) -> anyhow::Result<()> {
    let Some(cache) = get_global_cache() else {
        anyhow::bail!("global transform_cache 未初始化");
    };

    if cache.is_dbnum_loaded(dbnum) {
        return Ok(());
    }

    let lock = dbnum_load_lock(dbnum);
    let _guard = lock.lock().await;

    if cache.is_dbnum_loaded(dbnum) {
        return Ok(());
    }

    let loaded = if allow_build {
        Some(transform_rkyv_cache::load_or_build_dbnum_cache(db_option, dbnum).await?)
    } else {
        transform_rkyv_cache::load_dbnum_cache_if_fresh(db_option, dbnum)?
    };

    let Some(snapshot) = loaded else {
        anyhow::bail!(
            "transform rkyv cache 缺失或已失效: dbnum={} path={}",
            dbnum,
            transform_rkyv_cache::dbnum_cache_path(db_option, dbnum).display()
        );
    };

    cache.load_dbnum_snapshot(dbnum, snapshot);
    Ok(())
}

async fn ensure_dbnums_loaded_for_refnos(
    db_option: Option<&DbOptionExt>,
    refnos: &[RefnoEnum],
    allow_build: bool,
) -> anyhow::Result<()> {
    let mut dbnums = HashSet::new();
    for &refno in refnos {
        if let Some(dbnum) = resolve_dbnum(refno) {
            dbnums.insert(dbnum);
        }
    }

    let mut dbnums: Vec<u32> = dbnums.into_iter().collect();
    dbnums.sort_unstable();
    for dbnum in dbnums {
        ensure_dbnum_loaded(db_option, dbnum, allow_build).await?;
    }
    Ok(())
}

async fn best_effort_preload_dbnums(db_option: Option<&DbOptionExt>, refnos: &[RefnoEnum]) {
    let mut dbnums = HashSet::new();
    for &refno in refnos {
        if let Some(dbnum) = resolve_dbnum(refno) {
            dbnums.insert(dbnum);
        }
    }

    let mut dbnums: Vec<u32> = dbnums.into_iter().collect();
    dbnums.sort_unstable();
    for dbnum in dbnums {
        if let Err(err) = ensure_dbnum_loaded(db_option, dbnum, true).await {
            log::warn!(
                "[transform_cache] 加载/构建 dbnum={} 的 transform rkyv 失败，将回退 DB: {}",
                dbnum,
                err
            );
        }
    }
}

/// 模型生成专用：优先从内存/rkyv transform_cache 读取 world_transform；
/// miss 时按需走旧计算路径并回写内存缓存。
pub async fn get_world_transform_cache_first(
    db_option: Option<&DbOptionExt>,
    refno: RefnoEnum,
) -> anyhow::Result<Option<Transform>> {
    let dbnum = resolve_dbnum(refno);
    let use_cache = dbnum.is_some();

    if let Some(dbnum) = dbnum {
        if let Err(err) = ensure_dbnum_loaded(db_option, dbnum, true).await {
            log::warn!(
                "[transform_cache] 单点加载 dbnum={} 的 world transform 缓存失败: {}",
                dbnum,
                err
            );
        }
        if let Some(cache) = get_global_cache() {
            if let Some(hit) = cache.get_world_transform(dbnum, refno) {
                return Ok(Some(hit));
            }
        }
    }

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

/// 模型生成专用：优先从内存/rkyv transform_cache 读取 local_transform；
/// miss 时按需走旧计算路径并回写内存缓存。
pub async fn get_local_transform_cache_first(
    db_option: Option<&DbOptionExt>,
    refno: RefnoEnum,
) -> anyhow::Result<Option<Transform>> {
    let dbnum = resolve_dbnum(refno);
    let use_cache = dbnum.is_some();

    if let Some(dbnum) = dbnum {
        if let Err(err) = ensure_dbnum_loaded(db_option, dbnum, true).await {
            log::warn!(
                "[transform_cache] 单点加载 dbnum={} 的 local transform 缓存失败: {}",
                dbnum,
                err
            );
        }
        if let Some(cache) = get_global_cache() {
            if let Some(hit) = cache.get_local_transform(dbnum, refno) {
                return Ok(Some(hit));
            }
        }
    }

    let computed = aios_core::transform::get_local_mat4(refno)
        .await?
        .and_then(transform_from_dmat4);
    if let Some(local) = computed.clone() {
        if let (true, Some(dbnum)) = (use_cache, dbnum) {
            if let Some(cache) = GLOBAL_TRANSFORM_CACHE.get() {
                cache.insert_local_transform(dbnum, refno, local);
            }
        }
    }
    Ok(computed)
}

/// strict cache-only：只从内存/rkyv transform_cache 读取 world_transform。
/// miss 直接返回 Err（离线 Generate 阶段 miss 代表预取或 rkyv 不完整）。
pub async fn get_world_transforms_cache_only_batch(
    db_option: &DbOptionExt,
    refnos: &[RefnoEnum],
) -> anyhow::Result<HashMap<RefnoEnum, Transform>> {
    if refnos.is_empty() {
        return Ok(HashMap::new());
    }

    db_meta().ensure_loaded()?;
    init_global_transform_cache();
    ensure_dbnums_loaded_for_refnos(Some(db_option), refnos, false).await?;

    let Some(cache) = GLOBAL_TRANSFORM_CACHE.get() else {
        anyhow::bail!("global transform_cache 未初始化");
    };

    let mut out = HashMap::new();
    let mut missing = Vec::new();

    for &refno in refnos {
        let Some(dbnum) = db_meta().get_dbnum_by_refno(refno) else {
            anyhow::bail!("缺少 ref0->dbnum 映射: refno={}", refno);
        };
        if dbnum == 0 {
            anyhow::bail!(
                "无效 dbnum=0（缺少 ref0->dbnum 映射或元数据不完整）: refno={}",
                refno
            );
        }
        if let Some(hit) = cache.get_world_transform(dbnum, refno) {
            out.insert(refno, hit);
        } else {
            missing.push(refno);
        }
    }

    if !missing.is_empty() {
        missing.sort_by_key(|refno| refno.refno());
        const SAMPLE_LIMIT: usize = 16;
        let sample = missing
            .iter()
            .take(SAMPLE_LIMIT)
            .map(|refno| refno.to_string())
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

/// strict cache-only：只从内存/rkyv transform_cache 读取 local_transform。
pub async fn get_local_transforms_cache_only_batch(
    db_option: &DbOptionExt,
    refnos: &[RefnoEnum],
) -> anyhow::Result<HashMap<RefnoEnum, Transform>> {
    if refnos.is_empty() {
        return Ok(HashMap::new());
    }

    db_meta().ensure_loaded()?;
    init_global_transform_cache();
    ensure_dbnums_loaded_for_refnos(Some(db_option), refnos, false).await?;

    let Some(cache) = GLOBAL_TRANSFORM_CACHE.get() else {
        anyhow::bail!("global transform_cache 未初始化");
    };

    let mut out = HashMap::new();
    let mut missing = Vec::new();

    for &refno in refnos {
        let Some(dbnum) = db_meta().get_dbnum_by_refno(refno) else {
            anyhow::bail!("缺少 ref0->dbnum 映射: refno={}", refno);
        };
        if dbnum == 0 {
            anyhow::bail!(
                "无效 dbnum=0（缺少 ref0->dbnum 映射或元数据不完整）: refno={}",
                refno
            );
        }
        if let Some(hit) = cache.get_local_transform(dbnum, refno) {
            out.insert(refno, hit);
        } else {
            missing.push(refno);
        }
    }

    if !missing.is_empty() {
        missing.sort_by_key(|refno| refno.refno());
        const SAMPLE_LIMIT: usize = 16;
        let sample = missing
            .iter()
            .take(SAMPLE_LIMIT)
            .map(|refno| refno.to_string())
            .collect::<Vec<_>>()
            .join(", ");
        anyhow::bail!(
            "local transform_cache miss（cache-only 不允许回源 DB）：missing={}, sample=[{}]",
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

/// strict cache-only：读取单个 local_transform；miss 直接 Err。
pub async fn get_local_transform_cache_only(
    db_option: &DbOptionExt,
    refno: RefnoEnum,
) -> anyhow::Result<Transform> {
    let hm = get_local_transforms_cache_only_batch(db_option, &[refno]).await?;
    hm.get(&refno)
        .cloned()
        .ok_or_else(|| anyhow::anyhow!("local transform_cache miss: refno={}", refno))
}

/// strict cache-only：确保给定 refnos 的 world transform 均已存在于缓存；缺失直接 Err。
pub async fn ensure_world_transforms_present(
    db_option: &DbOptionExt,
    refnos: &[RefnoEnum],
) -> anyhow::Result<()> {
    let _ = get_world_transforms_cache_only_batch(db_option, refnos).await?;
    Ok(())
}

/// strict cache-only：确保给定 refnos 的 local transform 均已存在于缓存；缺失直接 Err。
pub async fn ensure_local_transforms_present(
    db_option: &DbOptionExt,
    refnos: &[RefnoEnum],
) -> anyhow::Result<()> {
    let _ = get_local_transforms_cache_only_batch(db_option, refnos).await?;
    Ok(())
}

/// 批量版 cache-first world_transform 获取：
/// - 先尝试按 dbnum 从 rkyv 文件加载到内存；
/// - 再读内存 transform_cache；
/// - miss 的 refno 再通过 SurrealDB pe_transform 批量查询；
/// - 仍 miss 的少量 refno 兜底走旧计算路径（aios_core::get_world_transform）。
pub async fn get_world_transforms_cache_first_batch(
    db_option: Option<&DbOptionExt>,
    refnos: &[RefnoEnum],
) -> anyhow::Result<HashMap<RefnoEnum, Transform>> {
    if refnos.is_empty() {
        return Ok(HashMap::new());
    }

    best_effort_preload_dbnums(db_option, refnos).await;

    let mut out = HashMap::new();
    let mut dbnum_map = HashMap::new();
    for &refno in refnos {
        if let Some(dbnum) = resolve_dbnum(refno) {
            dbnum_map.insert(refno, dbnum);
        }
    }

    let mut misses = Vec::new();
    if let Some(cache) = get_global_cache() {
        for &refno in refnos {
            let dbnum = *dbnum_map.get(&refno).unwrap_or(&0);
            if let Some(hit) = cache.get_world_transform(dbnum, refno) {
                out.insert(refno, hit);
            } else {
                misses.push(refno);
            }
        }
    } else {
        misses.extend_from_slice(refnos);
    }

    if misses.is_empty() {
        return Ok(out);
    }

    let queried = transform_rkyv_cache::query_world_transforms_from_pe_transform(&misses).await?;
    let mut still_missing: HashSet<RefnoEnum> = misses.iter().copied().collect();

    for (refno, world) in queried {
        still_missing.remove(&refno);
        out.insert(refno, world.clone());
        if let Some(cache) = GLOBAL_TRANSFORM_CACHE.get() {
            if let Some(&dbnum) = dbnum_map.get(&refno) {
                cache.insert_world_transform(dbnum, refno, world);
            }
        }
    }

    for refno in still_missing {
        if let Ok(Some(world)) = aios_core::get_world_transform(refno).await {
            out.insert(refno, world.clone());
            if let Some(cache) = GLOBAL_TRANSFORM_CACHE.get() {
                if let Some(&dbnum) = dbnum_map.get(&refno) {
                    cache.insert_world_transform(dbnum, refno, world);
                }
            }
        }
    }

    Ok(out)
}

/// 批量版 cache-first local_transform 获取：
/// - 先尝试按 dbnum 从 rkyv 文件加载到内存；
/// - 再读内存 transform_cache；
/// - miss 的 refno 兜底走旧计算路径（get_local_mat4）。
pub async fn get_local_transforms_cache_first_batch(
    db_option: Option<&DbOptionExt>,
    refnos: &[RefnoEnum],
) -> anyhow::Result<HashMap<RefnoEnum, Transform>> {
    if refnos.is_empty() {
        return Ok(HashMap::new());
    }

    best_effort_preload_dbnums(db_option, refnos).await;

    let mut out = HashMap::new();
    let mut dbnum_map = HashMap::new();
    for &refno in refnos {
        if let Some(dbnum) = resolve_dbnum(refno) {
            dbnum_map.insert(refno, dbnum);
        }
    }

    let mut misses = Vec::new();
    if let Some(cache) = get_global_cache() {
        for &refno in refnos {
            let dbnum = *dbnum_map.get(&refno).unwrap_or(&0);
            if let Some(hit) = cache.get_local_transform(dbnum, refno) {
                out.insert(refno, hit);
            } else {
                misses.push(refno);
            }
        }
    } else {
        misses.extend_from_slice(refnos);
    }

    if misses.is_empty() {
        return Ok(out);
    }

    let mut stream = futures::stream::iter(misses.iter().copied().map(|refno| async move {
        let local = aios_core::transform::get_local_mat4(refno)
            .await
            .ok()
            .flatten()
            .and_then(transform_from_dmat4);
        (refno, local)
    }))
    .buffer_unordered(64);

    while let Some((refno, local)) = stream.next().await {
        if let Some(local) = local {
            out.insert(refno, local.clone());
            if let Some(cache) = GLOBAL_TRANSFORM_CACHE.get() {
                if let Some(&dbnum) = dbnum_map.get(&refno) {
                    cache.insert_local_transform(dbnum, refno, local);
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

    #[test]
    fn test_load_dbnum_snapshot_marks_loaded() {
        let mgr = TransformCacheManager::new();
        let refno: RefnoEnum = "24381/36716".into();
        let dbnum = 1112u32;
        let mut world = HashMap::new();
        let mut local = HashMap::new();
        world.insert(refno, Transform::IDENTITY);
        local.insert(refno, Transform::IDENTITY);

        mgr.load_dbnum_snapshot(
            dbnum,
            LoadedTransformDbnum {
                source_version: "v1".into(),
                world,
                local,
            },
        );

        assert!(mgr.is_dbnum_loaded(dbnum));
        assert!(mgr.get_world_transform(dbnum, refno).is_some());
        assert!(mgr.get_local_transform(dbnum, refno).is_some());
    }
}
