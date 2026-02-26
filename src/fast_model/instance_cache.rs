//! [foyer-removal] 桩模块：InstanceCacheManager 已移除，此处仅提供编译兼容。
//! 所有方法均 panic / 返回空值，不应在运行时被调用。

use std::collections::{BTreeMap, HashMap};
use std::path::Path;

use aios_core::RefnoEnum;
use aios_core::geometry::{EleGeosInfo, EleInstGeosData, ShapeInstancesData};
use aios_core::parsed_data::TubiInfoData;
use aios_core::parsed_data::CateAxisParam;

/// 缓存的实例信息（桩）
#[derive(Clone, Debug)]
pub struct CachedInstInfo {
    pub info: EleGeosInfo,
    pub tubi: Option<TubiInfoData>,
    pub inst_key: String,
    pub neg_relates: Vec<RefnoEnum>,
    pub ngmr_neg_relates: Vec<(RefnoEnum, RefnoEnum)>,
}

/// 缓存的实例几何（桩）
#[derive(Clone, Debug)]
pub struct CachedInstGeos {
    pub geos_data: EleInstGeosData,
}

/// 实例缓存管理器（桩）
pub struct InstanceCacheManager {
    _path: std::path::PathBuf,
}

impl InstanceCacheManager {
    pub async fn new(cache_dir: &Path) -> anyhow::Result<Self> {
        Ok(Self { _path: cache_dir.to_path_buf() })
    }

    pub fn list_refnos(&self, _dbnum: u32) -> Vec<RefnoEnum> {
        Vec::new()
    }

    pub fn list_dbnums(&self) -> Vec<u32> {
        Vec::new()
    }

    pub async fn get_inst_info(&self, _dbnum: u32, _refno: RefnoEnum) -> Option<CachedInstInfo> {
        None
    }

    pub async fn get_inst_geos(&self, _dbnum: u32, _inst_key: &str) -> Option<CachedInstGeos> {
        None
    }

    pub fn insert_from_shape(&self, _dbnum: u32, _shape: &ShapeInstancesData) {}

    pub async fn get_ptset_maps_for_refnos_auto(
        &self,
        _refnos: &[RefnoEnum],
    ) -> HashMap<RefnoEnum, BTreeMap<i32, CateAxisParam>> {
        HashMap::new()
    }

    pub async fn close(&self) -> anyhow::Result<()> {
        Ok(())
    }
}
