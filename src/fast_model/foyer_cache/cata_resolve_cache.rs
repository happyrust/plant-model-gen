//! [foyer-removal] 桩模块：cata_resolve_cache 已移除，此处仅提供编译兼容。

use std::collections::BTreeMap;
use std::sync::Arc;

use aios_core::RefnoEnum;
use aios_core::Transform;
use aios_core::geometry::GeoBasicType;
use aios_core::parsed_data::CateAxisParam;
use aios_core::parsed_data::geo_params_data::PdmsGeoParam;
use dashmap::DashMap;
use once_cell::sync::OnceCell;

/// 预处理的实例几何信息（缓存用）
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

/// 已解析的元件库组件（缓存用）
#[derive(Clone, Debug)]
pub struct CataResolvedComp {
    pub created_at: i64,
    pub ptset_items: Vec<(i32, CateAxisParam)>,
    pub geos: Vec<PreparedInstGeo>,
    pub has_solid: bool,
}

impl CataResolvedComp {
    /// 将 ptset_items 转换为 BTreeMap
    pub fn ptset_map(&self) -> BTreeMap<i32, CateAxisParam> {
        self.ptset_items.iter().cloned().collect()
    }
}

/// 进程内 cata_resolve 缓存（桩实现）
pub struct CataResolveCache {
    cache: DashMap<String, CataResolvedComp>,
}

impl CataResolveCache {
    pub fn new() -> Self {
        Self {
            cache: DashMap::new(),
        }
    }

    pub fn get(&self, key: &str) -> Option<CataResolvedComp> {
        self.cache.get(key).map(|v| v.clone())
    }

    pub fn insert(&self, key: String, value: &CataResolvedComp) {
        self.cache.insert(key, value.clone());
    }
}

static GLOBAL_CACHE: OnceCell<Arc<CataResolveCache>> = OnceCell::new();

/// 初始化全局 cata_resolve 缓存
pub fn init_global_cata_resolve_cache() {
    let _ = GLOBAL_CACHE.set(Arc::new(CataResolveCache::new()));
}

/// 获取全局 cata_resolve 缓存
pub fn global_cata_resolve_cache() -> Option<Arc<CataResolveCache>> {
    GLOBAL_CACHE.get().cloned()
}
