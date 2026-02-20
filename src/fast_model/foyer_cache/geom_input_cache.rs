//! LOOP/PRIM/CATE 输入纯内存缓存
//!
//! 通过预取 LOOP/PRIM/CATE 所需的几何与属性数据到内存 DashMap，
//! 在实例生成阶段只读缓存，降低对 SurrealDB 的依赖（尤其是 Full Noun 流水线）。
//!
//! 数据结构：
//! - `LoopInput`：LOOP 类元素的预取输入（attmap、world_transform、loops、height 等）
//! - `PrimInput`：PRIM 类元素的预取输入（attmap、world_transform 等）
//! - `CateInput`：CATE 类元素的预取输入（attmap、world_transform 等；几何解析走 cata_resolve_cache）
//!
//! 使用流程：
//! 1. 预取阶段：`prefetch_*_inputs` 从 SurrealDB 批量拉取并写入缓存
//! 2. 生成阶段：`*_from_inputs` 仅从缓存读取

use std::collections::HashMap;

use aios_core::types::NamedAttrMap;
use aios_core::RefnoEnum;
use aios_core::Transform;
use dashmap::DashMap;
use dashmap::mapref::entry::Entry;
use glam::Vec3;
use serde::{Deserialize, Serialize};
use std::sync::OnceLock;

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
/// - 该结构用于让"模型生成阶段"在 cache-only 条件下也能构建 Polyhedron，
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

/// CATE 类元素的预取输入（实例级信息）。
///
/// 注意：
/// - CATE 的"可复用几何准备结果"不在此缓存（见 `foyer_cache/cata_resolve_cache`，按 cata_hash 缓存）。
/// - 这里主要缓存每个 refno 的 inst_info 相关字段，以便 Generate 阶段不回查 SurrealDB。
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CateInput {
    pub refno: RefnoEnum,
    pub attmap: NamedAttrMap,
    pub world_transform: Transform,
    pub owner_refno: RefnoEnum,
    pub owner_type: String,
    pub visible: bool,
}

// ---------------------------------------------------------------------------
// GeomInputCacheManager（纯内存 DashMap）
// ---------------------------------------------------------------------------

pub struct GeomInputCacheManager {
    loop_inputs: DashMap<(u32, RefnoEnum), LoopInput>,
    prim_inputs: DashMap<(u32, RefnoEnum), PrimInput>,
    cate_inputs: DashMap<(u32, RefnoEnum), CateInput>,
    pins: DashMap<(u32, RefnoEnum), u32>,
}

impl GeomInputCacheManager {
    pub fn new() -> Self {
        Self {
            loop_inputs: DashMap::new(),
            prim_inputs: DashMap::new(),
            cate_inputs: DashMap::new(),
            pins: DashMap::new(),
        }
    }

    /// 写入单个 LOOP 输入
    pub fn insert_loop_input(&self, dbnum: u32, refno: RefnoEnum, input: &LoopInput) {
        self.loop_inputs.insert((dbnum, refno), input.clone());
    }

    /// 写入单个 PRIM 输入
    pub fn insert_prim_input(&self, dbnum: u32, refno: RefnoEnum, input: &PrimInput) {
        self.prim_inputs.insert((dbnum, refno), input.clone());
    }

    /// 写入单个 CATE 输入
    pub fn insert_cate_input(&self, dbnum: u32, refno: RefnoEnum, input: &CateInput) {
        self.cate_inputs.insert((dbnum, refno), input.clone());
    }

    // -----------------------------------------------------------------------
    // 读取 API（per-refno，同步）
    // -----------------------------------------------------------------------

    /// 读取单个 LOOP 输入
    pub fn get_loop_input(&self, dbnum: u32, refno: RefnoEnum) -> Option<LoopInput> {
        self.loop_inputs.get(&(dbnum, refno)).map(|v| v.clone())
    }

    /// 读取单个 PRIM 输入
    pub fn get_prim_input(&self, dbnum: u32, refno: RefnoEnum) -> Option<PrimInput> {
        self.prim_inputs.get(&(dbnum, refno)).map(|v| v.clone())
    }

    /// 读取单个 CATE 输入
    pub fn get_cate_input(&self, dbnum: u32, refno: RefnoEnum) -> Option<CateInput> {
        self.cate_inputs.get(&(dbnum, refno)).map(|v| v.clone())
    }

    // -----------------------------------------------------------------------
    // 批量读取 API
    // -----------------------------------------------------------------------

    /// 列出所有已缓存的 dbnum
    pub fn list_dbnums(&self) -> Vec<u32> {
        let mut dbnums = std::collections::HashSet::new();
        for entry in self.loop_inputs.iter() {
            dbnums.insert(entry.key().0);
        }
        for entry in self.prim_inputs.iter() {
            dbnums.insert(entry.key().0);
        }
        for entry in self.cate_inputs.iter() {
            dbnums.insert(entry.key().0);
        }
        dbnums.into_iter().collect()
    }

    /// 读取指定 dbnum 下所有 loop inputs
    pub fn get_all_loop_inputs(&self, dbnum: u32) -> HashMap<RefnoEnum, LoopInput> {
        let mut result = HashMap::new();
        for entry in self.loop_inputs.iter() {
            if entry.key().0 == dbnum {
                result.insert(entry.key().1, entry.value().clone());
            }
        }
        result
    }

    /// 读取指定 dbnum 下所有 prim inputs
    pub fn get_all_prim_inputs(&self, dbnum: u32) -> HashMap<RefnoEnum, PrimInput> {
        let mut result = HashMap::new();
        for entry in self.prim_inputs.iter() {
            if entry.key().0 == dbnum {
                result.insert(entry.key().1, entry.value().clone());
            }
        }
        result
    }

    /// 读取指定 dbnum 下所有 cate inputs
    pub fn get_all_cate_inputs(&self, dbnum: u32) -> HashMap<RefnoEnum, CateInput> {
        let mut result = HashMap::new();
        for entry in self.cate_inputs.iter() {
            if entry.key().0 == dbnum {
                result.insert(entry.key().1, entry.value().clone());
            }
        }
        result
    }

    // -----------------------------------------------------------------------
    // 删除
    // -----------------------------------------------------------------------

    /// 删除指定 dbnum 下的所有缓存数据
    pub fn remove_dbnum(&self, dbnum: u32) -> usize {
        let mut count = 0usize;
        self.loop_inputs.retain(|k, _| {
            if k.0 == dbnum { count += 1; false } else { true }
        });
        self.prim_inputs.retain(|k, _| {
            if k.0 == dbnum { count += 1; false } else { true }
        });
        self.cate_inputs.retain(|k, _| {
            if k.0 == dbnum { count += 1; false } else { true }
        });
        count
    }

    /// 清空所有 LOOP/PRIM/CATE 输入缓存，释放内存（分批生成时在批次间调用）。
    pub fn clear(&self) -> usize {
        let count = self.loop_inputs.len() + self.prim_inputs.len() + self.cate_inputs.len();
        self.loop_inputs.clear();
        self.prim_inputs.clear();
        self.cate_inputs.clear();
        count
    }

    /// 按 refno 定向清理缓存，避免并发任务之间互相清空全局缓存。
    pub fn clear_refnos(&self, refnos: &[RefnoEnum]) -> usize {
        if refnos.is_empty() {
            return 0;
        }

        let keys = Self::map_refnos_to_keys(refnos);
        let mut count = 0usize;
        for (dbnum, refno) in keys {
            if self.loop_inputs.remove(&(dbnum, refno)).is_some() {
                count += 1;
            }
            if self.prim_inputs.remove(&(dbnum, refno)).is_some() {
                count += 1;
            }
            if self.cate_inputs.remove(&(dbnum, refno)).is_some() {
                count += 1;
            }
        }
        count
    }

    /// 为一组 refno 增加缓存租约（pin），用于并发任务隔离。
    pub fn pin_refnos(&self, refnos: &[RefnoEnum]) -> usize {
        if refnos.is_empty() {
            return 0;
        }
        let keys = Self::map_refnos_to_keys(refnos);
        self.pin_keys(&keys)
    }

    /// 释放一组 refno 的缓存租约（不清理缓存条目）。
    pub fn unpin_refnos(&self, refnos: &[RefnoEnum]) -> usize {
        if refnos.is_empty() {
            return 0;
        }
        let keys = Self::map_refnos_to_keys(refnos);
        self.unpin_keys(&keys)
    }

    /// 释放一组 refno 的缓存租约，并在无人持有时清理对应缓存条目。
    pub fn release_refnos_and_clear(&self, refnos: &[RefnoEnum]) -> usize {
        if refnos.is_empty() {
            return 0;
        }
        let keys = Self::map_refnos_to_keys(refnos);
        self.release_keys_and_clear(&keys)
    }

    fn map_refnos_to_keys(refnos: &[RefnoEnum]) -> Vec<(u32, RefnoEnum)> {
        let db_meta = crate::data_interface::db_meta_manager::db_meta();
        let _ = db_meta.ensure_loaded();

        let mut keys = Vec::with_capacity(refnos.len());
        for &refno in refnos {
            let Some(dbnum) = db_meta.get_dbnum_by_refno(refno) else {
                continue;
            };
            keys.push((dbnum, refno));
        }
        keys
    }

    fn pin_keys(&self, keys: &[(u32, RefnoEnum)]) -> usize {
        let mut pinned = 0usize;
        for &key in keys {
            match self.pins.entry(key) {
                Entry::Occupied(mut occ) => {
                    *occ.get_mut() += 1;
                }
                Entry::Vacant(vac) => {
                    vac.insert(1);
                }
            }
            pinned += 1;
        }
        pinned
    }

    fn unpin_keys(&self, keys: &[(u32, RefnoEnum)]) -> usize {
        let mut released = 0usize;
        for &key in keys {
            match self.pins.entry(key) {
                Entry::Occupied(mut occ) => {
                    let cnt = occ.get_mut();
                    if *cnt > 1 {
                        *cnt -= 1;
                    } else {
                        occ.remove();
                    }
                    released += 1;
                }
                Entry::Vacant(_) => {}
            }
        }
        released
    }

    fn release_keys_and_clear(&self, keys: &[(u32, RefnoEnum)]) -> usize {
        let mut removed = 0usize;
        for &key in keys {
            let mut can_clear = true;

            match self.pins.entry(key) {
                Entry::Occupied(mut occ) => {
                    let cnt = occ.get_mut();
                    if *cnt > 1 {
                        *cnt -= 1;
                        can_clear = false;
                    } else {
                        occ.remove();
                    }
                }
                Entry::Vacant(_) => {
                    // 未被 pin 的历史调用，维持兼容：允许直接清理。
                }
            }

            if can_clear {
                if self.loop_inputs.remove(&key).is_some() {
                    removed += 1;
                }
                if self.prim_inputs.remove(&key).is_some() {
                    removed += 1;
                }
                if self.cate_inputs.remove(&key).is_some() {
                    removed += 1;
                }
            }
        }
        removed
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

    if refnos.is_empty() {
        return Ok(0);
    }

    let t = std::time::Instant::now();
    let mut inputs: HashMap<RefnoEnum, LoopInput> = HashMap::new();
    let mut skipped = 0usize;

    // 1) attmap：批量拉取，避免逐 refno 查询
    let mut attmap_map: HashMap<RefnoEnum, aios_core::NamedAttrMap> = HashMap::new();
    match query_provider::get_attmaps_batch(refnos).await {
        Ok(list) => {
            for att in list {
                let r = att.get_refno_or_default();
                if r.is_valid() {
                    attmap_map.insert(r, att);
                }
            }
        }
        Err(e) => {
            eprintln!(
                "[geom_input_cache] prefetch_loop_inputs: dbnum={} 批量获取 attmaps 失败: {}",
                dbnum, e
            );
        }
    }

    // 2) world_transform：批量 cache-first 获取（miss 批量查 pe_transform）
    let world_map = match crate::fast_model::transform_cache::get_world_transforms_cache_first_batch(
        Some(db_option),
        refnos,
    )
    .await
    {
        Ok(m) => m,
        Err(e) => {
            eprintln!(
                "[geom_input_cache] prefetch_loop_inputs: dbnum={} 批量获取 world_transforms 失败: {}",
                dbnum, e
            );
            HashMap::new()
        }
    };

    // 3) owner_type：按 owner_refno 去重后批量取 PE（避免 shared::get_owner_info_from_attr 的逐个 get_pe）
    let mut owner_set: std::collections::HashSet<RefnoEnum> = std::collections::HashSet::new();
    for a in attmap_map.values() {
        let o = a.get_owner();
        if o != RefnoEnum::default() && o.is_valid() {
            owner_set.insert(o);
        }
    }
    let owner_vec: Vec<RefnoEnum> = owner_set.into_iter().collect();
    let mut owner_type_map: HashMap<RefnoEnum, String> = HashMap::new();
    if !owner_vec.is_empty() {
        match query_provider::get_pes_batch(&owner_vec).await {
            Ok(pes) => {
                for pe in pes {
                    owner_type_map.insert(pe.refno, pe.get_type_str().to_string());
                }
            }
            Err(e) => {
                eprintln!(
                    "[geom_input_cache] prefetch_loop_inputs: dbnum={} 批量获取 owner PEs 失败: {}",
                    dbnum, e
                );
            }
        }
    }

    for &refno in refnos {
        // 1) attmap（来自批量拉取）
        let Some(attmap) = attmap_map.get(&refno).cloned() else {
            skipped += 1;
            continue;
        };

        // 2) world_transform（来自批量 cache-first）
        let Some(world_transform) = world_map.get(&refno).copied() else {
            skipped += 1;
            continue;
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
        let owner_refno = attmap.get_owner();
        let owner_type = owner_type_map
            .get(&owner_refno)
            .cloned()
            .unwrap_or_default();

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
    for (refno, input) in &inputs {
        cache.insert_loop_input(dbnum, *refno, input);
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
/// - 该函数会查询深层节点属性（TreeIndex -> SurrealDB），仅应在"预取阶段"调用。
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

    if refnos.is_empty() {
        return Ok(0);
    }

    let t = std::time::Instant::now();
    let mut inputs: HashMap<RefnoEnum, PrimInput> = HashMap::new();
    let mut skipped = 0usize;

    // 1) attmap：批量拉取，避免逐 refno 查询
    let mut attmap_map: HashMap<RefnoEnum, aios_core::NamedAttrMap> = HashMap::new();
    match query_provider::get_attmaps_batch(refnos).await {
        Ok(list) => {
            for att in list {
                let r = att.get_refno_or_default();
                if r.is_valid() {
                    attmap_map.insert(r, att);
                }
            }
        }
        Err(e) => {
            eprintln!(
                "[geom_input_cache] prefetch_prim_inputs: dbnum={} 批量获取 attmaps 失败: {}",
                dbnum, e
            );
        }
    }

    // 2) world_transform：批量 cache-first 获取（miss 批量查 pe_transform）
    let world_map = match crate::fast_model::transform_cache::get_world_transforms_cache_first_batch(
        Some(db_option),
        refnos,
    )
    .await
    {
        Ok(m) => m,
        Err(e) => {
            eprintln!(
                "[geom_input_cache] prefetch_prim_inputs: dbnum={} 批量获取 world_transforms 失败: {}",
                dbnum, e
            );
            HashMap::new()
        }
    };

    // 3) owner_type：按 owner_refno 去重后批量取 PE
    let mut owner_set: std::collections::HashSet<RefnoEnum> = std::collections::HashSet::new();
    for a in attmap_map.values() {
        let o = a.get_owner();
        if o != RefnoEnum::default() && o.is_valid() {
            owner_set.insert(o);
        }
    }
    let owner_vec: Vec<RefnoEnum> = owner_set.into_iter().collect();
    let mut owner_type_map: HashMap<RefnoEnum, String> = HashMap::new();
    if !owner_vec.is_empty() {
        match query_provider::get_pes_batch(&owner_vec).await {
            Ok(pes) => {
                for pe in pes {
                    owner_type_map.insert(pe.refno, pe.get_type_str().to_string());
                }
            }
            Err(e) => {
                eprintln!(
                    "[geom_input_cache] prefetch_prim_inputs: dbnum={} 批量获取 owner PEs 失败: {}",
                    dbnum, e
                );
            }
        }
    }

    for &refno in refnos {
        // 1) attmap（来自批量拉取）
        let Some(attmap) = attmap_map.get(&refno).cloned() else {
            skipped += 1;
            continue;
        };

        // 2) world_transform（来自批量 cache-first）
        let Some(world_transform) = world_map.get(&refno).copied() else {
            skipped += 1;
            continue;
        };

        // 3) owner
        let owner_refno = attmap.get_owner();
        let owner_type = owner_type_map
            .get(&owner_refno)
            .cloned()
            .unwrap_or_default();

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
    for (refno, input) in &inputs {
        cache.insert_prim_input(dbnum, *refno, input);
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

/// 批量预取 CATE 输入数据并写入 geom_input_cache。
///
/// 说明：
/// - CATE 的 prepared geos/ptset 不在此处缓存（见 `cata_resolve_cache`，按 cata_hash 缓存）。
/// - 本函数仅缓存每个 refno 的 inst_info 级别字段（attmap/world_transform/owner/visible）。
pub async fn prefetch_cate_inputs(
    cache: &GeomInputCacheManager,
    db_option: &crate::options::DbOptionExt,
    dbnum: u32,
    refnos: &[RefnoEnum],
) -> anyhow::Result<usize> {
    use crate::fast_model::query_provider;

    if refnos.is_empty() {
        return Ok(0);
    }

    let t = std::time::Instant::now();
    let mut inputs: HashMap<RefnoEnum, CateInput> = HashMap::new();
    let mut skipped = 0usize;

    // 1) attmap：批量拉取
    let mut attmap_map: HashMap<RefnoEnum, aios_core::NamedAttrMap> = HashMap::new();
    match query_provider::get_attmaps_batch(refnos).await {
        Ok(list) => {
            for att in list {
                let r = att.get_refno_or_default();
                if r.is_valid() {
                    attmap_map.insert(r, att);
                }
            }
        }
        Err(e) => {
            eprintln!(
                "[geom_input_cache] prefetch_cate_inputs: dbnum={} 批量获取 attmaps 失败: {}",
                dbnum, e
            );
        }
    }

    // 2) world_transform：批量 cache-first 获取
    let world_map = match crate::fast_model::transform_cache::get_world_transforms_cache_first_batch(
        Some(db_option),
        refnos,
    )
    .await
    {
        Ok(m) => m,
        Err(e) => {
            eprintln!(
                "[geom_input_cache] prefetch_cate_inputs: dbnum={} 批量获取 world_transforms 失败: {}",
                dbnum, e
            );
            HashMap::new()
        }
    };

    // 3) owner_type：按 owner_refno 去重后批量取 PE
    let mut owner_set: std::collections::HashSet<RefnoEnum> = std::collections::HashSet::new();
    for a in attmap_map.values() {
        let o = a.get_owner();
        if o != RefnoEnum::default() && o.is_valid() {
            owner_set.insert(o);
        }
    }
    let owner_vec: Vec<RefnoEnum> = owner_set.into_iter().collect();
    let mut owner_type_map: HashMap<RefnoEnum, String> = HashMap::new();
    if !owner_vec.is_empty() {
        match query_provider::get_pes_batch(&owner_vec).await {
            Ok(pes) => {
                for pe in pes {
                    owner_type_map.insert(pe.refno, pe.get_type_str().to_string());
                }
            }
            Err(e) => {
                eprintln!(
                    "[geom_input_cache] prefetch_cate_inputs: dbnum={} 批量获取 owner PEs 失败: {}",
                    dbnum, e
                );
            }
        }
    }

    for &refno in refnos {
        let Some(attmap) = attmap_map.get(&refno).cloned() else {
            skipped += 1;
            continue;
        };
        let Some(world_transform) = world_map.get(&refno).copied() else {
            skipped += 1;
            continue;
        };

        let owner_refno = attmap.get_owner();
        let owner_type = owner_type_map
            .get(&owner_refno)
            .cloned()
            .unwrap_or_default();
        let visible = attmap.is_visible_by_level(None).unwrap_or(true);

        inputs.insert(
            refno,
            CateInput {
                refno,
                attmap,
                world_transform,
                owner_refno,
                owner_type,
                visible,
            },
        );
    }

    let count = inputs.len();
    for (refno, input) in &inputs {
        cache.insert_cate_input(dbnum, *refno, input);
    }

    println!(
        "[geom_input_cache] prefetch_cate_inputs: dbnum={}, total={}, cached={}, skipped={}, elapsed={} ms",
        dbnum,
        refnos.len(),
        count,
        skipped,
        t.elapsed().as_millis()
    );

    Ok(count)
}

// ---------------------------------------------------------------------------
// 全局缓存管理（纯内存，无需 db_option）
// ---------------------------------------------------------------------------

static GLOBAL_GEOM_INPUT_CACHE: OnceLock<GeomInputCacheManager> = OnceLock::new();

/// 初始化全局 geom_input_cache（幂等，仅首次生效）。
pub fn init_global_geom_input_cache() {
    let _ = GLOBAL_GEOM_INPUT_CACHE.get_or_init(|| GeomInputCacheManager::new());
}

/// 获取全局 geom_input_cache 引用（未初始化返回 None）。
pub fn global_geom_input_cache() -> Option<&'static GeomInputCacheManager> {
    GLOBAL_GEOM_INPUT_CACHE.get()
}

/// 清空全局 geom_input 缓存（分批生成时在批次间调用）。
pub fn clear_global_geom_input_cache() -> usize {
    GLOBAL_GEOM_INPUT_CACHE.get()
        .map(|mgr| mgr.clear())
        .unwrap_or(0)
}

/// 按 refno 清理全局 geom_input 缓存（用于分批任务隔离）。
pub fn clear_global_geom_input_cache_for_refnos(refnos: &[RefnoEnum]) -> usize {
    GLOBAL_GEOM_INPUT_CACHE
        .get()
        .map(|mgr| mgr.release_refnos_and_clear(refnos))
        .unwrap_or(0)
}

/// 为一组 refno 增加全局 geom_input 缓存租约（pin）。
pub fn pin_global_geom_input_cache_for_refnos(refnos: &[RefnoEnum]) -> usize {
    GLOBAL_GEOM_INPUT_CACHE
        .get()
        .map(|mgr| mgr.pin_refnos(refnos))
        .unwrap_or(0)
}

/// 释放一组 refno 的全局 geom_input 缓存租约（不清理条目）。
pub fn release_global_geom_input_cache_for_refnos(refnos: &[RefnoEnum]) -> usize {
    GLOBAL_GEOM_INPUT_CACHE
        .get()
        .map(|mgr| mgr.unpin_refnos(refnos))
        .unwrap_or(0)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CacheRunMode {
    /// 不使用输入缓存，保持原实时查询路径。
    Direct,
    /// 先批量预取输入到缓存，再由模型消费缓存生成。
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
// Orchestrator 入口：按 dbnum 分组预取 LOOP/PRIM/CATE 输入
// ---------------------------------------------------------------------------

/// 预取指定 refnos 的 LOOP/PRIM/CATE 输入数据到 geom_input_cache。
///
/// 按 dbnum 分组，分别调用 `prefetch_loop_inputs` / `prefetch_prim_inputs` / `prefetch_cate_inputs`。
/// 需要先调用 `init_global_geom_input_cache` 初始化全局缓存。
pub async fn prefetch_all_geom_inputs(
    db_option: &crate::options::DbOptionExt,
    loop_refnos: &[RefnoEnum],
    prim_refnos: &[RefnoEnum],
    cate_refnos: &[RefnoEnum],
) -> anyhow::Result<(usize, usize, usize)> {
    let cache = global_geom_input_cache()
        .ok_or_else(|| anyhow::anyhow!("geom_input_cache 未初始化"))?;

    let t = std::time::Instant::now();

    let loop_groups = group_refnos_by_dbnum_strict(loop_refnos)?;
    let prim_groups = group_refnos_by_dbnum_strict(prim_refnos)?;
    let cate_groups = group_refnos_by_dbnum_strict(cate_refnos)?;

    let mut total_loop = 0usize;
    let mut total_prim = 0usize;
    let mut total_cate = 0usize;

    for (dbnum, refs) in loop_groups {
        total_loop += prefetch_loop_inputs(cache, db_option, dbnum, &refs).await?;
    }

    for (dbnum, refs) in prim_groups {
        total_prim += prefetch_prim_inputs(cache, db_option, dbnum, &refs).await?;
    }

    for (dbnum, refs) in cate_groups {
        total_cate += prefetch_cate_inputs(cache, db_option, dbnum, &refs).await?;
    }

    println!(
        "[geom_input_cache] prefetch_all 完成: loop={}, prim={}, cate={}, elapsed={} ms",
        total_loop,
        total_prim,
        total_cate,
        t.elapsed().as_millis()
    );

    Ok((total_loop, total_prim, total_cate))
}

/// 校验：指定 refnos 的 LOOP/PRIM/CATE 输入是否已完整落入全局 geom_input_cache。
///
/// 约定：用于 PrefetchThenGenerate 的"预取完成 -> 进入离线生成"前的完整性校验。
/// - 若有缺失，直接返回错误（离线 Generate 阶段不允许回查 DB；miss 视为流程不正确）。
/// - 调用方需保证已先 `init_global_geom_input_cache`。
pub fn ensure_geom_inputs_present_for_refnos_from_global(
    loop_refnos: &[RefnoEnum],
    prim_refnos: &[RefnoEnum],
    cate_refnos: &[RefnoEnum],
) -> anyhow::Result<()> {
    fn sample_refnos(missing: &[RefnoEnum], limit: usize) -> String {
        missing
            .iter()
            .take(limit)
            .map(|r| r.to_string())
            .collect::<Vec<_>>()
            .join(", ")
    }

    const SAMPLE_LIMIT: usize = 32;

    if !loop_refnos.is_empty() {
        let got = load_loop_inputs_for_refnos_from_global(loop_refnos)?;
        if got.len() != loop_refnos.len() {
            let mut missing: Vec<RefnoEnum> = loop_refnos
                .iter()
                .copied()
                .filter(|r| !got.contains_key(r))
                .collect();
            missing.sort_by_key(|r| r.refno());
            anyhow::bail!(
                "geom_input_cache LOOP 输入不完整: request={}, hit={}, missing={}, sample=[{}]",
                loop_refnos.len(),
                got.len(),
                missing.len(),
                sample_refnos(&missing, SAMPLE_LIMIT)
            );
        }
    }

    if !prim_refnos.is_empty() {
        let got = load_prim_inputs_for_refnos_from_global(prim_refnos)?;
        if got.len() != prim_refnos.len() {
            let mut missing: Vec<RefnoEnum> = prim_refnos
                .iter()
                .copied()
                .filter(|r| !got.contains_key(r))
                .collect();
            missing.sort_by_key(|r| r.refno());
            anyhow::bail!(
                "geom_input_cache PRIM 输入不完整: request={}, hit={}, missing={}, sample=[{}]",
                prim_refnos.len(),
                got.len(),
                missing.len(),
                sample_refnos(&missing, SAMPLE_LIMIT)
            );
        }
    }

    if !cate_refnos.is_empty() {
        let got = load_cate_inputs_for_refnos_from_global(cate_refnos)?;
        if got.len() != cate_refnos.len() {
            let mut missing: Vec<RefnoEnum> = cate_refnos
                .iter()
                .copied()
                .filter(|r| !got.contains_key(r))
                .collect();
            missing.sort_by_key(|r| r.refno());
            anyhow::bail!(
                "geom_input_cache CATE 输入不完整: request={}, hit={}, missing={}, sample=[{}]",
                cate_refnos.len(),
                got.len(),
                missing.len(),
                sample_refnos(&missing, SAMPLE_LIMIT)
            );
        }
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// 从全局缓存加载（同步）
// ---------------------------------------------------------------------------

/// 从全局 geom_input_cache 加载指定 dbnum 的所有 LOOP 输入。
pub fn load_loop_inputs_from_global(dbnum: u32) -> HashMap<RefnoEnum, LoopInput> {
    match global_geom_input_cache() {
        Some(cache) => cache.get_all_loop_inputs(dbnum),
        None => HashMap::new(),
    }
}

/// 从全局 geom_input_cache 加载指定 dbnum 的所有 PRIM 输入。
pub fn load_prim_inputs_from_global(dbnum: u32) -> HashMap<RefnoEnum, PrimInput> {
    match global_geom_input_cache() {
        Some(cache) => cache.get_all_prim_inputs(dbnum),
        None => HashMap::new(),
    }
}

/// 从全局 geom_input_cache 加载指定 dbnum 的所有 CATE 输入。
pub fn load_cate_inputs_from_global(dbnum: u32) -> HashMap<RefnoEnum, CateInput> {
    match global_geom_input_cache() {
        Some(cache) => cache.get_all_cate_inputs(dbnum),
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

/// 按 refno 集合加载 LOOP 输入（逐 refno 精确读取，不扫描全库）。
pub fn load_loop_inputs_for_refnos_from_global(
    refnos: &[RefnoEnum],
) -> anyhow::Result<HashMap<RefnoEnum, LoopInput>> {
    if refnos.is_empty() {
        return Ok(HashMap::new());
    }
    let cache = global_geom_input_cache()
        .ok_or_else(|| anyhow::anyhow!("geom_input_cache 未初始化"))?;
    let groups = group_refnos_by_dbnum_strict(refnos)?;

    let mut result = HashMap::with_capacity(refnos.len());
    for (dbnum, refs) in groups {
        for refno in refs {
            if let Some(input) = cache.get_loop_input(dbnum, refno) {
                result.insert(refno, input);
            }
        }
    }
    Ok(result)
}

/// 按 refno 集合加载 PRIM 输入（逐 refno 精确读取，不扫描全库）。
pub fn load_prim_inputs_for_refnos_from_global(
    refnos: &[RefnoEnum],
) -> anyhow::Result<HashMap<RefnoEnum, PrimInput>> {
    if refnos.is_empty() {
        return Ok(HashMap::new());
    }
    let cache = global_geom_input_cache()
        .ok_or_else(|| anyhow::anyhow!("geom_input_cache 未初始化"))?;
    let groups = group_refnos_by_dbnum_strict(refnos)?;

    let mut result = HashMap::with_capacity(refnos.len());
    for (dbnum, refs) in groups {
        for refno in refs {
            if let Some(input) = cache.get_prim_input(dbnum, refno) {
                result.insert(refno, input);
            }
        }
    }
    Ok(result)
}

/// 按 refno 集合加载 CATE 输入（逐 refno 精确读取，不扫描全库）。
pub fn load_cate_inputs_for_refnos_from_global(
    refnos: &[RefnoEnum],
) -> anyhow::Result<HashMap<RefnoEnum, CateInput>> {
    if refnos.is_empty() {
        return Ok(HashMap::new());
    }
    let cache = global_geom_input_cache()
        .ok_or_else(|| anyhow::anyhow!("geom_input_cache 未初始化"))?;
    let groups = group_refnos_by_dbnum_strict(refnos)?;

    let mut result = HashMap::with_capacity(refnos.len());
    for (dbnum, refs) in groups {
        for refno in refs {
            if let Some(input) = cache.get_cate_input(dbnum, refno) {
                result.insert(refno, input);
            }
        }
    }
    Ok(result)
}

/// 从全局 geom_input_cache 加载所有 dbnum 的 LOOP 输入。
pub fn load_all_loop_inputs_from_global() -> HashMap<RefnoEnum, LoopInput> {
    let Some(cache) = global_geom_input_cache() else {
        return HashMap::new();
    };
    let mut result = HashMap::new();
    for dbnum in cache.list_dbnums() {
        result.extend(cache.get_all_loop_inputs(dbnum));
    }
    result
}

/// 从全局 geom_input_cache 加载所有 dbnum 的 PRIM 输入。
pub fn load_all_prim_inputs_from_global() -> HashMap<RefnoEnum, PrimInput> {
    let Some(cache) = global_geom_input_cache() else {
        return HashMap::new();
    };
    let mut result = HashMap::new();
    for dbnum in cache.list_dbnums() {
        result.extend(cache.get_all_prim_inputs(dbnum));
    }
    result
}

/// 从全局 geom_input_cache 加载所有 dbnum 的 CATE 输入。
pub fn load_all_cate_inputs_from_global() -> HashMap<RefnoEnum, CateInput> {
    let Some(cache) = global_geom_input_cache() else {
        return HashMap::new();
    };
    let mut result = HashMap::new();
    for dbnum in cache.list_dbnums() {
        result.extend(cache.get_all_cate_inputs(dbnum));
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_geom_input_cache_cate_roundtrip() {
        let mgr = GeomInputCacheManager::new();
        let refno: RefnoEnum = "24381/36716".into();

        let cate_input = CateInput {
            refno,
            attmap: aios_core::NamedAttrMap::default(),
            world_transform: aios_core::Transform::IDENTITY,
            owner_refno: RefnoEnum::default(),
            owner_type: String::new(),
            visible: true,
        };

        mgr.insert_cate_input(1112, refno, &cate_input);

        let got = mgr.get_cate_input(1112, refno).expect("cate input must exist");
        assert_eq!(got.refno, refno);
        assert!(got.visible);

        let all = mgr.get_all_cate_inputs(1112);
        assert!(all.contains_key(&refno));
    }

    #[test]
    fn test_geom_input_cache_pin_release_two_tasks_same_refno() {
        let mgr = GeomInputCacheManager::new();
        let refno: RefnoEnum = "24381/36716".into();
        let dbnum = 1112u32;

        let loop_input = LoopInput {
            refno: refno.clone(),
            attmap: aios_core::NamedAttrMap::default(),
            world_transform: aios_core::Transform::IDENTITY,
            loops: Vec::new(),
            height: 0.0,
            owner_refno: RefnoEnum::default(),
            owner_type: String::new(),
            visible: true,
            neg_refnos: Vec::new(),
            cmpf_neg_refnos: Vec::new(),
        };
        let prim_input = PrimInput {
            refno: refno.clone(),
            attmap: aios_core::NamedAttrMap::default(),
            world_transform: aios_core::Transform::IDENTITY,
            owner_refno: RefnoEnum::default(),
            owner_type: String::new(),
            visible: true,
            neg_refnos: Vec::new(),
            poly_extra: None,
        };
        let cate_input = CateInput {
            refno: refno.clone(),
            attmap: aios_core::NamedAttrMap::default(),
            world_transform: aios_core::Transform::IDENTITY,
            owner_refno: RefnoEnum::default(),
            owner_type: String::new(),
            visible: true,
        };

        mgr.insert_loop_input(dbnum, refno.clone(), &loop_input);
        mgr.insert_prim_input(dbnum, refno.clone(), &prim_input);
        mgr.insert_cate_input(dbnum, refno.clone(), &cate_input);

        let keys = vec![(dbnum, refno.clone())];
        assert_eq!(mgr.pin_keys(&keys), 1);
        assert_eq!(mgr.pin_keys(&keys), 1);

        let removed_first = mgr.release_keys_and_clear(&keys);
        assert_eq!(removed_first, 0, "仍有并发租约时不应清理缓存");
        assert!(mgr.get_loop_input(dbnum, refno.clone()).is_some());
        assert!(mgr.get_prim_input(dbnum, refno.clone()).is_some());
        assert!(mgr.get_cate_input(dbnum, refno.clone()).is_some());

        let removed_second = mgr.release_keys_and_clear(&keys);
        assert_eq!(removed_second, 3, "最后一个租约释放后应清理全部同 key 条目");
        assert!(mgr.get_loop_input(dbnum, refno.clone()).is_none());
        assert!(mgr.get_prim_input(dbnum, refno.clone()).is_none());
        assert!(mgr.get_cate_input(dbnum, refno).is_none());
    }
}
