use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;

use aios_core::geometry::{EleGeosInfo, EleInstGeosData, ShapeInstancesData};
use aios_core::parsed_data::geo_params_data::PdmsGeoParam;
use aios_core::RefnoEnum;
use foyer::{DirectFsDeviceOptionsBuilder, HybridCache, HybridCacheBuilder};
use serde::{Deserialize, Serialize};
use crate::data_interface::db_meta_manager::db_meta;

#[derive(Clone, Hash, Eq, PartialEq, Serialize, Deserialize)]
pub struct InstanceCacheKey {
    pub dbnum: u32,
    pub batch_id: String,
}

#[derive(Clone, Serialize, Deserialize)]
pub struct InstanceCacheValue {
    pub data: CachedInstanceBatch,
}

#[derive(Clone, Serialize, Deserialize)]
pub struct CachedInstanceBatch {
    pub dbnum: u32,
    pub batch_id: String,
    pub created_at: i64,
    pub inst_info_map: HashMap<RefnoEnum, EleGeosInfo>,
    pub inst_geos_map: HashMap<String, EleInstGeosData>,
    pub inst_tubi_map: HashMap<RefnoEnum, EleGeosInfo>,
    pub neg_relate_map: HashMap<RefnoEnum, Vec<RefnoEnum>>,
    pub ngmr_neg_relate_map: HashMap<RefnoEnum, Vec<(RefnoEnum, RefnoEnum)>>,
}

#[derive(Clone, Serialize, Deserialize)]
pub struct CachedGeoParam {
    pub geo_hash: u64,
    pub geo_param: PdmsGeoParam,
    pub unit_flag: bool,
}

#[derive(Default)]
struct CacheIndex {
    by_dbnum: HashMap<u32, Vec<String>>,
    by_dbnum_set: HashMap<u32, HashSet<String>>,
}

pub struct InstanceCacheManager {
    cache: HybridCache<InstanceCacheKey, InstanceCacheValue>,
    index: Mutex<CacheIndex>,
    counter: AtomicU64,
    cache_dir: PathBuf,
}

impl InstanceCacheManager {
    pub async fn new(cache_dir: &Path) -> anyhow::Result<Self> {
        if !cache_dir.exists() {
            std::fs::create_dir_all(cache_dir)?;
        }

        let device_config = DirectFsDeviceOptionsBuilder::new(cache_dir)
            .with_capacity(1024 * 1024 * 1024)
            .build();

        let cache = HybridCacheBuilder::new()
            .memory(128 * 1024 * 1024)
            .storage()
            .with_device_config(device_config)
            .build()
            .await?;

        Ok(Self {
            cache,
            index: Mutex::new(CacheIndex::default()),
            counter: AtomicU64::new(0),
            cache_dir: cache_dir.to_path_buf(),
        })
    }

    pub fn cache_dir(&self) -> &Path {
        &self.cache_dir
    }

    pub fn insert_batch(&self, batch: CachedInstanceBatch) {
        let key = InstanceCacheKey {
            dbnum: batch.dbnum,
            batch_id: batch.batch_id.clone(),
        };
        let value = InstanceCacheValue { data: batch };
        self.cache.insert(key, value);
    }

    pub fn insert_from_shape(&self, dbnum: u32, shape_insts: &ShapeInstancesData) -> String {
        println!(
            "[cache] insert_from_shape 调用: dbnum={}, inst_cnt={}",
            dbnum,
            shape_insts.inst_cnt()
        );
        let batch_id = self.next_batch_id(dbnum);
        let batch = CachedInstanceBatch {
            dbnum,
            batch_id: batch_id.clone(),
            created_at: chrono::Utc::now().timestamp_millis(),
            inst_info_map: shape_insts.inst_info_map.clone(),
            inst_geos_map: shape_insts.inst_geos_map.clone(),
            inst_tubi_map: shape_insts.inst_tubi_map.clone(),
            neg_relate_map: shape_insts.neg_relate_map.clone(),
            ngmr_neg_relate_map: shape_insts.ngmr_neg_relate_map.clone(),
        };

        self.insert_batch(batch);
        batch_id
    }

    pub async fn get(&self, dbnum: u32, batch_id: &str) -> Option<CachedInstanceBatch> {
        let key = InstanceCacheKey {
            dbnum,
            batch_id: batch_id.to_string(),
        };
        self.cache
            .get(&key)
            .await
            .ok()
            .flatten()
            .map(|entry| entry.value().data.clone())
    }

    pub fn list_batches(&self, dbnum: u32) -> Vec<String> {
        let index = self.index.lock().expect("cache index lock poisoned");
        index
            .by_dbnum
            .get(&dbnum)
            .cloned()
            .unwrap_or_default()
    }

    pub fn list_dbnums(&self) -> Vec<u32> {
        let index = self.index.lock().expect("cache index lock poisoned");
        index.by_dbnum.keys().copied().collect()
    }

    pub fn collect_geo_params(batch: &CachedInstanceBatch) -> Vec<CachedGeoParam> {
        let mut seen = HashSet::new();
        let mut items = Vec::new();

        for geos_data in batch.inst_geos_map.values() {
            for inst in &geos_data.insts {
                if seen.insert(inst.geo_hash) {
                    items.push(CachedGeoParam {
                        geo_hash: inst.geo_hash,
                        geo_param: inst.geo_param.clone(),
                        unit_flag: inst.unit_flag,
                    });
                }
            }
        }

        items
    }

    fn next_batch_id(&self, dbnum: u32) -> String {
        let seq = self.counter.fetch_add(1, Ordering::Relaxed);
        let batch_id = format!("{}_{}", dbnum, seq);

        let mut index = self.index.lock().expect("cache index lock poisoned");
        let set = index.by_dbnum_set.entry(dbnum).or_default();
        if set.insert(batch_id.clone()) {
            index
                .by_dbnum
                .entry(dbnum)
                .or_default()
                .push(batch_id.clone());
        }

        batch_id
    }
}

