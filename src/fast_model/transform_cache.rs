use std::hash::BuildHasherDefault;
use std::path::{Path, PathBuf};

use aios_core::RefnoEnum;
use bevy_transform::prelude::Transform;
use foyer::{DirectFsDeviceOptionsBuilder, HybridCache, HybridCacheBuilder};
use serde::{Deserialize, Serialize};
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

fn resolve_dbnum(refno: RefnoEnum) -> u32 {
    if db_meta().ensure_loaded().is_ok() {
        if let Some(dbnum) = db_meta().get_dbnum_by_refno(refno) {
            return dbnum;
        }
    }
    // 兜底：沿用旧逻辑（ref0）。
    refno.refno().get_0()
}

/// 模型生成专用：从 foyer transform cache 读取 world_transform；miss 时按需生成并回写缓存。
///
/// 与旧逻辑区分：旧逻辑直接调用 `aios_core::get_world_transform`（会优先查 pe_transform 表）。
pub async fn get_world_transform_cache_first(
    db_option: Option<&DbOptionExt>,
    refno: RefnoEnum,
) -> anyhow::Result<Option<Transform>> {
    let dbnum = resolve_dbnum(refno);

    if let Some(cache) = get_global_cache(db_option).await? {
        if let Some(hit) = cache.get_world_transform(dbnum, refno).await {
            return Ok(Some(hit));
        }
    }

    // miss：先用“直接读 pe.world_trans”的轻量路径（不依赖 pe_transform 预热）。
    if let Ok(Some(world)) = aios_core::rs_surreal::query_pe_world_trans(refno).await {
        if let Some(cache) = GLOBAL_TRANSFORM_CACHE.get() {
            cache.insert_world_transform(dbnum, refno, world.clone());
        }
        return Ok(Some(world));
    }

    // 再兜底走旧计算路径（策略/惰性计算）。
    let computed = aios_core::get_world_transform(refno).await?;
    if let Some(world) = computed.clone() {
        if let Some(cache) = GLOBAL_TRANSFORM_CACHE.get() {
            cache.insert_world_transform(dbnum, refno, world);
        }
    }
    Ok(computed)
}
