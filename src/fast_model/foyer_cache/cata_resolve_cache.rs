//! CATA `resolve_desi_comp` 产物缓存（按 cata_hash）
//!
//! 目标：将 CATE 元件库的"可复用几何准备结果"缓存到内存，避免后续运行重复调用
//! `resolve_desi_comp -> try_convert -> unit 参数/scale 归一` 这一整段链路。
//!
//! 注意：
//! - 缓存粒度：`cata_hash`（同组 design_refno 共享）。
//! - 纯内存缓存，进程退出后丢失。

use aios_core::geometry::GeoBasicType;
use aios_core::parsed_data::geo_params_data::PdmsGeoParam;
use aios_core::parsed_data::CateAxisParam;
use aios_core::RefnoEnum;
use aios_core::Transform;
use dashmap::DashMap;
use std::sync::OnceLock;

// ---------------------------------------------------------------------------
// 对外数据结构
// ---------------------------------------------------------------------------

#[derive(Clone, Debug)]
pub struct PreparedInstGeo {
    pub geo_hash: u64,
    pub geom_refno: RefnoEnum,
    pub pts: Vec<i32>,
    pub geo_transform: Transform,
    pub geo_param: PdmsGeoParam,
    pub shape_visible: bool,
    pub is_tubi: bool,
    pub geo_type: GeoBasicType,
    pub unit_flag: bool,
}

#[derive(Clone, Debug)]
pub struct CataResolvedComp {
    pub created_at: i64,
    pub ptset_items: Vec<(i32, CateAxisParam)>,
    pub geos: Vec<PreparedInstGeo>,
    pub has_solid: bool,
}

impl CataResolvedComp {
    #[inline]
    pub fn ptset_map(&self) -> std::collections::BTreeMap<i32, CateAxisParam> {
        self.ptset_items.iter().cloned().collect()
    }
}

// ---------------------------------------------------------------------------
// Cache Manager（纯内存 DashMap）
// ---------------------------------------------------------------------------

pub struct CataResolveCacheManager {
    cache: DashMap<String, CataResolvedComp>,
}

impl CataResolveCacheManager {
    pub fn new() -> Self {
        Self {
            cache: DashMap::new(),
        }
    }

    pub fn insert(&self, cata_hash: String, value: &CataResolvedComp) {
        self.cache.insert(cata_hash, value.clone());
    }

    pub fn get(&self, cata_hash: &str) -> Option<CataResolvedComp> {
        self.cache.get(cata_hash).map(|v| v.clone())
    }
}

// ---------------------------------------------------------------------------
// 全局缓存管理
// ---------------------------------------------------------------------------

static GLOBAL_CATA_RESOLVE_CACHE: OnceLock<CataResolveCacheManager> = OnceLock::new();

/// 初始化全局 cata_resolve_cache（幂等，仅首次生效）。
pub fn init_global_cata_resolve_cache() {
    let _ = GLOBAL_CATA_RESOLVE_CACHE.get_or_init(|| CataResolveCacheManager::new());
}

/// 获取全局 cata_resolve_cache 引用（未初始化返回 None）。
pub fn global_cata_resolve_cache() -> Option<&'static CataResolveCacheManager> {
    GLOBAL_CATA_RESOLVE_CACHE.get()
}
