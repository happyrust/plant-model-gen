// 实用工具函数
//
// 从旧 gen_model.rs 迁移的辅助函数

use super::tree_index_manager::TreeIndexManager;
use crate::fast_model::resolve_desi_comp;
// specs/023 M2：cata_hash 分组主路径切 pe_owner 快照（pe.cata_hash 字段）；tree 回退
use crate::versioned_db::pe_owner_snapshot::get_or_load_pe_snapshot;
use crate::versioned_db::pe_owner_tree::latest_tree_source_is_pe_owner;
use aios_core::parsed_data::geo_params_data::CateGeoParam::{BoxImplied, TubeImplied};
use aios_core::pdms_types::{BRAN_COMPONENT_NOUN_NAMES, CataHashRefnoKV, USE_CATE_NOUN_NAMES};
use aios_core::prim_geo::tubing::TubiSize;
use aios_core::tool::db_tool::db1_hash;
use aios_core::tree_query::{TreeIndex, TreeQuery, TreeQueryFilter};
use aios_core::{RefU64, RefnoEnum};
use anyhow::Result;
use dashmap::DashMap;
use once_cell::sync::Lazy;
use std::collections::{HashMap, HashSet};

/// 检查是否启用 E3D 调试模式
#[allow(dead_code)]
pub fn is_e3d_debug_enabled() -> bool {
    #[cfg(feature = "debug_e3d")]
    {
        false // TODO: 需要从原来的 E3D_DEBUG_ENABLED 获取
    }
    #[cfg(not(feature = "debug_e3d"))]
    {
        false
    }
}

/// 检查是否启用 E3D info 模式
#[allow(dead_code)]
pub fn is_e3d_info_enabled() -> bool {
    #[cfg(feature = "debug_e3d")]
    {
        false // TODO: 需要从原来的 E3D_INFO_ENABLED 获取
    }
    #[cfg(not(feature = "debug_e3d"))]
    {
        false
    }
}

/// 检查是否启用 E3D trace 模式
#[allow(dead_code)]
pub fn is_e3d_trace_enabled() -> bool {
    #[cfg(feature = "debug_e3d")]
    {
        false // TODO: 需要从原来的 E3D_TRACE_ENABLED 获取
    }
    #[cfg(not(feature = "debug_e3d"))]
    {
        false
    }
}

/// 查询 Tubi 尺寸
///
/// 优先从 SCOM PARA 直接读取（廉价），仅在失败时回退到完整几何求解
pub async fn query_tubi_size(
    refno: RefnoEnum,
    tubi_cat_ref: RefnoEnum,
    is_hang: bool,
) -> Result<TubiSize> {
    // 快速路径：直接从 SCOM 的 PARA 读取管径（1 次 DB 查询）
    if let Ok(cat_att) = aios_core::get_named_attmap(tubi_cat_ref).await {
        let params = cat_att.get_f32_vec("PARA").unwrap_or_default();
        if params.len() >= 2 {
            let tubi_bore = params[if is_hang { 0 } else { 1 }] as f32;
            if tubi_bore > 0.0 {
                return Ok(TubiSize::BoreSize(tubi_bore));
            }
        }
    }

    // 慢速路径：完整几何求解（含表达式计算）
    let tubi_geoms_info = resolve_desi_comp(refno, Some(tubi_cat_ref), None)
        .await
        .unwrap_or_default();
    for geom in &tubi_geoms_info.geometries {
        if let BoxImplied(d) = geom {
            return Ok(TubiSize::BoxSize((d.height, d.width)));
        } else if let TubeImplied(d) = geom {
            return Ok(TubiSize::BoreSize(d.diameter));
        }
    }

    Ok(TubiSize::None)
}

static BRAN_HASH: Lazy<u32> = Lazy::new(|| db1_hash("BRAN"));
static HANG_HASH: Lazy<u32> = Lazy::new(|| db1_hash("HANG"));
static CATE_NOUN_HASHES: Lazy<HashSet<u32>> = Lazy::new(|| {
    USE_CATE_NOUN_NAMES
        .iter()
        .map(|noun| db1_hash(noun))
        .collect()
});

fn is_bran_or_hang(noun_hash: u32) -> bool {
    noun_hash == *BRAN_HASH || noun_hash == *HANG_HASH
}

fn is_cate_noun(noun_hash: u32) -> bool {
    CATE_NOUN_HASHES.contains(&noun_hash)
}

pub(crate) fn is_valid_cata_hash(cata_hash: &str) -> bool {
    if cata_hash.is_empty() || cata_hash == "0" {
        return false;
    }
    cata_hash.chars().all(|ch| ch.is_ascii_digit())
}

fn build_refno_cata_key(refno: &RefnoEnum) -> String {
    format!("refno_{}", refno.to_string().replace('/', "_"))
}

fn insert_cata_hash_refno_by_values(
    map: &DashMap<String, CataHashRefnoKV>,
    refno: RefnoEnum,
    noun_hash: u32,
    cata_hash: Option<u64>,
) {
    if is_bran_or_hang(noun_hash) {
        return;
    }
    let has_valid_hash = cata_hash.is_some_and(|hash| hash != 0);
    if !has_valid_hash && !is_cate_noun(noun_hash) {
        return;
    }
    let fallback_key = build_refno_cata_key(&refno);
    let key = cata_hash
        .filter(|&hash| hash != 0)
        .map(|hash| hash.to_string())
        .unwrap_or(fallback_key);
    let mut entry = map.entry(key.clone()).or_insert(CataHashRefnoKV {
        cata_hash: key,
        group_refnos: Vec::new(),
        exist_inst: false,
        ptset: None,
    });
    entry.group_refnos.push(refno);
}

fn insert_cata_hash_refno(
    map: &DashMap<String, CataHashRefnoKV>,
    meta: &aios_core::tree_query::TreeNodeMeta,
) {
    // 带有效 cata_hash 的元素必然是元件引用件（cal_cata_hash 基于 SPRE/CATR 计算成功），
    // 直接入组 —— 否则 BRAN 子管件（ELBO/VALV/OLET/ATTA 等，不在 USE_CATE_NOUN_NAMES
    // 白名单内）会被整体滤掉，导致 BRAN 管线 unique_cata=0、管件实例从不生成。
    // 无有效 hash 的退化路径（refno key）仍按 CATE noun 白名单收口，避免容器节点混入。
    let refno = RefnoEnum::from(meta.refno);
    insert_cata_hash_refno_by_values(map, refno, meta.noun, meta.cata_hash);
}

async fn build_cata_hash_map_from_tree_index(
    index: &TreeIndex,
    refnos: &[RefnoEnum],
) -> Result<DashMap<String, CataHashRefnoKV>> {
    let mut visited: HashSet<RefU64> = HashSet::new();
    let result_map: DashMap<String, CataHashRefnoKV> = DashMap::new();

    for refno in refnos {
        let root = refno.refno();
        if visited.insert(root) {
            if let Some(meta) = index.node_meta(root) {
                insert_cata_hash_refno(&result_map, &meta);
            }
        }
        let children = index
            .query_children(root, TreeQueryFilter::default())
            .await?;
        for child in children {
            if !visited.insert(child) {
                continue;
            }
            if let Some(meta) = index.node_meta(child) {
                insert_cata_hash_refno(&result_map, &meta);
            }
        }
    }

    Ok(result_map)
}

/// 可能持有 cata_hash 的 noun 集合（元件引用件 + BRAN 子管件）。
/// pe_owner 快照路径下，这些 noun 若 cata_hash 缺失（站点未回填），回退 attmap 计算并记 miss。
static CATA_HASH_BEARING_NOUN_HASHES: Lazy<HashSet<u32>> = Lazy::new(|| {
    USE_CATE_NOUN_NAMES
        .iter()
        .chain(BRAN_COMPONENT_NOUN_NAMES.iter())
        .map(|noun| db1_hash(noun))
        .collect()
});

/// 快照路径的单节点入组：pe.cata_hash 优先；元件类 noun 缺 hash 时回退 attmap
/// 计算（`model-version backfill-pe-cata-hash` 未跑的存量站点），并记 cache_miss_report。
async fn insert_cata_hash_refno_from_snapshot(
    map: &DashMap<String, CataHashRefnoKV>,
    refno: RefnoEnum,
    noun_hash: u32,
    cata_hash: Option<u64>,
) {
    let mut cata_hash = cata_hash;
    if cata_hash.is_none()
        && !is_bran_or_hang(noun_hash)
        && CATA_HASH_BEARING_NOUN_HASHES.contains(&noun_hash)
    {
        super::cache_miss_report::with_global_report(|report| {
            report.record_refno_miss(
                "build_cata_hash_map",
                "pe_cata_hash_missing",
                refno,
                Some("pe.cata_hash 缺失，回退 attmap 计算（建议 backfill-pe-cata-hash）"),
            );
        });
        cata_hash = aios_core::get_named_attmap(refno)
            .await
            .ok()
            .and_then(|att| att.cal_cata_hash());
    }
    insert_cata_hash_refno_by_values(map, refno, noun_hash, cata_hash);
}

/// 基于 pe_owner 快照（按 dbnum）构建 cata_hash 分组：roots 自身 + 直接子节点，
/// 与 tree 路径 `build_cata_hash_map_from_tree_index` 同构。
async fn build_cata_hash_map_from_snapshot_by_dbnum(
    dbnum: u32,
    refnos: &[RefnoEnum],
) -> Result<DashMap<String, CataHashRefnoKV>> {
    let snap = get_or_load_pe_snapshot(dbnum).await?;
    let mut visited: HashSet<RefU64> = HashSet::new();
    let result_map: DashMap<String, CataHashRefnoKV> = DashMap::new();

    for refno in refnos {
        let root = refno.refno();
        if visited.insert(root) {
            if let Some(meta) = snap.node_meta(root) {
                insert_cata_hash_refno_from_snapshot(
                    &result_map,
                    RefnoEnum::from(meta.refno),
                    meta.noun,
                    meta.cata_hash,
                )
                .await;
            }
        }
        for child in snap.collect_children(root, &TreeQueryFilter::default()) {
            if !visited.insert(child) {
                continue;
            }
            if let Some(meta) = snap.node_meta(child) {
                insert_cata_hash_refno_from_snapshot(
                    &result_map,
                    RefnoEnum::from(meta.refno),
                    meta.noun,
                    meta.cata_hash,
                )
                .await;
            }
        }
    }

    Ok(result_map)
}

/// 基于 tree 文件（按 dbnum）构建 cata_hash 分组
pub async fn build_cata_hash_map_from_tree_by_dbnum(
    dbnum: u32,
    refnos: &[RefnoEnum],
) -> Result<DashMap<String, CataHashRefnoKV>> {
    if refnos.is_empty() {
        return Ok(DashMap::new());
    }
    // specs/023 M2 双源：pe_owner（默认）→ pe 快照；AIOS_TREE_QUERY_SOURCE=tree → .tree
    if latest_tree_source_is_pe_owner() {
        return build_cata_hash_map_from_snapshot_by_dbnum(dbnum, refnos).await;
    }
    let manager = TreeIndexManager::with_default_dir(vec![dbnum]);
    let index = manager.load_index(dbnum)?;
    build_cata_hash_map_from_tree_index(&index, refnos).await
}

async fn build_cata_hash_map_from_db(
    refnos: &[RefnoEnum],
) -> Result<DashMap<String, CataHashRefnoKV>> {
    let result_map: DashMap<String, CataHashRefnoKV> = DashMap::new();
    for &refno in refnos {
        let att = match aios_core::get_named_attmap(refno).await {
            Ok(att) => att,
            Err(e) => {
                eprintln!(
                    "[build_cata_hash_map][db_fallback] get_named_attmap 失败: refno={} err={}",
                    refno, e
                );
                continue;
            }
        };
        let noun_hash = db1_hash(att.get_type_str());
        insert_cata_hash_refno_by_values(&result_map, refno, noun_hash, att.cal_cata_hash());
    }
    Ok(result_map)
}

/// 基于 tree 文件（自动按 dbnum 分组）构建 cata_hash 分组
pub async fn build_cata_hash_map_from_tree(
    refnos: &[RefnoEnum],
) -> Result<DashMap<String, CataHashRefnoKV>> {
    if refnos.is_empty() {
        return Ok(DashMap::new());
    }
    // 关键：RefnoEnum 的 ref0（例如 17496）并不等同于 dbnum（例如 1112）。
    // Full Noun 模式下若未提前加载 db_meta_info.json，直接用 ref0 当 dbnum 会导致找不到 tree 文件，
    // 进而整批 refno 被跳过，最终 target_cata_map 为空。
    //
    // 因此这里优先用本仓的 db_meta_manager 做 refno->dbnum 映射，并尽力 ensure_loaded。
    // 若仍无法映射：直接报错（禁止回退用 ref0 当 dbnum），以免悄然跳过整批 refno。
    let db_meta = crate::data_interface::db_meta_manager::db_meta();
    let _ = db_meta.ensure_loaded();

    let mut dbnum_groups: HashMap<u32, Vec<RefnoEnum>> = HashMap::new();
    let mut missing_ref0s: std::collections::HashSet<u32> = std::collections::HashSet::new();
    for refno in refnos {
        let dbnum_opt = db_meta
            .get_dbnum_by_refno(*refno)
            .or_else(|| crate::fast_model::db_meta_cache::get_dbnum_for_refno(*refno));

        if let Some(dbnum) = dbnum_opt {
            dbnum_groups.entry(dbnum).or_default().push(*refno);
        } else {
            missing_ref0s.insert(refno.refno().get_0());
        }
    }

    if !missing_ref0s.is_empty() {
        anyhow::bail!(
            "缺少 ref0->dbnum 映射（ref0s={:?}）。请先生成/更新 output/<project>/scene_tree/db_meta_info.json 的 ref0_to_dbnum。",
            missing_ref0s
        );
    }

    let merged_map: DashMap<String, CataHashRefnoKV> = DashMap::new();
    for (dbnum, group_refnos) in dbnum_groups {
        let map = match build_cata_hash_map_from_tree_by_dbnum(dbnum, &group_refnos).await {
            Ok(map) => map,
            Err(e) => {
                eprintln!(
                    "[build_cata_hash_map] dbnum={} 加载 tree/构建 cata_hash 失败，回退 DB 构建（{} 个 refno）: {}",
                    dbnum,
                    group_refnos.len(),
                    e
                );
                build_cata_hash_map_from_db(&group_refnos).await?
            }
        };
        for entry in map.into_iter() {
            let (cata_hash, kv) = entry;
            if let Some(mut existing) = merged_map.get_mut(&cata_hash) {
                existing.group_refnos.extend(kv.group_refnos);
            } else {
                merged_map.insert(cata_hash, kv);
            }
        }
    }

    Ok(merged_map)
}

#[cfg(test)]
mod tests {
    use super::*;
    use aios_core::RefU64;
    use aios_core::tool::db_tool::db1_hash;
    use aios_core::tree_query::{TreeFile, TreeIndex, TreeNodeMeta};
    use indextree::Arena;

    #[tokio::test]
    async fn test_query_tubi_size_none() {
        // RefnoEnum 没有 RefU64 变体，这里用 RefU64 -> RefnoEnum 的通用转换构造一个不存在的 refno，
        // 期望查询失败时能兜底返回 TubiSize::None。
        let dummy = RefnoEnum::from(RefU64::from_two_nums(999999, 0));
        let result = query_tubi_size(dummy, dummy, false).await;

        assert!(result.is_ok());
        if let Ok(size) = result {
            assert!(matches!(size, TubiSize::None));
        }
    }

    #[tokio::test]
    async fn test_build_cata_hash_map_from_tree_index() {
        let mut arena = Arena::new();
        let root_refno = RefU64::from_two_nums(1, 0);
        let root_id = arena.new_node(TreeNodeMeta {
            refno: root_refno,
            owner: root_refno,
            noun: db1_hash("SITE"),
            cata_hash: None,
        });
        let child_refno = RefU64::from_two_nums(1, 1);
        let child_id = arena.new_node(TreeNodeMeta {
            refno: child_refno,
            owner: root_refno,
            noun: db1_hash("EQUI"),
            cata_hash: Some(123456),
        });
        root_id.append(child_id, &mut arena);

        let tree = TreeFile {
            dbnum: 1,
            root_refno,
            arena,
        };
        let index = TreeIndex::from_tree_file(tree);

        let refnos = vec![RefnoEnum::from(root_refno)];
        let map = build_cata_hash_map_from_tree_index(&index, &refnos)
            .await
            .expect("build cata map");
        let entry = map.get("123456").expect("missing 123456");
        assert_eq!(entry.group_refnos.len(), 1);
        assert_eq!(entry.group_refnos[0], RefnoEnum::from(child_refno));
    }
}
