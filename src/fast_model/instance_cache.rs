use std::collections::{HashMap, HashSet};
use std::hash::BuildHasherDefault;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;
use std::fs;

use aios_core::geometry::{EleGeosInfo, EleInstGeo, EleInstGeosData, GeoBasicType, ShapeInstancesData};
use aios_core::parsed_data::geo_params_data::PdmsGeoParam;
use aios_core::parsed_data::CateAxisParam;
use aios_core::RefnoEnum;
use aios_core::Transform;
use glam::Vec3;
use foyer::{DirectFsDeviceOptionsBuilder, HybridCache, HybridCacheBuilder};
use serde::{Deserialize, Serialize};
use twox_hash::XxHash64;
use crate::data_interface::db_meta_manager::db_meta;
use anyhow::Context;

use crate::fast_model::foyer_cache::rkyv_payload;

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
#[derive(Clone, Serialize, Deserialize, Debug, rkyv::Archive, rkyv::Serialize, rkyv::Deserialize)]
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

// ---------------------------------------------------------------------------
// rkyv payload（V1 schema）
// ---------------------------------------------------------------------------

const INSTANCE_CACHE_TYPE_TAG: u16 = 2001;
const INSTANCE_CACHE_SCHEMA_V1: u16 = 1;

#[derive(Clone, Copy, Debug, Default, rkyv::Archive, rkyv::Serialize, rkyv::Deserialize)]
struct TransformV1 {
    t: [f32; 3],
    r: [f32; 4], // x,y,z,w
    s: [f32; 3],
}

impl From<Transform> for TransformV1 {
    fn from(v: Transform) -> Self {
        Self {
            t: [v.translation.x, v.translation.y, v.translation.z],
            r: [v.rotation.x, v.rotation.y, v.rotation.z, v.rotation.w],
            s: [v.scale.x, v.scale.y, v.scale.z],
        }
    }
}

impl From<TransformV1> for Transform {
    fn from(v: TransformV1) -> Self {
        Transform {
            translation: Vec3::new(v.t[0], v.t[1], v.t[2]),
            rotation: glam::Quat::from_xyzw(v.r[0], v.r[1], v.r[2], v.r[3]),
            scale: Vec3::new(v.s[0], v.s[1], v.s[2]),
        }
    }
}

#[derive(Clone, Debug, rkyv::Archive, rkyv::Serialize, rkyv::Deserialize)]
struct EleGeosInfoV1 {
    refno: RefnoEnum,
    sesno: i32,
    owner_refno: RefnoEnum,
    owner_type: String,
    cata_hash: Option<String>,
    visible: bool,
    generic_type: aios_core::pdms_types::PdmsGenericType,
    world_transform: TransformV1,
    ptset_items: Vec<(i32, CateAxisParam)>,
    is_solid: bool,
}

#[derive(Clone, Debug, rkyv::Archive, rkyv::Serialize, rkyv::Deserialize)]
struct EleInstGeoV1 {
    geo_hash: u64,
    refno: RefnoEnum,
    pts: Vec<i32>,
    geo_transform: TransformV1,
    geo_param: PdmsGeoParam,
    visible: bool,
    is_tubi: bool,
    geo_type: GeoBasicType,
    cata_neg_refnos: Vec<RefnoEnum>,
}

#[derive(Clone, Debug, rkyv::Archive, rkyv::Serialize, rkyv::Deserialize)]
struct EleInstGeosDataV1 {
    inst_key: String,
    refno: RefnoEnum,
    insts: Vec<EleInstGeoV1>,
    type_name: String,
}

#[derive(Clone, Debug, rkyv::Archive, rkyv::Serialize, rkyv::Deserialize)]
struct CachedInstanceBatchV1 {
    dbnum: u32,
    batch_id: String,
    created_at: i64,
    inst_info_items: Vec<(RefnoEnum, EleGeosInfoV1)>,
    inst_geos_items: Vec<(String, EleInstGeosDataV1)>,
    inst_tubi_items: Vec<(RefnoEnum, EleGeosInfoV1)>,
    neg_relate_items: Vec<(RefnoEnum, Vec<RefnoEnum>)>,
    ngmr_neg_relate_items: Vec<(RefnoEnum, Vec<(RefnoEnum, RefnoEnum)>)>,
    inst_relate_bool_items: Vec<(RefnoEnum, CachedInstRelateBool)>,
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
        let _span = crate::profile_span!(
            "cache_insert_batch",
            dbnum = batch.dbnum,
            inst_info_cnt = batch.inst_info_map.len(),
            inst_geos_cnt = batch.inst_geos_map.len()
        );
        let key = InstanceCacheKey {
            dbnum: batch.dbnum,
            batch_id: batch.batch_id.clone(),
        };

        // 迁移策略（方案1）：payload 统一写 rkyv；旧 JSON cache 读到即 miss，由上游重建。
        let ser_start = std::time::Instant::now();
        let v1 = CachedInstanceBatchV1 {
            dbnum: batch.dbnum,
            batch_id: batch.batch_id.clone(),
            created_at: batch.created_at,
            inst_info_items: batch
                .inst_info_map
                .into_iter()
                .map(|(k, v)| {
                    let ptset_items = v.ptset_map.into_iter().collect::<Vec<_>>();
                    (
                        k,
                        EleGeosInfoV1 {
                            refno: v.refno,
                            sesno: v.sesno,
                            owner_refno: v.owner_refno,
                            owner_type: v.owner_type,
                            cata_hash: v.cata_hash,
                            visible: v.visible,
                            generic_type: v.generic_type,
                            world_transform: v.world_transform.into(),
                            ptset_items,
                            is_solid: v.is_solid,
                        },
                    )
                })
                .collect(),
            inst_geos_items: batch
                .inst_geos_map
                .into_iter()
                .map(|(k, v)| {
                    let insts = v
                        .insts
                        .into_iter()
                        .map(|g| EleInstGeoV1 {
                            geo_hash: g.geo_hash,
                            refno: g.refno,
                            pts: g.pts,
                            geo_transform: g.geo_transform.into(),
                            geo_param: g.geo_param,
                            visible: g.visible,
                            is_tubi: g.is_tubi,
                            geo_type: g.geo_type,
                            cata_neg_refnos: g.cata_neg_refnos,
                        })
                        .collect();
                    (
                        k,
                        EleInstGeosDataV1 {
                            inst_key: v.inst_key,
                            refno: v.refno,
                            insts,
                            type_name: v.type_name,
                        },
                    )
                })
                .collect(),
            inst_tubi_items: batch
                .inst_tubi_map
                .into_iter()
                .map(|(k, v)| {
                    let ptset_items = v.ptset_map.into_iter().collect::<Vec<_>>();
                    (
                        k,
                        EleGeosInfoV1 {
                            refno: v.refno,
                            sesno: v.sesno,
                            owner_refno: v.owner_refno,
                            owner_type: v.owner_type,
                            cata_hash: v.cata_hash,
                            visible: v.visible,
                            generic_type: v.generic_type,
                            world_transform: v.world_transform.into(),
                            ptset_items,
                            is_solid: v.is_solid,
                        },
                    )
                })
                .collect(),
            neg_relate_items: batch.neg_relate_map.into_iter().collect(),
            ngmr_neg_relate_items: batch.ngmr_neg_relate_map.into_iter().collect(),
            inst_relate_bool_items: batch.inst_relate_bool_map.into_iter().collect(),
        };

        let payload = match rkyv_payload::encode(INSTANCE_CACHE_TYPE_TAG, INSTANCE_CACHE_SCHEMA_V1, &v1) {
            Ok(bytes) => bytes,
            Err(e) => {
                eprintln!(
                    "[cache] rkyv 序列化失败，跳过写入: dbnum={}, batch_id={}, err={}",
                    key.dbnum, key.batch_id, e
                );
                return;
            }
        };
        let ser_ms = ser_start.elapsed().as_millis();
        #[cfg(feature = "profile")]
        tracing::info!(payload_bytes = payload.len(), serialize_ms = ser_ms as u64, "cache_insert_batch rkyv serialized");
        let value = InstanceCacheValue { payload };
        let dbnum = key.dbnum;
        let batch_id = key.batch_id.clone();
        self.cache.insert(key, value);
        if let Err(e) = self.update_index(dbnum, &batch_id) {
            eprintln!(
                "[cache] 写入索引失败: dbnum={}, batch_id={}, err={}",
                dbnum, batch_id, e
            );
        }
    }

    pub fn insert_from_shape(&self, dbnum: u32, shape_insts: &ShapeInstancesData) -> String {
        // 高频路径：默认不打印到 stdout，避免 profile 场景下 IO 成为瓶颈。
        // 需要时可设置环境变量 `AIOS_CACHE_INSERT_STDOUT=1|true` 打开。
        let stdout_enabled = std::env::var("AIOS_CACHE_INSERT_STDOUT")
            .ok()
            .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
            .unwrap_or(false);
        if stdout_enabled {
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
        }
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

    #[cfg_attr(feature = "profile", tracing::instrument(skip_all, name = "cache_get_batch"))]
    pub async fn get(&self, dbnum: u32, batch_id: &str) -> Option<CachedInstanceBatch> {
        let key = InstanceCacheKey {
            dbnum,
            batch_id: batch_id.to_string(),
        };
        match self.cache.get(&key).await {
            Ok(Some(entry)) => {
                let payload = &entry.value().payload;
                let deser_start = std::time::Instant::now();
                let v1 = match rkyv_payload::decode::<CachedInstanceBatchV1>(
                    INSTANCE_CACHE_TYPE_TAG,
                    INSTANCE_CACHE_SCHEMA_V1,
                    payload,
                ) {
                    Ok(v) => v,
                    Err(e) => {
                        // 迁移策略（方案1）：旧 JSON payload / schema 不匹配 一律视为 miss。
                        eprintln!(
                            "[cache] payload decode miss: dbnum={}, batch_id={}, err={}",
                            dbnum, batch_id, e
                        );
                        return None;
                    }
                };

                let mut inst_info_map: HashMap<RefnoEnum, EleGeosInfo> = HashMap::new();
                for (k, info) in v1.inst_info_items {
                    let mut ptset_map = std::collections::BTreeMap::new();
                    for (n, p) in info.ptset_items {
                        ptset_map.insert(n, p);
                    }
                    inst_info_map.insert(
                        k,
                        EleGeosInfo {
                            refno: info.refno,
                            sesno: info.sesno,
                            owner_refno: info.owner_refno,
                            owner_type: info.owner_type,
                            cata_hash: info.cata_hash,
                            visible: info.visible,
                            generic_type: info.generic_type,
                            world_transform: info.world_transform.into(),
                            ptset_map,
                            is_solid: info.is_solid,
                            ..Default::default()
                        },
                    );
                }

                let mut inst_tubi_map: HashMap<RefnoEnum, EleGeosInfo> = HashMap::new();
                for (k, info) in v1.inst_tubi_items {
                    let mut ptset_map = std::collections::BTreeMap::new();
                    for (n, p) in info.ptset_items {
                        ptset_map.insert(n, p);
                    }
                    inst_tubi_map.insert(
                        k,
                        EleGeosInfo {
                            refno: info.refno,
                            sesno: info.sesno,
                            owner_refno: info.owner_refno,
                            owner_type: info.owner_type,
                            cata_hash: info.cata_hash,
                            visible: info.visible,
                            generic_type: info.generic_type,
                            world_transform: info.world_transform.into(),
                            ptset_map,
                            is_solid: info.is_solid,
                            ..Default::default()
                        },
                    );
                }

                let mut inst_geos_map: HashMap<String, EleInstGeosData> = HashMap::new();
                for (k, gd) in v1.inst_geos_items {
                    let insts = gd
                        .insts
                        .into_iter()
                        .map(|g| EleInstGeo {
                            geo_hash: g.geo_hash,
                            refno: g.refno,
                            pts: g.pts,
                            aabb: None,
                            geo_transform: g.geo_transform.into(),
                            geo_param: g.geo_param,
                            visible: g.visible,
                            is_tubi: g.is_tubi,
                            geo_type: g.geo_type,
                            cata_neg_refnos: g.cata_neg_refnos,
                        })
                        .collect();

                    inst_geos_map.insert(
                        k,
                        EleInstGeosData {
                            inst_key: gd.inst_key,
                            refno: gd.refno,
                            insts,
                            aabb: None,
                            type_name: gd.type_name,
                            ..Default::default()
                        },
                    );
                }

                #[cfg(feature = "profile")]
                tracing::debug!(
                    payload_bytes = payload.len(),
                    deserialize_ms = deser_start.elapsed().as_millis() as u64,
                    "cache_get_batch rkyv deserialized"
                );

                Some(CachedInstanceBatch {
                    dbnum: v1.dbnum,
                    batch_id: v1.batch_id,
                    created_at: v1.created_at,
                    inst_info_map,
                    inst_geos_map,
                    inst_tubi_map,
                    neg_relate_map: v1.neg_relate_items.into_iter().collect(),
                    ngmr_neg_relate_map: v1.ngmr_neg_relate_items.into_iter().collect(),
                    inst_relate_bool_map: v1.inst_relate_bool_items.into_iter().collect(),
                })
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
