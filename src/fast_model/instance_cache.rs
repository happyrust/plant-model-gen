use std::collections::{HashMap, HashSet};
use std::hash::BuildHasherDefault;
use std::path::{Path, PathBuf};
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

use crate::fast_model::foyer_cache::rkyv_payload;

// ---------------------------------------------------------------------------
// Key / Value 类型
// ---------------------------------------------------------------------------

/// refno 级别的缓存 Key
#[derive(Clone, Hash, Eq, PartialEq, Serialize, Deserialize)]
pub struct InstInfoKey {
    pub dbnum: u32,
    pub refno: RefnoEnum,
}

/// inst_key 级别的缓存 Key（多 refno 可共享同一 inst_key）
#[derive(Clone, Hash, Eq, PartialEq, Serialize, Deserialize)]
pub struct InstGeosKey {
    pub dbnum: u32,
    pub inst_key: String,
}

/// 通用 payload 包装
#[derive(Clone, Serialize, Deserialize)]
pub struct CachePayloadValue {
    pub payload: Vec<u8>,
}

/// inst_relate_bool 的缓存条目（cache-only：用于 enable_holes=true 时选择 booled mesh）。
#[derive(Clone, Serialize, Deserialize, Debug, rkyv::Archive, rkyv::Serialize, rkyv::Deserialize)]
pub struct CachedInstRelateBool {
    pub mesh_id: String,
    pub status: String,
    pub created_at: i64,
}

/// 单个 refno 的全部数据聚合
#[derive(Clone, Debug, rkyv::Archive, rkyv::Serialize, rkyv::Deserialize)]
pub struct CachedInstInfo {
    pub info: EleGeosInfo,
    pub tubi: Option<EleGeosInfo>,
    pub inst_key: String,
    pub neg_relates: Vec<RefnoEnum>,
    pub ngmr_neg_relates: Vec<(RefnoEnum, RefnoEnum)>,
    pub relate_bool: Option<CachedInstRelateBool>,
    pub created_at: i64,
}

/// 几何数据（按 inst_key 独立存储）
#[derive(Clone, Debug, rkyv::Archive, rkyv::Serialize, rkyv::Deserialize)]
pub struct CachedInstGeos {
    pub geos_data: EleInstGeosData,
    pub created_at: i64,
}

#[derive(Clone, Serialize, Deserialize)]
pub struct CachedGeoParam {
    pub geo_hash: u64,
    pub geo_param: PdmsGeoParam,
    pub unit_flag: bool,
}

// ---------------------------------------------------------------------------
// rkyv payload 常量（新 schema，旧 type_tag 2001 自动 miss）
// ---------------------------------------------------------------------------

const INST_INFO_TYPE_TAG: u16 = 2010;
const INST_INFO_SCHEMA_V1: u16 = 1;

const INST_GEOS_TYPE_TAG: u16 = 2011;
const INST_GEOS_SCHEMA_V1: u16 = 1;

// ---------------------------------------------------------------------------
// 索引：dbnum -> HashSet<RefnoEnum> + dbnum -> HashSet<String>(inst_key)
// ---------------------------------------------------------------------------

#[derive(Default, Serialize, Deserialize, Clone)]
struct RefnoIndex {
    /// dbnum -> 已缓存的 refno 集合
    by_dbnum: HashMap<u32, HashSet<RefnoEnum>>,
    /// dbnum -> 已缓存的 inst_key 集合
    #[serde(default)]
    geos_by_dbnum: HashMap<u32, HashSet<String>>,
}

// ---------------------------------------------------------------------------
// InstanceCacheManager
// ---------------------------------------------------------------------------

pub struct InstanceCacheManager {
    info_cache: HybridCache<InstInfoKey, CachePayloadValue, BuildHasherDefault<XxHash64>>,
    geos_cache: HybridCache<InstGeosKey, CachePayloadValue, BuildHasherDefault<XxHash64>>,
    index: Mutex<RefnoIndex>,
    cache_dir: PathBuf,
}

impl InstanceCacheManager {
    const INDEX_FILE_NAME: &'static str = "instance_cache_refno_index.json";

    pub async fn new(cache_dir: &Path) -> anyhow::Result<Self> {
        if !cache_dir.exists() {
            std::fs::create_dir_all(cache_dir)?;
        }

        let index = Self::load_index(cache_dir);

        // info_cache：per-refno，条目多但单条小
        let info_dir = cache_dir.join("info");
        if !info_dir.exists() {
            std::fs::create_dir_all(&info_dir)?;
        }
        let info_device = DirectFsDeviceOptionsBuilder::new(&info_dir)
            .with_capacity(512 * 1024 * 1024)
            .build();
        let info_cache = HybridCacheBuilder::new()
            .memory(64 * 1024 * 1024)
            .with_hash_builder(BuildHasherDefault::<XxHash64>::default())
            .storage()
            .with_device_config(info_device)
            .build()
            .await?;

        // geos_cache：per-inst_key，条目较少但单条较大
        let geos_dir = cache_dir.join("geos");
        if !geos_dir.exists() {
            std::fs::create_dir_all(&geos_dir)?;
        }
        let geos_device = DirectFsDeviceOptionsBuilder::new(&geos_dir)
            .with_capacity(512 * 1024 * 1024)
            .build();
        let geos_cache = HybridCacheBuilder::new()
            .memory(64 * 1024 * 1024)
            .with_hash_builder(BuildHasherDefault::<XxHash64>::default())
            .storage()
            .with_device_config(geos_device)
            .build()
            .await?;

        Ok(Self {
            info_cache,
            geos_cache,
            index: Mutex::new(index),
            cache_dir: cache_dir.to_path_buf(),
        })
    }

    pub fn cache_dir(&self) -> &Path {
        &self.cache_dir
    }

    // -----------------------------------------------------------------------
    // 写入 API
    // -----------------------------------------------------------------------

    /// 从 ShapeInstancesData 写入缓存（签名不变，内部拆散为逐 refno 写入）。
    ///
    /// 返回写入的 refno 数量（用于日志）。
    pub fn insert_from_shape(&self, dbnum: u32, shape_insts: &ShapeInstancesData) -> usize {
        let _span = crate::profile_span!(
            "cache_insert_from_shape",
            dbnum = dbnum,
            inst_info_cnt = shape_insts.inst_info_map.len(),
            inst_geos_cnt = shape_insts.inst_geos_map.len()
        );

        let now = chrono::Utc::now().timestamp_millis();
        let mut count = 0usize;

        // 1) 写入 geos_cache（按 inst_key）
        for (inst_key, geos_data) in &shape_insts.inst_geos_map {
            self.insert_inst_geos(dbnum, inst_key.clone(), geos_data, now);
        }

        // 2) 写入 info_cache（按 refno）
        for (refno, info) in &shape_insts.inst_info_map {
            let inst_key = info.get_inst_key();
            let tubi = shape_insts.inst_tubi_map.get(refno).cloned();
            let neg_relates = shape_insts
                .neg_relate_map
                .get(refno)
                .cloned()
                .unwrap_or_default();
            let ngmr_neg_relates = shape_insts
                .ngmr_neg_relate_map
                .get(refno)
                .cloned()
                .unwrap_or_default();

            let cached = CachedInstInfo {
                info: info.clone(),
                tubi,
                inst_key,
                neg_relates,
                ngmr_neg_relates,
                relate_bool: None,
                created_at: now,
            };

            self.insert_inst_info(dbnum, *refno, &cached);
            count += 1;
        }

        count
    }

    /// 写入单个 refno 的 inst_info
    pub fn insert_inst_info(&self, dbnum: u32, refno: RefnoEnum, info: &CachedInstInfo) {
        let key = InstInfoKey { dbnum, refno };
        let payload = match rkyv_payload::encode(INST_INFO_TYPE_TAG, INST_INFO_SCHEMA_V1, info) {
            Ok(bytes) => bytes,
            Err(e) => {
                eprintln!(
                    "[instance_cache] rkyv 序列化 inst_info 失败: dbnum={}, refno={}, err={}",
                    dbnum, refno, e
                );
                return;
            }
        };
        self.info_cache
            .insert(key, CachePayloadValue { payload });
        self.update_refno_index(dbnum, refno);
    }

    /// 写入单个 inst_key 的 geos 数据
    pub fn insert_inst_geos(
        &self,
        dbnum: u32,
        inst_key: String,
        geos_data: &EleInstGeosData,
        created_at: i64,
    ) {
        let key = InstGeosKey {
            dbnum,
            inst_key: inst_key.clone(),
        };
        let cached = CachedInstGeos {
            geos_data: geos_data.clone(),
            created_at,
        };
        let payload = match rkyv_payload::encode(INST_GEOS_TYPE_TAG, INST_GEOS_SCHEMA_V1, &cached)
        {
            Ok(bytes) => bytes,
            Err(e) => {
                eprintln!(
                    "[instance_cache] rkyv 序列化 inst_geos 失败: dbnum={}, inst_key={}, err={}",
                    dbnum, inst_key, e
                );
                return;
            }
        };
        self.geos_cache
            .insert(key, CachePayloadValue { payload });
        self.update_geos_index(dbnum, &inst_key);
    }

    /// 更新布尔运算结果（直接读写单条 refno，无需反序列化整个 batch）。
    pub async fn upsert_inst_relate_bool(
        &self,
        dbnum: u32,
        refno: RefnoEnum,
        mesh_id: String,
        status: &str,
    ) -> anyhow::Result<()> {
        let mut info = self.get_inst_info(dbnum, refno).await.ok_or_else(|| {
            anyhow::anyhow!(
                "instance_cache inst_info 不存在，无法写入 relate_bool: dbnum={} refno={}",
                dbnum,
                refno
            )
        })?;

        info.relate_bool = Some(CachedInstRelateBool {
            mesh_id,
            status: status.to_string(),
            created_at: chrono::Utc::now().timestamp_millis(),
        });

        self.insert_inst_info(dbnum, refno, &info);
        Ok(())
    }

    // -----------------------------------------------------------------------
    // 读取 API
    // -----------------------------------------------------------------------

    /// 读取单个 refno 的 inst_info
    pub async fn get_inst_info(&self, dbnum: u32, refno: RefnoEnum) -> Option<CachedInstInfo> {
        let key = InstInfoKey { dbnum, refno };
        match self.info_cache.get(&key).await {
            Ok(Some(entry)) => {
                let payload = &entry.value().payload;
                match rkyv_payload::decode::<CachedInstInfo>(
                    INST_INFO_TYPE_TAG,
                    INST_INFO_SCHEMA_V1,
                    payload,
                ) {
                    Ok(v) => Some(v),
                    Err(e) => {
                        eprintln!(
                            "[instance_cache] inst_info decode miss: dbnum={}, refno={}, err={}",
                            dbnum, refno, e
                        );
                        None
                    }
                }
            }
            Ok(None) => None,
            Err(e) => {
                eprintln!(
                    "[instance_cache] inst_info 读取失败: dbnum={}, refno={}, err={}",
                    dbnum, refno, e
                );
                None
            }
        }
    }

    /// 读取单个 inst_key 的 geos 数据
    pub async fn get_inst_geos(&self, dbnum: u32, inst_key: &str) -> Option<CachedInstGeos> {
        let key = InstGeosKey {
            dbnum,
            inst_key: inst_key.to_string(),
        };
        match self.geos_cache.get(&key).await {
            Ok(Some(entry)) => {
                let payload = &entry.value().payload;
                match rkyv_payload::decode::<CachedInstGeos>(
                    INST_GEOS_TYPE_TAG,
                    INST_GEOS_SCHEMA_V1,
                    payload,
                ) {
                    Ok(v) => Some(v),
                    Err(e) => {
                        eprintln!(
                            "[instance_cache] inst_geos decode miss: dbnum={}, inst_key={}, err={}",
                            dbnum, inst_key, e
                        );
                        None
                    }
                }
            }
            Ok(None) => None,
            Err(e) => {
                eprintln!(
                    "[instance_cache] inst_geos 读取失败: dbnum={}, inst_key={}, err={}",
                    dbnum, inst_key, e
                );
                None
            }
        }
    }

    /// 列出指定 dbnum 下所有已缓存的 refno
    pub fn list_refnos(&self, dbnum: u32) -> Vec<RefnoEnum> {
        let index = self.index.lock().expect("instance_cache index lock poisoned");
        index
            .by_dbnum
            .get(&dbnum)
            .map(|s| s.iter().copied().collect())
            .unwrap_or_default()
    }

    /// 列出所有已缓存的 dbnum
    pub fn list_dbnums(&self) -> Vec<u32> {
        let index = self.index.lock().expect("instance_cache index lock poisoned");
        index.by_dbnum.keys().copied().collect()
    }

    /// 列出指定 dbnum 下所有已缓存的 inst_key
    pub fn list_inst_keys(&self, dbnum: u32) -> Vec<String> {
        let index = self.index.lock().expect("instance_cache index lock poisoned");
        index
            .geos_by_dbnum
            .get(&dbnum)
            .map(|s| s.iter().cloned().collect())
            .unwrap_or_default()
    }

    // -----------------------------------------------------------------------
    // 批量读取 API（兼容旧消费者）
    // -----------------------------------------------------------------------

    /// 批量获取指定 refno 列表的 ptset_map（ARRIVE/LEAVE 点）
    /// 返回 HashMap<RefnoEnum, [CateAxisParam; 2]>，其中 [0]=ARRIVE, [1]=LEAVE
    pub async fn get_ptset_maps_for_refnos(
        &self,
        dbnum: u32,
        refnos: &[RefnoEnum],
    ) -> HashMap<RefnoEnum, [CateAxisParam; 2]> {
        let mut result = HashMap::new();
        if refnos.is_empty() {
            return result;
        }

        // arrive/leave 点编号来自元件属性 ARRI/LEAV
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

        for &refno in refnos {
            let Some(info) = self.get_inst_info(dbnum, refno).await else {
                continue;
            };
            let (arrive_no, leave_no) = al_numbers
                .get(&refno.refno().0)
                .copied()
                .unwrap_or((1, 2));
            let arrive = info
                .info
                .ptset_map
                .values()
                .find(|p| p.number == arrive_no)
                .cloned();
            let leave = info
                .info
                .ptset_map
                .values()
                .find(|p| p.number == leave_no)
                .cloned();
            if let (Some(arrive), Some(leave)) = (arrive, leave) {
                result.insert(refno, [arrive, leave]);
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
    pub async fn get_ptset_for_refno(
        &self,
        dbnum: u32,
        refno: RefnoEnum,
    ) -> Option<[CateAxisParam; 2]> {
        let info = self.get_inst_info(dbnum, refno).await?;
        let arrive = info.info.ptset_map.get(&1)?;
        let leave = info.info.ptset_map.get(&2)?;
        Some([arrive.clone(), leave.clone()])
    }

    /// 收集指定 dbnum 下所有 geo_params（用于 mesh 生成）
    pub async fn collect_all_geo_params(&self, dbnum: u32) -> Vec<CachedGeoParam> {
        let mut seen = HashSet::new();
        let mut items = Vec::new();

        for inst_key in self.list_inst_keys(dbnum) {
            let Some(cached) = self.get_inst_geos(dbnum, &inst_key).await else {
                continue;
            };
            for inst in &cached.geos_data.insts {
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

    // -----------------------------------------------------------------------
    // 删除 API
    // -----------------------------------------------------------------------

    /// 删除指定 dbnum 下的所有缓存数据
    pub fn remove_dbnum(&self, dbnum: u32) -> usize {
        let refnos = self.list_refnos(dbnum);
        let inst_keys = self.list_inst_keys(dbnum);
        let count = refnos.len();

        for refno in &refnos {
            let key = InstInfoKey {
                dbnum,
                refno: *refno,
            };
            self.info_cache.remove(&key);
        }
        for inst_key in &inst_keys {
            let key = InstGeosKey {
                dbnum,
                inst_key: inst_key.clone(),
            };
            self.geos_cache.remove(&key);
        }

        if let Ok(mut index) = self.index.lock() {
            index.by_dbnum.remove(&dbnum);
            index.geos_by_dbnum.remove(&dbnum);
            let _ = self.save_index_locked(&index);
        }

        count
    }

    pub async fn close(&self) -> anyhow::Result<()> {
        self.info_cache.close().await?;
        self.geos_cache.close().await?;
        Ok(())
    }

    // -----------------------------------------------------------------------
    // 索引管理
    // -----------------------------------------------------------------------

    fn index_path(cache_dir: &Path) -> PathBuf {
        cache_dir.join(Self::INDEX_FILE_NAME)
    }

    fn load_index(cache_dir: &Path) -> RefnoIndex {
        let path = Self::index_path(cache_dir);
        if let Ok(text) = fs::read_to_string(&path) {
            if let Ok(index) = serde_json::from_str::<RefnoIndex>(&text) {
                return index;
            }
        }
        RefnoIndex::default()
    }

    fn update_refno_index(&self, dbnum: u32, refno: RefnoEnum) {
        if let Ok(mut index) = self.index.lock() {
            if index.by_dbnum.entry(dbnum).or_default().insert(refno) {
                let _ = self.save_index_locked(&index);
            }
        }
    }

    fn update_geos_index(&self, dbnum: u32, inst_key: &str) {
        if let Ok(mut index) = self.index.lock() {
            if index
                .geos_by_dbnum
                .entry(dbnum)
                .or_default()
                .insert(inst_key.to_string())
            {
                let _ = self.save_index_locked(&index);
            }
        }
    }

    fn save_index_locked(&self, index: &RefnoIndex) -> anyhow::Result<()> {
        let path = Self::index_path(&self.cache_dir);
        let json = serde_json::to_string(index)?;
        fs::write(&path, json)?;
        Ok(())
    }

}
