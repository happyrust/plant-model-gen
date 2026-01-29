use std::collections::{HashMap, HashSet};
use std::hash::BuildHasherDefault;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;
use std::fs;

use aios_core::geometry::{EleGeosInfo, EleInstGeosData, ShapeInstancesData};
use aios_core::parsed_data::geo_params_data::PdmsGeoParam;
use aios_core::RefnoEnum;
use foyer::{DirectFsDeviceOptionsBuilder, HybridCache, HybridCacheBuilder};
use serde::{Deserialize, Serialize};
use twox_hash::XxHash64;
use crate::data_interface::db_meta_manager::db_meta;
use anyhow::Context;

#[derive(Clone, Hash, Eq, PartialEq, Serialize, Deserialize)]
pub struct InstanceCacheKey {
    pub dbnum: u32,
    pub batch_id: String,
}

#[derive(Clone, Serialize, Deserialize)]
pub struct InstanceCacheValue {
    pub payload: Vec<u8>,
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

#[derive(Default, Serialize, Deserialize, Clone)]
struct CacheIndex {
    by_dbnum: HashMap<u32, Vec<String>>,
    #[serde(default)]
    by_dbnum_set: HashMap<u32, HashSet<String>>,
}

pub struct InstanceCacheManager {
    cache: HybridCache<InstanceCacheKey, InstanceCacheValue, BuildHasherDefault<XxHash64>>,
    index: Mutex<CacheIndex>,
    counter: AtomicU64,
    cache_dir: PathBuf,
}

impl InstanceCacheManager {
    const INDEX_FILE_NAME: &'static str = "instance_cache_index.json";

    pub async fn new(cache_dir: &Path) -> anyhow::Result<Self> {
        if !cache_dir.exists() {
            std::fs::create_dir_all(cache_dir)?;
        }

        let (index, counter_start) = Self::load_index_with_counter(cache_dir);

        let device_config = DirectFsDeviceOptionsBuilder::new(cache_dir)
            .with_capacity(1024 * 1024 * 1024)
            .build();

        let cache = HybridCacheBuilder::new()
            .memory(128 * 1024 * 1024)
            .with_hash_builder(BuildHasherDefault::<XxHash64>::default())
            .storage()
            .with_device_config(device_config)
            .build()
            .await?;

        Ok(Self {
            cache,
            index: Mutex::new(index),
            counter: AtomicU64::new(counter_start),
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
        let payload = match serde_json::to_vec(&batch) {
            Ok(bytes) => bytes,
            Err(e) => {
                eprintln!(
                    "[cache] 序列化失败，跳过写入: dbnum={}, batch_id={}, err={}",
                    batch.dbnum, batch.batch_id, e
                );
                return;
            }
        };
        let value = InstanceCacheValue { payload };
        let dbnum = batch.dbnum;
        let batch_id = batch.batch_id.clone();
        self.cache.insert(key, value);
        if let Err(e) = self.update_index(dbnum, &batch_id) {
            eprintln!(
                "[cache] 写入索引失败: dbnum={}, batch_id={}, err={}",
                dbnum, batch_id, e
            );
        }
    }

    pub fn insert_from_shape(&self, dbnum: u32, shape_insts: &ShapeInstancesData) -> String {
        println!(
            "[cache] insert_from_shape 调用: dbnum={}, inst_cnt={}, inst_info={}, inst_geos={}, inst_tubi={}, neg={}, ngmr={}",
            dbnum,
            shape_insts.inst_cnt(),
            shape_insts.inst_info_map.len(),
            shape_insts.inst_geos_map.len(),
            shape_insts.inst_tubi_map.len(),
            shape_insts.neg_relate_map.len(),
            shape_insts.ngmr_neg_relate_map.len()
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
        match self.cache.get(&key).await {
            Ok(Some(entry)) => {
                let payload = &entry.value().payload;
                match serde_json::from_slice::<CachedInstanceBatch>(payload) {
                    Ok(batch) => Some(batch),
                    Err(e) => {
                        eprintln!(
                            "[cache] 反序列化失败: dbnum={}, batch_id={}, err={}",
                            dbnum, batch_id, e
                        );
                        None
                    }
                }
            }
            Ok(None) => None,
            Err(e) => {
                eprintln!(
                    "[cache] 读取失败: dbnum={}, batch_id={}, err={}",
                    dbnum, batch_id, e
                );
                None
            }
        }
    }

    pub async fn close(&self) -> anyhow::Result<()> {
        self.cache.close().await?;
        Ok(())
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
        format!("{}_{}", dbnum, seq)
    }

    fn index_path(cache_dir: &Path) -> PathBuf {
        cache_dir.join(Self::INDEX_FILE_NAME)
    }

    fn load_index_with_counter(cache_dir: &Path) -> (CacheIndex, u64) {
        let path = Self::index_path(cache_dir);
        let text = fs::read_to_string(&path).ok();
        if let Some(text) = text {
            if let Ok(mut index) = serde_json::from_str::<CacheIndex>(&text) {
                if index.by_dbnum_set.is_empty() && !index.by_dbnum.is_empty() {
                    for (dbnum, batches) in &index.by_dbnum {
                        let set = index.by_dbnum_set.entry(*dbnum).or_default();
                        for batch_id in batches {
                            set.insert(batch_id.clone());
                        }
                    }
                }
                let max_seq = index
                    .by_dbnum
                    .values()
                    .flatten()
                    .filter_map(|id| Self::parse_batch_seq(id))
                    .max()
                    .unwrap_or(0);
                return (index, max_seq + 1);
            }
        }
        (CacheIndex::default(), 0)
    }

    fn parse_batch_seq(batch_id: &str) -> Option<u64> {
        batch_id.rsplit('_').next()?.parse().ok()
    }

    fn update_index(&self, dbnum: u32, batch_id: &str) -> anyhow::Result<()> {
        let mut index = self.index.lock().expect("cache index lock poisoned");
        let set = index.by_dbnum_set.entry(dbnum).or_default();
        if set.insert(batch_id.to_string()) {
            index
                .by_dbnum
                .entry(dbnum)
                .or_default()
                .push(batch_id.to_string());
            self.save_index_locked(&index)?;
        }
        Ok(())
    }

    fn save_index_locked(&self, index: &CacheIndex) -> anyhow::Result<()> {
        let path = Self::index_path(&self.cache_dir);
        let json = serde_json::to_string(index).context("序列化缓存索引失败")?;
        fs::write(&path, json)
            .with_context(|| format!("写入缓存索引失败: {}", path.display()))?;
        Ok(())
    }
}
