// 实用工具函数
//
// 从旧 gen_model.rs 迁移的辅助函数

use crate::fast_model::resolve_desi_comp;
use super::tree_index_manager::TreeIndexManager;
use aios_core::pdms_types::CataHashRefnoKV;
use aios_core::parsed_data::geo_params_data::CateGeoParam::{BoxImplied, TubeImplied};
use aios_core::prim_geo::tubing::TubiSize;
use aios_core::tool::db_tool::db1_hash;
use aios_core::tree_query::{TreeIndex, TreeQuery, TreeQueryFilter};
use aios_core::{RefnoEnum, RefU64};
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
/// 从旧 gen_model.rs 迁移，用于 cata_model
pub async fn query_tubi_size(
    refno: RefnoEnum,
    tubi_cat_ref: RefnoEnum,
    is_hang: bool,
) -> Result<TubiSize> {
    let tubi_geoms_info = resolve_desi_comp(refno, Some(tubi_cat_ref))
        .await
        .unwrap_or_default();

    // 从几何参数查询尺寸
    for geom in &tubi_geoms_info.geometries {
        if let BoxImplied(d) = geom {
            return Ok(TubiSize::BoxSize((d.height, d.width)));
        } else if let TubeImplied(d) = geom {
            return Ok(TubiSize::BoreSize(d.diameter));
        }
    }

    // 从属性映射查询
    if let Ok(cat_att) = aios_core::get_named_attmap(tubi_cat_ref).await {
        let params = cat_att.get_f32_vec("PARA").unwrap_or_default();
        if params.len() >= 2 {
            let tubi_bore = params[if is_hang { 0 } else { 1 }] as f32;
            return Ok(TubiSize::BoreSize(tubi_bore));
        }
    }


    Ok(TubiSize::None)
}

static BRAN_HASH: Lazy<u32> = Lazy::new(|| db1_hash("BRAN"));
static HANG_HASH: Lazy<u32> = Lazy::new(|| db1_hash("HANG"));

fn is_bran_or_hang(noun_hash: u32) -> bool {
    noun_hash == *BRAN_HASH || noun_hash == *HANG_HASH
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

fn insert_cata_hash_refno(
    map: &DashMap<String, CataHashRefnoKV>,
    meta: &aios_core::tree_query::TreeNodeMeta,
) {
    if is_bran_or_hang(meta.noun) {
        return;
    }
    let refno = RefnoEnum::from(meta.refno);
    let fallback_key = build_refno_cata_key(&refno);
    let key = meta
        .cata_hash
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
        let children = index.query_children(root, TreeQueryFilter::default()).await?;
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

/// 基于 tree 文件（按 dbnum）构建 cata_hash 分组
pub async fn build_cata_hash_map_from_tree_by_dbnum(
    dbnum: u32,
    refnos: &[RefnoEnum],
) -> Result<DashMap<String, CataHashRefnoKV>> {
    if refnos.is_empty() {
        return Ok(DashMap::new());
    }
    let manager = TreeIndexManager::with_default_dir(vec![dbnum]);
    let index = manager.load_index(dbnum)?;
    build_cata_hash_map_from_tree_index(&index, refnos).await
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
    let db_meta = crate::data_interface::db_meta_manager::db_meta();
    let _ = db_meta.ensure_loaded();

    let mut dbnum_groups: HashMap<u32, Vec<RefnoEnum>> = HashMap::new();
    for refno in refnos {
        let dbnum = db_meta
            .get_dbnum_by_refno(*refno)
            .or_else(|| crate::fast_model::db_meta_cache::get_dbnum_for_refno(*refno))
            .unwrap_or_else(|| refno.refno().get_0());
        dbnum_groups.entry(dbnum).or_default().push(*refno);
    }

    let merged_map: DashMap<String, CataHashRefnoKV> = DashMap::new();
    for (dbnum, group_refnos) in dbnum_groups {
        let Ok(map) = build_cata_hash_map_from_tree_by_dbnum(dbnum, &group_refnos).await else {
            continue;
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
    use aios_core::tool::db_tool::db1_hash;
    use aios_core::tree_query::{TreeFile, TreeIndex, TreeNodeMeta};
    use aios_core::RefU64;
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
