//! LOOP/PRIM 输入缓存
//!
//! 通过预取 LOOP/PRIM 所需的几何与属性数据到 foyer cache，
//! 在实例生成阶段只读缓存，降低对 SurrealDB 的依赖。
//!
//! 数据结构：
//! - `LoopInput`：LOOP 类元素的预取输入（attmap、world_transform、loops、height 等）
//! - `PrimInput`：PRIM 类元素的预取输入（attmap、world_transform 等）
//! - `GeomInputBatch`：按 dbnum 分批存储的输入缓存条目
//!
//! 使用流程：
//! 1. 预取阶段：`prefetch_loop_inputs` / `prefetch_prim_inputs` 从 SurrealDB 批量拉取并写入缓存
//! 2. 生成阶段：`gen_loop_geos_from_cache` / `gen_prim_geos_from_cache` 仅从缓存读取

use std::collections::HashMap;
use std::hash::BuildHasherDefault;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;

use aios_core::types::NamedAttrMap;
use aios_core::RefnoEnum;
use aios_core::Transform;
use foyer::{DirectFsDeviceOptionsBuilder, HybridCache, HybridCacheBuilder};
use glam::Vec3;
use serde::{Deserialize, Serialize};
use tokio::sync::OnceCell;
use twox_hash::XxHash64;

use super::rkyv_payload;

// ---------------------------------------------------------------------------
// 数据结构
// ---------------------------------------------------------------------------

/// LOOP 类元素的预取输入
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct LoopInput {
    pub refno: RefnoEnum,
    pub attmap: NamedAttrMap,
    pub world_transform: Transform,
    /// 所有 loop 的顶点数据（来自 fetch_loops_and_height）
    pub loops: Vec<Vec<Vec3>>,
    /// 高度值
    pub height: f32,
    pub owner_refno: RefnoEnum,
    pub owner_type: String,
    pub visible: bool,
    /// 负实体 refno 列表
    pub neg_refnos: Vec<RefnoEnum>,
    /// CMPF 下的负实体 refno 列表
    pub cmpf_neg_refnos: Vec<RefnoEnum>,
}

/// PRIM 类元素的预取输入
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PrimPolyPolygon {
    pub loops: Vec<Vec<Vec3>>,
}

/// PRIM：多面体（POHE/POLYHE）所需的额外输入。
///
/// 说明：
/// - 该结构用于让“模型生成阶段”在 cache-only 条件下也能构建 Polyhedron，
///   避免在生成阶段再去查询 POIN/POLOOP/LOOPTS 等深层节点属性。
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PrimPolyExtra {
    pub polygons: Vec<PrimPolyPolygon>,
    pub is_polyhe: bool,
}

/// PRIM 类元素的预取输入
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PrimInput {
    pub refno: RefnoEnum,
    pub attmap: NamedAttrMap,
    pub world_transform: Transform,
    pub owner_refno: RefnoEnum,
    pub owner_type: String,
    pub visible: bool,
    /// 负实体 refno 列表
    pub neg_refnos: Vec<RefnoEnum>,
    /// 多面体额外输入（仅 POHE/POLYHE 需要）
    pub poly_extra: Option<PrimPolyExtra>,
}

/// 按 dbnum 分批存储的输入缓存条目
#[derive(Clone, Serialize, Deserialize)]
pub struct GeomInputBatch {
    pub dbnum: u32,
    pub batch_id: String,
    pub created_at: i64,
    pub loop_inputs: HashMap<RefnoEnum, LoopInput>,
    pub prim_inputs: HashMap<RefnoEnum, PrimInput>,
}

// ---------------------------------------------------------------------------
// rkyv payload（V1 schema）
// ---------------------------------------------------------------------------

const GEOM_INPUT_TYPE_TAG: u16 = 1001;
const GEOM_INPUT_SCHEMA_V2: u16 = 2;

#[derive(Clone, Copy, Debug, Default, rkyv::Archive, rkyv::Serialize, rkyv::Deserialize)]
struct Vec3V1 {
    x: f32,
    y: f32,
    z: f32,
}

impl From<Vec3> for Vec3V1 {
    fn from(v: Vec3) -> Self {
        Self { x: v.x, y: v.y, z: v.z }
    }
}

impl From<Vec3V1> for Vec3 {
    fn from(v: Vec3V1) -> Self {
        Vec3::new(v.x, v.y, v.z)
    }
}

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
struct LoopInputV1 {
    refno: RefnoEnum,
    attmap: NamedAttrMap,
    world_transform: TransformV1,
    loops: Vec<Vec<Vec3V1>>,
    height: f32,
    owner_refno: RefnoEnum,
    owner_type: String,
    visible: bool,
    neg_refnos: Vec<RefnoEnum>,
    cmpf_neg_refnos: Vec<RefnoEnum>,
}

#[derive(Clone, Debug, rkyv::Archive, rkyv::Serialize, rkyv::Deserialize)]
struct PrimPolyPolygonV1 {
    loops: Vec<Vec<Vec3V1>>,
}

#[derive(Clone, Debug, rkyv::Archive, rkyv::Serialize, rkyv::Deserialize)]
struct PrimPolyExtraV1 {
    polygons: Vec<PrimPolyPolygonV1>,
    is_polyhe: bool,
}

#[derive(Clone, Debug, rkyv::Archive, rkyv::Serialize, rkyv::Deserialize)]
struct PrimInputV1 {
    refno: RefnoEnum,
    attmap: NamedAttrMap,
    world_transform: TransformV1,
    owner_refno: RefnoEnum,
    owner_type: String,
    visible: bool,
    neg_refnos: Vec<RefnoEnum>,
    poly_extra: Option<PrimPolyExtraV1>,
}

#[derive(Clone, Debug, rkyv::Archive, rkyv::Serialize, rkyv::Deserialize)]
struct GeomInputBatchV1 {
    dbnum: u32,
    batch_id: String,
    created_at: i64,
    loop_inputs: Vec<LoopInputV1>,
    prim_inputs: Vec<PrimInputV1>,
}

// ---------------------------------------------------------------------------
// 缓存 Key / Value
// ---------------------------------------------------------------------------

#[derive(Clone, Hash, Eq, PartialEq, Serialize, Deserialize)]
pub struct GeomInputCacheKey {
    pub dbnum: u32,
    pub batch_id: String,
}

#[derive(Clone, Serialize, Deserialize)]
pub struct GeomInputCacheValue {
    pub payload: Vec<u8>,
}

// ---------------------------------------------------------------------------
// 索引
// ---------------------------------------------------------------------------

#[derive(Default, Serialize, Deserialize, Clone)]
struct CacheIndex {
    by_dbnum: HashMap<u32, Vec<String>>,
}

// ---------------------------------------------------------------------------
// GeomInputCacheManager
// ---------------------------------------------------------------------------

pub struct GeomInputCacheManager {
    cache: HybridCache<GeomInputCacheKey, GeomInputCacheValue, BuildHasherDefault<XxHash64>>,
    index: Mutex<CacheIndex>,
    counter: AtomicU64,
    cache_dir: PathBuf,
}

impl GeomInputCacheManager {
    const INDEX_FILE_NAME: &'static str = "geom_input_cache_index.json";

    pub async fn new(cache_dir: &Path) -> anyhow::Result<Self> {
        if !cache_dir.exists() {
            std::fs::create_dir_all(cache_dir)?;
        }

        let (index, counter_start) = Self::load_index_with_counter(cache_dir);

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
            index: Mutex::new(index),
            counter: AtomicU64::new(counter_start),
            cache_dir: cache_dir.to_path_buf(),
        })
    }

    /// 写入一个 batch
    pub fn insert_batch(&self, batch: GeomInputBatch) {
        let key = GeomInputCacheKey {
            dbnum: batch.dbnum,
            batch_id: batch.batch_id.clone(),
        };
        let v1 = GeomInputBatchV1 {
            dbnum: batch.dbnum,
            batch_id: batch.batch_id.clone(),
            created_at: batch.created_at,
            loop_inputs: batch
                .loop_inputs
                .into_iter()
                .map(|(refno, v)| LoopInputV1 {
                    refno,
                    attmap: v.attmap,
                    world_transform: v.world_transform.into(),
                    loops: v
                        .loops
                        .into_iter()
                        .map(|lp| lp.into_iter().map(Vec3V1::from).collect())
                        .collect(),
                    height: v.height,
                    owner_refno: v.owner_refno,
                    owner_type: v.owner_type,
                    visible: v.visible,
                    neg_refnos: v.neg_refnos,
                    cmpf_neg_refnos: v.cmpf_neg_refnos,
                })
                .collect(),
            prim_inputs: batch
                .prim_inputs
                .into_iter()
                .map(|(refno, v)| PrimInputV1 {
                    refno,
                    attmap: v.attmap,
                    world_transform: v.world_transform.into(),
                    owner_refno: v.owner_refno,
                    owner_type: v.owner_type,
                    visible: v.visible,
                    neg_refnos: v.neg_refnos,
                    poly_extra: v.poly_extra.map(|pe| PrimPolyExtraV1 {
                        polygons: pe
                            .polygons
                            .into_iter()
                            .map(|p| PrimPolyPolygonV1 {
                                loops: p
                                    .loops
                                    .into_iter()
                                    .map(|lp| lp.into_iter().map(Vec3V1::from).collect())
                                    .collect(),
                            })
                            .collect(),
                        is_polyhe: pe.is_polyhe,
                    }),
                })
                .collect(),
        };

        let payload = match rkyv_payload::encode(GEOM_INPUT_TYPE_TAG, GEOM_INPUT_SCHEMA_V2, &v1) {
            Ok(bytes) => bytes,
            Err(e) => {
                eprintln!(
                    "[geom_input_cache] rkyv 序列化失败: dbnum={}, batch_id={}, err={}",
                    batch.dbnum, batch.batch_id, e
                );
                return;
            }
        };
        let dbnum = batch.dbnum;
        let batch_id = batch.batch_id.clone();
        self.cache
            .insert(key, GeomInputCacheValue { payload });
        if let Err(e) = self.update_index(dbnum, &batch_id) {
            eprintln!(
                "[geom_input_cache] 写入索引失败: dbnum={}, batch_id={}, err={}",
                dbnum, batch_id, e
            );
        }
    }

    /// 读取一个 batch
    pub async fn get(&self, dbnum: u32, batch_id: &str) -> Option<GeomInputBatch> {
        let key = GeomInputCacheKey {
            dbnum,
            batch_id: batch_id.to_string(),
        };
        match self.cache.get(&key).await {
            Ok(Some(entry)) => {
                let payload = &entry.value().payload;
                let v1 = match rkyv_payload::decode::<GeomInputBatchV1>(
                    GEOM_INPUT_TYPE_TAG,
                    GEOM_INPUT_SCHEMA_V2,
                    payload,
                ) {
                    Ok(v) => v,
                    Err(e) => {
                        // 迁移策略（方案1）：旧 JSON payload / schema 不匹配 一律视为 miss。
                        eprintln!(
                            "[geom_input_cache] payload decode miss: dbnum={}, batch_id={}, err={}",
                            dbnum, batch_id, e
                        );
                        return None;
                    }
                };

                let mut loop_inputs = HashMap::new();
                for v in v1.loop_inputs {
                    let refno: RefnoEnum = v.refno;
                    loop_inputs.insert(
                        refno,
                        LoopInput {
                            refno,
                            attmap: v.attmap,
                            world_transform: v.world_transform.into(),
                            loops: v
                                .loops
                                .into_iter()
                                .map(|lp| lp.into_iter().map(Vec3::from).collect())
                                .collect(),
                            height: v.height,
                            owner_refno: v.owner_refno,
                            owner_type: v.owner_type,
                            visible: v.visible,
                            neg_refnos: v.neg_refnos,
                            cmpf_neg_refnos: v.cmpf_neg_refnos,
                        },
                    );
                }

                let mut prim_inputs = HashMap::new();
                for v in v1.prim_inputs {
                    let refno: RefnoEnum = v.refno;
                    prim_inputs.insert(
                        refno,
                        PrimInput {
                            refno,
                            attmap: v.attmap,
                            world_transform: v.world_transform.into(),
                            owner_refno: v.owner_refno,
                            owner_type: v.owner_type,
                            visible: v.visible,
                            neg_refnos: v.neg_refnos,
                            poly_extra: v.poly_extra.map(|pe| PrimPolyExtra {
                                polygons: pe
                                    .polygons
                                    .into_iter()
                                    .map(|p| PrimPolyPolygon {
                                        loops: p
                                            .loops
                                            .into_iter()
                                            .map(|lp| lp.into_iter().map(Vec3::from).collect())
                                            .collect(),
                                    })
                                    .collect(),
                                is_polyhe: pe.is_polyhe,
                            }),
                        },
                    );
                }

                Some(GeomInputBatch {
                    dbnum: v1.dbnum,
                    batch_id: v1.batch_id,
                    created_at: v1.created_at,
                    loop_inputs,
                    prim_inputs,
                })
            }
            Ok(None) => None,
            Err(e) => {
                eprintln!(
                    "[geom_input_cache] 读取失败: dbnum={}, batch_id={}, err={}",
                    dbnum, batch_id, e
                );
                None
            }
        }
    }

    /// 列出指定 dbnum 下的所有 batch_id
    pub fn list_batches(&self, dbnum: u32) -> Vec<String> {
        let index = self.index.lock().expect("geom_input_cache index lock poisoned");
        index
            .by_dbnum
            .get(&dbnum)
            .cloned()
            .unwrap_or_default()
    }

    /// 列出所有已缓存的 dbnum
    pub fn list_dbnums(&self) -> Vec<u32> {
        let index = self.index.lock().expect("geom_input_cache index lock poisoned");
        index.by_dbnum.keys().copied().collect()
    }

    /// 读取指定 dbnum 下所有 batch 的 loop inputs，合并为一个 HashMap
    pub async fn get_all_loop_inputs(&self, dbnum: u32) -> HashMap<RefnoEnum, LoopInput> {
        let mut result = HashMap::new();
        for batch_id in self.list_batches(dbnum) {
            if let Some(batch) = self.get(dbnum, &batch_id).await {
                result.extend(batch.loop_inputs);
            }
        }
        result
    }

    /// 读取指定 dbnum 下所有 batch 的 prim inputs，合并为一个 HashMap
    pub async fn get_all_prim_inputs(&self, dbnum: u32) -> HashMap<RefnoEnum, PrimInput> {
        let mut result = HashMap::new();
        for batch_id in self.list_batches(dbnum) {
            if let Some(batch) = self.get(dbnum, &batch_id).await {
                result.extend(batch.prim_inputs);
            }
        }
        result
    }

    /// 删除指定 dbnum 下的所有 batch 数据
    pub fn remove_dbnum(&self, dbnum: u32) -> usize {
        let batch_ids = self.list_batches(dbnum);
        let count = batch_ids.len();
        for batch_id in &batch_ids {
            let key = GeomInputCacheKey {
                dbnum,
                batch_id: batch_id.clone(),
            };
            self.cache.remove(&key);
        }
        if let Ok(mut index) = self.index.lock() {
            index.by_dbnum.remove(&dbnum);
            let _ = self.save_index_locked(&index);
        }
        count
    }

    pub async fn close(&self) -> anyhow::Result<()> {
        self.cache.close().await?;
        Ok(())
    }

    fn next_batch_id(&self, dbnum: u32) -> String {
        let seq = self.counter.fetch_add(1, Ordering::Relaxed);
        format!("gi_{}_{}", dbnum, seq)
    }

    fn index_path(cache_dir: &Path) -> PathBuf {
        cache_dir.join(Self::INDEX_FILE_NAME)
    }

    fn load_index_with_counter(cache_dir: &Path) -> (CacheIndex, u64) {
        let path = Self::index_path(cache_dir);
        if let Ok(text) = std::fs::read_to_string(&path) {
            if let Ok(index) = serde_json::from_str::<CacheIndex>(&text) {
                let max_seq = index
                    .by_dbnum
                    .values()
                    .flatten()
                    .filter_map(|id| id.rsplit('_').next()?.parse::<u64>().ok())
                    .max()
                    .unwrap_or(0);
                return (index, max_seq + 1);
            }
        }
        (CacheIndex::default(), 0)
    }

    fn update_index(&self, dbnum: u32, batch_id: &str) -> anyhow::Result<()> {
        let mut index = self.index.lock().expect("geom_input_cache index lock poisoned");
        let list = index.by_dbnum.entry(dbnum).or_default();
        if !list.contains(&batch_id.to_string()) {
            list.push(batch_id.to_string());
            self.save_index_locked(&index)?;
        }
        Ok(())
    }

    fn save_index_locked(&self, index: &CacheIndex) -> anyhow::Result<()> {
        let path = Self::index_path(&self.cache_dir);
        let json = serde_json::to_string(index)?;
        std::fs::write(&path, json)?;
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// 预取：从 SurrealDB 批量拉取 LOOP 输入并写入缓存
// ---------------------------------------------------------------------------

/// 批量预取 LOOP 输入数据并写入 geom_input_cache。
///
/// 对每个 refno：
/// - `get_named_attmap` → attmap
/// - `get_world_transform_cache_first` → world_transform
/// - `fetch_loops_and_height` → loops + height
/// - `get_owner_info_from_attr` → owner_refno + owner_type
/// - `get_descendants_by_types` → neg_refnos / cmpf_neg_refnos
pub async fn prefetch_loop_inputs(
    cache: &GeomInputCacheManager,
    db_option: &crate::options::DbOptionExt,
    dbnum: u32,
    refnos: &[RefnoEnum],
) -> anyhow::Result<usize> {
    use aios_core::pdms_types::GENRAL_NEG_NOUN_NAMES;
    use crate::fast_model::query_provider;
    use crate::fast_model::shared;

    if refnos.is_empty() {
        return Ok(0);
    }

    let t = std::time::Instant::now();
    let mut inputs: HashMap<RefnoEnum, LoopInput> = HashMap::new();
    let mut skipped = 0usize;

    for &refno in refnos {
        // 1) attmap
        let attmap = match aios_core::get_named_attmap(refno).await {
            Ok(a) => a,
            Err(_) => {
                skipped += 1;
                continue;
            }
        };

        // 2) world_transform
        let world_transform = match crate::fast_model::transform_cache::get_world_transform_cache_first(
            Some(db_option),
            refno,
        )
        .await
        {
            Ok(Some(t)) => t,
            _ => {
                skipped += 1;
                continue;
            }
        };

        // 3) loops + height
        let loop_res = match aios_core::fetch_loops_and_height(refno).await {
            Ok(r) => r,
            Err(_) => {
                skipped += 1;
                continue;
            }
        };

        // 4) owner
        let (owner_refno, owner_type) = shared::get_owner_info_from_attr(&attmap).await;

        // 5) visible
        let visible = attmap.is_visible_by_level(None).unwrap_or(true);

        // 6) neg_refnos
        let neg_refnos = if !attmap.is_neg() {
            query_provider::get_descendants_by_types(refno, &GENRAL_NEG_NOUN_NAMES, None)
                .await
                .unwrap_or_default()
        } else {
            vec![]
        };

        // 7) cmpf_neg_refnos
        let cmpf_neg_refnos = if !attmap.is_neg() {
            let cmpf_refnos =
                query_provider::get_descendants_by_types(refno, &["CMPF"], None)
                    .await
                    .unwrap_or_default();
            if !cmpf_refnos.is_empty() {
                query_provider::query_multi_descendants(&cmpf_refnos, &GENRAL_NEG_NOUN_NAMES)
                    .await
                    .unwrap_or_default()
            } else {
                vec![]
            }
        } else {
            vec![]
        };

        inputs.insert(
            refno,
            LoopInput {
                refno,
                attmap,
                world_transform,
                loops: loop_res.loops,
                height: loop_res.height,
                owner_refno,
                owner_type,
                visible,
                neg_refnos,
                cmpf_neg_refnos,
            },
        );
    }

    let count = inputs.len();
    if count > 0 {
        let batch_id = cache.next_batch_id(dbnum);
        cache.insert_batch(GeomInputBatch {
            dbnum,
            batch_id,
            created_at: chrono::Utc::now().timestamp_millis(),
            loop_inputs: inputs,
            prim_inputs: HashMap::new(),
        });
    }

    println!(
        "[geom_input_cache] prefetch_loop_inputs: dbnum={}, total={}, cached={}, skipped={}, elapsed={} ms",
        dbnum,
        refnos.len(),
        count,
        skipped,
        t.elapsed().as_millis()
    );

    Ok(count)
}

// ---------------------------------------------------------------------------
// 预取：从 SurrealDB 批量拉取 PRIM 输入并写入缓存
// ---------------------------------------------------------------------------

/// 构造 POHE/POLYHE 所需的多面体额外输入（用于 cache-only 生成）。
///
/// 注意：
/// - 该函数会查询深层节点属性（TreeIndex -> SurrealDB），仅应在“预取阶段”调用。
/// - 生成阶段应只使用 `PrimPolyExtra`（不得再回查 DB）。
pub async fn try_build_prim_poly_extra(refno: RefnoEnum) -> anyhow::Result<Option<PrimPolyExtra>> {
    use crate::fast_model::query_compat::query_filter_deep_children_atts;
    use crate::fast_model::query_provider;

    // 多面体：先取子节点（TreeIndex）
    let pgo_refnos = query_provider::get_children(refno).await.unwrap_or_default();
    if pgo_refnos.is_empty() {
        return Ok(None);
    }

    let first_type = aios_core::get_type_name(pgo_refnos[0])
        .await
        .unwrap_or_default();

    let mut polygons: Vec<PrimPolyPolygon> = Vec::new();
    let mut is_polyhe = false;

    if first_type == "POLPTL" {
        is_polyhe = true;

        // 1) 预取顶点位置：POIN -> POS
        let mut verts_map: HashMap<RefnoEnum, Vec3> = HashMap::new();
        let poin_refnos = query_provider::query_multi_descendants_with_self(&[pgo_refnos[0]], &["POIN"], false)
            .await
            .unwrap_or_default();
        // POIN 顶点位置：批量拉取 attmaps，避免逐点查询。
        let poin_atts = query_provider::get_attmaps_batch(&poin_refnos)
            .await
            .unwrap_or_default();
        for v_attmap in poin_atts {
            let v = v_attmap.get_refno_or_default();
            let pos = v_attmap.get_position().unwrap_or_default();
            verts_map.insert(v, pos);
        }

        // 2) 预取 LOOPTS：POLOOP -> VXREF(POIN...)
        let index_loops = query_filter_deep_children_atts(refno, &["LOOPTS"])
            .await
            .unwrap_or_default();
        let mut index_map: HashMap<RefnoEnum, Vec<RefnoEnum>> = HashMap::new(); // POLOOP -> VXREFs
        for x in &index_loops {
            let owner = x.get_owner(); // POLOOP
            let vx_refnos = x.get_refno_vec("VXREF").unwrap_or_default();
            index_map.entry(owner).or_default().extend(vx_refnos);
        }

        // 3) 预取 POLOOP：POLPTL(owner) + POLOOP(refno) -> 取对应 VXREFs
        let loop_atts = query_filter_deep_children_atts(refno, &["POLOOP"])
            .await
            .unwrap_or_default();
        let mut loops_map: HashMap<RefnoEnum, Vec<Vec<RefnoEnum>>> = HashMap::new(); // POLPTL -> loops(vxrefs)
        for x in &loop_atts {
            let owner = x.get_owner(); // POLPTL
            let poloo_refno = x.get_refno_or_default();
            if let Some(vxrefs) = index_map.get(&poloo_refno) {
                loops_map.entry(owner).or_default().push(vxrefs.clone());
            }
        }

        // 4) 组装 polygons
        for (_polptl, loops_vxrefs) in loops_map {
            let mut loops: Vec<Vec<Vec3>> = Vec::new();
            for vxrefs in loops_vxrefs {
                let mut verts: Vec<Vec3> = Vec::new();
                for index_refno in vxrefs {
                    if let Some(vert) = verts_map.get(&index_refno) {
                        verts.push(*vert);
                    }
                }
                loops.push(verts);
            }
            polygons.push(PrimPolyPolygon { loops });
        }
    } else {
        // 兼容旧逻辑：每个子节点 pgo_refno 的子节点属性中包含 position
        for pgo_refno in pgo_refnos {
            let v_atts = aios_core::collect_children_filter_attrs(pgo_refno, &[])
                .await
                .unwrap_or_default();
            let mut verts: Vec<Vec3> = Vec::new();
            for v in v_atts {
                verts.push(v.get_position().unwrap_or_default());
            }
            polygons.push(PrimPolyPolygon {
                loops: vec![verts],
            });
        }
    }

    Ok(Some(PrimPolyExtra { polygons, is_polyhe }))
}

/// 批量预取 PRIM 输入数据并写入 geom_input_cache。
pub async fn prefetch_prim_inputs(
    cache: &GeomInputCacheManager,
    db_option: &crate::options::DbOptionExt,
    dbnum: u32,
    refnos: &[RefnoEnum],
) -> anyhow::Result<usize> {
    use aios_core::pdms_types::GENRAL_NEG_NOUN_NAMES;
    use crate::fast_model::query_provider;
    use crate::fast_model::shared;

    if refnos.is_empty() {
        return Ok(0);
    }

    let t = std::time::Instant::now();
    let mut inputs: HashMap<RefnoEnum, PrimInput> = HashMap::new();
    let mut skipped = 0usize;

    for &refno in refnos {
        // 1) attmap
        let attmap = match aios_core::get_named_attmap(refno).await {
            Ok(a) => a,
            Err(_) => {
                skipped += 1;
                continue;
            }
        };

        // 2) world_transform
        let world_transform = match crate::fast_model::transform_cache::get_world_transform_cache_first(
            Some(db_option),
            refno,
        )
        .await
        {
            Ok(Some(t)) => t,
            _ => {
                skipped += 1;
                continue;
            }
        };

        // 3) owner
        let (owner_refno, owner_type) = shared::get_owner_info_from_attr(&attmap).await;

        // 4) visible
        let visible = attmap.is_visible_by_level(None).unwrap_or(true);

        // 5) neg_refnos
        let neg_refnos = query_provider::query_multi_descendants_with_self(
            &[refno],
            &GENRAL_NEG_NOUN_NAMES,
            false,
        )
        .await
        .unwrap_or_default();

        // 6) poly_extra（仅 POHE/POLYHE）
        let poly_extra = match attmap.get_type_str() {
            "POHE" | "POLYHE" => match try_build_prim_poly_extra(refno).await {
                Ok(v) => v,
                Err(e) => {
                    eprintln!(
                        "[geom_input_cache] prefetch_prim_inputs: refno={} 构造 poly_extra 失败: {}",
                        refno, e
                    );
                    None
                }
            },
            _ => None,
        };

        inputs.insert(
            refno,
            PrimInput {
                refno,
                attmap,
                world_transform,
                owner_refno,
                owner_type,
                visible,
                neg_refnos,
                poly_extra,
            },
        );
    }

    let count = inputs.len();
    if count > 0 {
        let batch_id = cache.next_batch_id(dbnum);
        cache.insert_batch(GeomInputBatch {
            dbnum,
            batch_id,
            created_at: chrono::Utc::now().timestamp_millis(),
            loop_inputs: HashMap::new(),
            prim_inputs: inputs,
        });
    }

    println!(
        "[geom_input_cache] prefetch_prim_inputs: dbnum={}, total={}, cached={}, skipped={}, elapsed={} ms",
        dbnum,
        refnos.len(),
        count,
        skipped,
        t.elapsed().as_millis()
    );

    Ok(count)
}

// ---------------------------------------------------------------------------
// 全局缓存管理（与 transform_cache 同模式）
// ---------------------------------------------------------------------------

static GLOBAL_GEOM_INPUT_CACHE: OnceCell<GeomInputCacheManager> = OnceCell::const_new();

/// 初始化全局 geom_input_cache（幂等，仅首次生效）。
pub async fn init_global_geom_input_cache(
    db_option: &crate::options::DbOptionExt,
) -> anyhow::Result<()> {
    let dir = db_option
        .get_foyer_cache_dir()
        .join("geom_input_cache");
    let _ = GLOBAL_GEOM_INPUT_CACHE
        .get_or_try_init(|| async move { GeomInputCacheManager::new(&dir).await })
        .await?;
    Ok(())
}

/// 获取全局 geom_input_cache 引用（未初始化返回 None）。
pub fn global_geom_input_cache() -> Option<&'static GeomInputCacheManager> {
    GLOBAL_GEOM_INPUT_CACHE.get()
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CacheRunMode {
    /// 不使用输入缓存，保持原实时查询路径。
    Direct,
    /// 先批量预取输入到 foyer cache，再由模型消费缓存生成。
    PrefetchThenGenerate,
    /// 严格只读缓存，不允许任何回查数据库。
    CacheOnly,
}

impl CacheRunMode {
    pub fn as_str(self) -> &'static str {
        match self {
            CacheRunMode::Direct => "direct",
            CacheRunMode::PrefetchThenGenerate => "prefetch_then_generate",
            CacheRunMode::CacheOnly => "cache_only",
        }
    }
}

fn read_bool_env(name: &str) -> bool {
    std::env::var(name)
        .ok()
        .map(|v| v == "1" || v.to_lowercase() == "true")
        .unwrap_or(false)
}

/// 统一解析输入缓存运行模式。
///
/// 优先级：
/// 1) `AIOS_GEN_INPUT_CACHE_ONLY=1` -> `CacheOnly`
/// 2) `AIOS_GEN_INPUT_CACHE=1`      -> `PrefetchThenGenerate`
/// 3) 其他                           -> `Direct`
pub fn resolve_cache_run_mode() -> CacheRunMode {
    if read_bool_env("AIOS_GEN_INPUT_CACHE_ONLY") {
        return CacheRunMode::CacheOnly;
    }
    if read_bool_env("AIOS_GEN_INPUT_CACHE") {
        return CacheRunMode::PrefetchThenGenerate;
    }
    CacheRunMode::Direct
}

/// 检查是否启用了输入缓存模式（环境变量 `AIOS_GEN_INPUT_CACHE=1`）。
pub fn is_geom_input_cache_enabled() -> bool {
    matches!(
        resolve_cache_run_mode(),
        CacheRunMode::PrefetchThenGenerate | CacheRunMode::CacheOnly
    )
}

/// 检查是否要求仅从缓存读取（环境变量 `AIOS_GEN_INPUT_CACHE_ONLY=1`）。
pub fn is_geom_input_cache_only() -> bool {
    matches!(resolve_cache_run_mode(), CacheRunMode::CacheOnly)
}

/// 检查是否启用了输入缓存流水线模式（环境变量 `AIOS_GEN_INPUT_CACHE_PIPELINE=1`）。
pub fn is_geom_input_cache_pipeline_enabled() -> bool {
    read_bool_env("AIOS_GEN_INPUT_CACHE_PIPELINE")
}

// ---------------------------------------------------------------------------
// Orchestrator 入口：按 dbnum 分组预取 LOOP/PRIM 输入
// ---------------------------------------------------------------------------

/// 预取指定 refnos 的 LOOP/PRIM 输入数据到 geom_input_cache。
///
/// 按 dbnum 分组，分别调用 `prefetch_loop_inputs` / `prefetch_prim_inputs`。
/// 需要先调用 `init_global_geom_input_cache` 初始化全局缓存。
pub async fn prefetch_all_geom_inputs(
    db_option: &crate::options::DbOptionExt,
    loop_refnos: &[RefnoEnum],
    prim_refnos: &[RefnoEnum],
) -> anyhow::Result<(usize, usize)> {
    let cache = global_geom_input_cache()
        .ok_or_else(|| anyhow::anyhow!("geom_input_cache 未初始化"))?;

    let t = std::time::Instant::now();

    // 按 dbnum 分组
    let db_meta = crate::data_interface::db_meta_manager::db_meta();
    let _ = db_meta.ensure_loaded();

    let mut loop_groups: HashMap<u32, Vec<RefnoEnum>> = HashMap::new();
    for &r in loop_refnos {
        let dbnum = db_meta
            .get_dbnum_by_refno(r)
            .ok_or_else(|| anyhow::anyhow!("缺少 ref0->dbnum 映射: refno={}", r))?;
        loop_groups.entry(dbnum).or_default().push(r);
    }

    let mut prim_groups: HashMap<u32, Vec<RefnoEnum>> = HashMap::new();
    for &r in prim_refnos {
        let dbnum = db_meta
            .get_dbnum_by_refno(r)
            .ok_or_else(|| anyhow::anyhow!("缺少 ref0->dbnum 映射: refno={}", r))?;
        prim_groups.entry(dbnum).or_default().push(r);
    }

    let mut total_loop = 0usize;
    let mut total_prim = 0usize;

    for (dbnum, refs) in loop_groups {
        match prefetch_loop_inputs(cache, db_option, dbnum, &refs).await {
            Ok(n) => total_loop += n,
            Err(e) => eprintln!(
                "[geom_input_cache] prefetch_loop dbnum={} 失败: {}",
                dbnum, e
            ),
        }
    }

    for (dbnum, refs) in prim_groups {
        match prefetch_prim_inputs(cache, db_option, dbnum, &refs).await {
            Ok(n) => total_prim += n,
            Err(e) => eprintln!(
                "[geom_input_cache] prefetch_prim dbnum={} 失败: {}",
                dbnum, e
            ),
        }
    }

    println!(
        "[geom_input_cache] prefetch_all 完成: loop={}, prim={}, elapsed={} ms",
        total_loop,
        total_prim,
        t.elapsed().as_millis()
    );

    Ok((total_loop, total_prim))
}

/// 从全局 geom_input_cache 加载指定 dbnum 的所有 LOOP 输入。
pub async fn load_loop_inputs_from_global(dbnum: u32) -> HashMap<RefnoEnum, LoopInput> {
    match global_geom_input_cache() {
        Some(cache) => cache.get_all_loop_inputs(dbnum).await,
        None => HashMap::new(),
    }
}

/// 从全局 geom_input_cache 加载指定 dbnum 的所有 PRIM 输入。
pub async fn load_prim_inputs_from_global(dbnum: u32) -> HashMap<RefnoEnum, PrimInput> {
    match global_geom_input_cache() {
        Some(cache) => cache.get_all_prim_inputs(dbnum).await,
        None => HashMap::new(),
    }
}

pub fn group_refnos_by_dbnum_strict(
    refnos: &[RefnoEnum],
) -> anyhow::Result<HashMap<u32, Vec<RefnoEnum>>> {
    let db_meta = crate::data_interface::db_meta_manager::db_meta();
    let _ = db_meta.ensure_loaded();

    let mut groups: HashMap<u32, Vec<RefnoEnum>> = HashMap::new();
    for &refno in refnos {
        let dbnum = db_meta
            .get_dbnum_by_refno(refno)
            .ok_or_else(|| anyhow::anyhow!("缺少 ref0->dbnum 映射: refno={}", refno))?;
        groups.entry(dbnum).or_default().push(refno);
    }
    Ok(groups)
}

/// 按 refno 集合加载 LOOP 输入（严格按 dbnum 分桶，不扫描全库）。
pub async fn load_loop_inputs_for_refnos_from_global(
    refnos: &[RefnoEnum],
) -> anyhow::Result<HashMap<RefnoEnum, LoopInput>> {
    if refnos.is_empty() {
        return Ok(HashMap::new());
    }
    let cache = global_geom_input_cache()
        .ok_or_else(|| anyhow::anyhow!("geom_input_cache 未初始化"))?;
    let groups = group_refnos_by_dbnum_strict(refnos)?;

    let mut result = HashMap::new();
    for (dbnum, refs) in groups {
        let mut db_inputs = cache.get_all_loop_inputs(dbnum).await;
        for refno in refs {
            if let Some(input) = db_inputs.remove(&refno) {
                result.insert(refno, input);
            }
        }
    }
    Ok(result)
}

/// 按 refno 集合加载 PRIM 输入（严格按 dbnum 分桶，不扫描全库）。
pub async fn load_prim_inputs_for_refnos_from_global(
    refnos: &[RefnoEnum],
) -> anyhow::Result<HashMap<RefnoEnum, PrimInput>> {
    if refnos.is_empty() {
        return Ok(HashMap::new());
    }
    let cache = global_geom_input_cache()
        .ok_or_else(|| anyhow::anyhow!("geom_input_cache 未初始化"))?;
    let groups = group_refnos_by_dbnum_strict(refnos)?;

    let mut result = HashMap::new();
    for (dbnum, refs) in groups {
        let mut db_inputs = cache.get_all_prim_inputs(dbnum).await;
        for refno in refs {
            if let Some(input) = db_inputs.remove(&refno) {
                result.insert(refno, input);
            }
        }
    }
    Ok(result)
}

/// 从全局 geom_input_cache 加载所有 dbnum 的 LOOP 输入。
pub async fn load_all_loop_inputs_from_global() -> HashMap<RefnoEnum, LoopInput> {
    let Some(cache) = global_geom_input_cache() else {
        return HashMap::new();
    };
    let mut result = HashMap::new();
    for dbnum in cache.list_dbnums() {
        result.extend(cache.get_all_loop_inputs(dbnum).await);
    }
    result
}

/// 从全局 geom_input_cache 加载所有 dbnum 的 PRIM 输入。
pub async fn load_all_prim_inputs_from_global() -> HashMap<RefnoEnum, PrimInput> {
    let Some(cache) = global_geom_input_cache() else {
        return HashMap::new();
    };
    let mut result = HashMap::new();
    for dbnum in cache.list_dbnums() {
        result.extend(cache.get_all_prim_inputs(dbnum).await);
    }
    result
}
