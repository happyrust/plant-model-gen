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

use aios_core::pdms_types::PdmsGenericType;
use aios_core::types::NamedAttrMap;
use aios_core::RefnoEnum;
use bevy_transform::components::Transform;
use foyer::{DirectFsDeviceOptionsBuilder, HybridCache, HybridCacheBuilder};
use glam::Vec3;
use serde::{Deserialize, Serialize};
use tokio::sync::OnceCell;
use twox_hash::XxHash64;

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
    pub generic_type: PdmsGenericType,
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
    pub generic_type: PdmsGenericType,
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
        let payload = match serde_json::to_vec(&batch) {
            Ok(bytes) => bytes,
            Err(e) => {
                eprintln!(
                    "[geom_input_cache] 序列化失败: dbnum={}, batch_id={}, err={}",
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
                serde_json::from_slice::<GeomInputBatch>(payload)
                    .map_err(|e| {
                        eprintln!(
                            "[geom_input_cache] 反序列化失败: dbnum={}, batch_id={}, err={}",
                            dbnum, batch_id, e
                        );
                    })
                    .ok()
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
/// - `get_generic_type` → generic_type
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

        // 6) generic_type
        let generic_type = crate::fast_model::get_generic_type(refno)
            .await
            .unwrap_or_default();

        // 7) neg_refnos
        let neg_refnos = if !attmap.is_neg() {
            query_provider::get_descendants_by_types(refno, &GENRAL_NEG_NOUN_NAMES, None)
                .await
                .unwrap_or_default()
        } else {
            vec![]
        };

        // 8) cmpf_neg_refnos
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
                generic_type,
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
        for v in poin_refnos {
            let v_attmap = aios_core::get_named_attmap(v).await.unwrap_or_default();
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

        // 5) generic_type
        let generic_type = crate::fast_model::get_generic_type(refno)
            .await
            .unwrap_or_default();

        // 6) neg_refnos
        let neg_refnos = query_provider::query_multi_descendants_with_self(
            &[refno],
            &GENRAL_NEG_NOUN_NAMES,
            false,
        )
        .await
        .unwrap_or_default();

        // 7) poly_extra（仅 POHE/POLYHE）
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
                generic_type,
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

/// 检查是否启用了输入缓存模式（环境变量 `AIOS_GEN_INPUT_CACHE=1`）。
pub fn is_geom_input_cache_enabled() -> bool {
    std::env::var("AIOS_GEN_INPUT_CACHE")
        .ok()
        .map(|v| v == "1" || v.to_lowercase() == "true")
        .unwrap_or(false)
}

/// 检查是否要求仅从缓存读取（环境变量 `AIOS_GEN_INPUT_CACHE_ONLY=1`）。
pub fn is_geom_input_cache_only() -> bool {
    std::env::var("AIOS_GEN_INPUT_CACHE_ONLY")
        .ok()
        .map(|v| v == "1" || v.to_lowercase() == "true")
        .unwrap_or(false)
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
        let dbnum = db_meta.get_dbnum_by_refno(r).unwrap_or(0);
        if dbnum > 0 {
            loop_groups.entry(dbnum).or_default().push(r);
        }
    }

    let mut prim_groups: HashMap<u32, Vec<RefnoEnum>> = HashMap::new();
    for &r in prim_refnos {
        let dbnum = db_meta.get_dbnum_by_refno(r).unwrap_or(0);
        if dbnum > 0 {
            prim_groups.entry(dbnum).or_default().push(r);
        }
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
