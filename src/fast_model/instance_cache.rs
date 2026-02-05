use std::collections::{HashMap, HashSet};
use std::hash::BuildHasherDefault;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;
use std::fs;

use aios_core::geometry::{EleGeosInfo, EleInstGeosData, ShapeInstancesData};
use aios_core::parsed_data::geo_params_data::PdmsGeoParam;
use aios_core::parsed_data::CateAxisParam;
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

/// inst_relate_bool 的缓存条目（cache-only：用于 enable_holes=true 时选择 booled mesh）。
#[derive(Clone, Serialize, Deserialize, Debug)]
pub struct CachedInstRelateBool {
    pub mesh_id: String,
    pub status: String,
    pub created_at: i64,
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
    /// refno -> bool 结果（serde default 以兼容旧缓存文件）。
    #[serde(default)]
    pub inst_relate_bool_map: HashMap<RefnoEnum, CachedInstRelateBool>,
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
            inst_relate_bool_map: HashMap::new(),
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

    /// 回写 cache-only 布尔结果（以 batch 为最小回写单元）。
    pub async fn upsert_inst_relate_bool(
        &self,
        dbnum: u32,
        batch_id: &str,
        refno: RefnoEnum,
        mesh_id: String,
        status: &str,
    ) -> anyhow::Result<()> {
        let Some(mut batch) = self.get(dbnum, batch_id).await else {
            anyhow::bail!(
                "instance_cache batch 不存在，无法写入 inst_relate_bool: dbnum={} batch_id={} refno={}",
                dbnum,
                batch_id,
                refno
            );
        };

        batch.inst_relate_bool_map.insert(
            refno,
            CachedInstRelateBool {
                mesh_id,
                status: status.to_string(),
                created_at: chrono::Utc::now().timestamp_millis(),
            },
        );

        self.insert_batch(batch);
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

    /// 删除指定 dbnum 下的所有 batch 数据
    pub fn remove_dbnum(&self, dbnum: u32) -> usize {
        let batch_ids = self.list_batches(dbnum);
        let count = batch_ids.len();

        for batch_id in &batch_ids {
            let key = InstanceCacheKey {
                dbnum,
                batch_id: batch_id.clone(),
            };
            self.cache.remove(&key);
        }

        // 从索引中移除整个 dbnum
        if let Err(e) = self.remove_dbnum_from_index(dbnum) {
            eprintln!("[cache] 更新索引失败: dbnum={}, err={}", dbnum, e);
        }

        count
    }

    fn remove_dbnum_from_index(&self, dbnum: u32) -> anyhow::Result<()> {
        let mut index = self.index.lock().expect("cache index lock poisoned");
        index.by_dbnum.remove(&dbnum);
        index.by_dbnum_set.remove(&dbnum);
        self.save_index_locked(&index)
    }

    /// 批量获取指定 refno 列表的 ptset_map（ARRIVE/LEAVE 点）
    /// 返回 HashMap<RefnoEnum, [CateAxisParam; 2]>，其中 [0]=ARRIVE(ptset[1]), [1]=LEAVE(ptset[2])
    pub async fn get_ptset_maps_for_refnos(
        &self,
        dbnum: u32,
        refnos: &[RefnoEnum],
    ) -> HashMap<RefnoEnum, [CateAxisParam; 2]> {
        let mut result = HashMap::new();
        if refnos.is_empty() {
            return result;
        }

        // arrive/leave 点编号来自元件属性 ARRI/LEAV；不同元件并非固定为 1/2。
        // cache-only：若无法读取属性（例如未初始化 SurrealDB），则回退到旧假设 (1,2)。
        let mut al_numbers: HashMap<u64, (i32, i32)> = HashMap::new();
        for &r in refnos {
            let mut arrive = 1i32;
            let mut leave = 2i32;
            if let Ok(att) = aios_core::get_named_attmap(r).await {
                let a = att.get_i32("ARRI").unwrap_or(0);
                let l = att.get_i32("LEAV").unwrap_or(0);
                if a > 0 && l > 0 {
                    arrive = a;
                    leave = l;
                }
            }
            al_numbers.insert(r.refno().0, (arrive, leave));
        }

        let want_set: HashSet<u64> = refnos.iter().map(|r| r.refno().0).collect();
        let batch_ids = self.list_batches(dbnum);

        // 倒序遍历，优先取最新 batch
        for batch_id in batch_ids.iter().rev() {
            let Some(batch) = self.get(dbnum, batch_id).await else {
                continue;
            };

            for (k, info) in batch.inst_info_map.iter() {
                let refno_u64 = k.refno().0;
                if !want_set.contains(&refno_u64) {
                    continue;
                }
                if result.contains_key(k) {
                    continue; // 已找到，跳过
                }

                let (arrive_no, leave_no) = al_numbers.get(&refno_u64).copied().unwrap_or((1, 2));
                let arrive = info.ptset_map.values().find(|p| p.number == arrive_no).cloned();
                let leave = info.ptset_map.values().find(|p| p.number == leave_no).cloned();
                if let (Some(arrive), Some(leave)) = (arrive, leave) {
                    result.insert(*k, [arrive, leave]);
                }
            }

            // 如果已找到所有，提前退出
            if result.len() >= refnos.len() {
                break;
            }
        }

        result
    }

    pub async fn get_ptset_maps_for_refnos_auto(
        &self,
        refnos: &[RefnoEnum],
    ) -> HashMap<RefnoEnum, [CateAxisParam; 2]> {
        let mut result = HashMap::new();
        if refnos.is_empty() {
            return result;
        }

        if let Err(e) = db_meta().ensure_loaded() {
            log::warn!(
                "[cache] db_meta 未加载，无法自动分组 dbnum 读取 ptset_map，将返回空结果: {}",
                e
            );
            return result;
        }

        let mut groups: HashMap<u32, Vec<RefnoEnum>> = HashMap::new();
        for &refno in refnos {
            let Some(dbnum) = db_meta().get_dbnum_by_refno(refno) else {
                continue;
            };
            if dbnum == 0 {
                continue;
            }
            groups.entry(dbnum).or_default().push(refno);
        }

        for (dbnum, group) in groups {
            let hm = self.get_ptset_maps_for_refnos(dbnum, &group).await;
            result.extend(hm);
        }

        result
    }

    /// 获取单个 refno 的 ptset_map（ARRIVE/LEAVE 点）
    /// 返回 Option<[CateAxisParam; 2]>，其中 [0]=ARRIVE(ptset[1]), [1]=LEAVE(ptset[2])
    pub async fn get_ptset_for_refno(
        &self,
        dbnum: u32,
        refno: RefnoEnum,
    ) -> Option<[CateAxisParam; 2]> {
        let batch_ids = self.list_batches(dbnum);
        let want_u64 = refno.refno().0;

        // 倒序遍历，优先取最新 batch
        for batch_id in batch_ids.iter().rev() {
            let Some(batch) = self.get(dbnum, batch_id).await else {
                continue;
            };

            for (k, info) in batch.inst_info_map.iter() {
                let k_u64 = k.refno().0;
                if k_u64 != want_u64 {
                    continue;
                }

                // ptset_map: [1]=ARRIVE, [2]=LEAVE
                if let (Some(arrive), Some(leave)) = (info.ptset_map.get(&1), info.ptset_map.get(&2)) {
                    return Some([arrive.clone(), leave.clone()]);
                }
            }
        }

        None
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
                        unit_flag: inst.geo_param.is_reuse_unit(),
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
