//! foyer cache 运行时上下文
//!
//! 统一封装 cache-only 路径下的公共依赖：
//! - `cache_dir`：foyer cache 根目录
//! - `InstanceCacheManager`：foyer/instance_cache 的读写入口
//!
//! 设计原则：
//! - 尽量在上层（orchestrator）只初始化一次并复用（Arc clone）。
//! - cache-only 逻辑**不回退** SurrealDB；缺失即返回错误或显式跳过（由上层决定）。

use std::path::{Path, PathBuf};
use std::sync::Arc;

use crate::fast_model::instance_cache::InstanceCacheManager;
use crate::fast_model::foyer_cache::geom_input_cache::GeomInputCacheManager;
use crate::options::DbOptionExt;

/// foyer cache-only 运行时上下文
#[derive(Clone)]
pub struct FoyerCacheContext {
    cache_dir: PathBuf,
    cache: Arc<InstanceCacheManager>,
    geom_input_cache: Option<Arc<GeomInputCacheManager>>,
}

impl FoyerCacheContext {
    /// 从 `DbOptionExt` 构造 cache-only 上下文。
    ///
    /// - 若 `db_option.use_cache=false`，请使用 [`Self::try_from_db_option`]。
    /// - 该函数会尽力预初始化 `transform_cache`（失败仅降级，不阻断）。
    pub async fn from_db_option(db_option: &DbOptionExt) -> anyhow::Result<Self> {
        // 纯内存 transform_cache：模型生成阶段统一走 cache-first 获取 world_transform。
        crate::fast_model::transform_cache::init_global_transform_cache();

        let cache_dir = db_option.get_foyer_cache_dir();
        let cache = Arc::new(InstanceCacheManager::new(&cache_dir).await?);

        // 初始化 geom_input_cache（LOOP/PRIM 输入缓存）
        let geom_input_cache = Some(Arc::new(GeomInputCacheManager::new()));

        Ok(Self { cache_dir, cache, geom_input_cache })
    }

    /// 尝试从 `DbOptionExt` 构造 cache-only 上下文。
    ///
    /// - `db_option.use_cache=true` -> `Ok(Some(ctx))`
    /// - `db_option.use_cache=false` -> `Ok(None)`
    pub async fn try_from_db_option(db_option: &DbOptionExt) -> anyhow::Result<Option<Self>> {
        if !db_option.use_cache {
            return Ok(None);
        }
        Ok(Some(Self::from_db_option(db_option).await?))
    }

    /// 直接从 cache 目录构造上下文（用于工具/测试/调试）。
    pub async fn from_cache_dir(cache_dir: impl AsRef<Path>) -> anyhow::Result<Self> {
        let cache_dir = cache_dir.as_ref().to_path_buf();
        let cache = Arc::new(InstanceCacheManager::new(&cache_dir).await?);
        let geom_input_cache = Some(Arc::new(GeomInputCacheManager::new()));
        Ok(Self { cache_dir, cache, geom_input_cache })
    }

    /// foyer cache 根目录
    pub fn cache_dir(&self) -> &Path {
        &self.cache_dir
    }

    /// 获取 cache 管理器（引用）
    pub fn cache(&self) -> &InstanceCacheManager {
        self.cache.as_ref()
    }

    /// 获取 cache 管理器（Arc clone）
    pub fn cache_arc(&self) -> Arc<InstanceCacheManager> {
        self.cache.clone()
    }

    /// 获取 geom_input_cache 管理器（引用）
    pub fn geom_input_cache(&self) -> Option<&GeomInputCacheManager> {
        self.geom_input_cache.as_deref()
    }

    /// 获取 geom_input_cache 管理器（Arc clone）
    pub fn geom_input_cache_arc(&self) -> Option<Arc<GeomInputCacheManager>> {
        self.geom_input_cache.clone()
    }
}

