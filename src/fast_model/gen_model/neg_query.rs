//! pe_owner 批量查询辅助：按 dbnum 分组、复用快照，返回 root -> descendants 映射。
//!
//! 背景：输入缓存 batch 路径中若对每个 refno 单独调用层级查询，会引入大量重复固定开销
//!（重复推导 dbnum、重复构造过滤器、过量 task 调度等）。本模块将其收敛为“每 dbnum 一次加载”。

use std::collections::{HashMap, HashSet};
use std::path::Path;

use aios_core::RefnoEnum;
use aios_core::tool::db_tool::db1_hash;
use aios_core::tree_query::{TreeQueryFilter, TreeQueryOptions};

use crate::data_interface::db_meta;

pub fn group_by_dbnum<F>(
    refnos: &[RefnoEnum],
    mut resolver: F,
) -> anyhow::Result<HashMap<u32, Vec<RefnoEnum>>>
where
    F: FnMut(RefnoEnum) -> anyhow::Result<u32>,
{
    let mut out: HashMap<u32, Vec<RefnoEnum>> = HashMap::new();
    for &r in refnos {
        let dbnum = resolver(r)?;
        out.entry(dbnum).or_default().push(r);
    }
    Ok(out)
}

/// 按 dbnum 分组，但不会因单个元素解析失败而整体失败。
///
/// 返回：`(grouped, missing)`：
/// - `grouped`: dbnum -> roots
/// - `missing`: 无法解析 dbnum 的 roots（由调用侧决定如何处理：报错/日志/默认空）
pub fn group_by_dbnum_best_effort<F>(
    refnos: &[RefnoEnum],
    mut resolver: F,
) -> (HashMap<u32, Vec<RefnoEnum>>, Vec<RefnoEnum>)
where
    F: FnMut(RefnoEnum) -> Option<u32>,
{
    let mut out: HashMap<u32, Vec<RefnoEnum>> = HashMap::new();
    let mut missing: Vec<RefnoEnum> = Vec::new();
    for &r in refnos {
        match resolver(r) {
            Some(dbnum) => out.entry(dbnum).or_default().push(r),
            None => missing.push(r),
        }
    }
    (out, missing)
}

/// 按 dbnum 分组查询子孙集合（pe_owner 快照），返回 root -> descendants 映射。
///
/// - `_tree_dir`：保留参数以兼容旧调用签名（不再读取 `.tree`）。
/// - `roots`：起点节点列表。
/// - `nouns`：noun 过滤列表；空表示不按 noun 过滤。
/// - `include_self`：是否在结果中包含 root 本身（若其满足过滤条件）。
pub async fn query_descendants_map_by_dbnum(
    _tree_dir: impl AsRef<Path>,
    roots: &[RefnoEnum],
    nouns: &[&str],
    include_self: bool,
) -> anyhow::Result<HashMap<RefnoEnum, Vec<RefnoEnum>>> {
    use crate::versioned_db::pe_owner_snapshot::get_or_load_pe_snapshot;

    if roots.is_empty() {
        return Ok(HashMap::new());
    }

    let _ = db_meta().ensure_loaded();
    let (grouped, missing_dbnum) =
        group_by_dbnum_best_effort(roots, |r| db_meta().get_dbnum_by_refno(r));

    let noun_hashes: Option<HashSet<u32>> = if nouns.is_empty() {
        None
    } else {
        Some(nouns.iter().map(|n| db1_hash(n)).collect())
    };
    let options = TreeQueryOptions {
        include_self,
        max_depth: None,
        filter: TreeQueryFilter {
            noun_hashes,
            ..Default::default()
        },
        prune_on_match: false,
    };

    let mut out: HashMap<RefnoEnum, Vec<RefnoEnum>> = HashMap::new();
    for (dbnum, db_roots) in grouped {
        let snap = match get_or_load_pe_snapshot(dbnum).await {
            Ok(s) => s,
            Err(e) => {
                log::warn!(
                    "[neg_query] pe 快照加载失败，跳过 dbnum={} 的 {} 个 roots: {}",
                    dbnum,
                    db_roots.len(),
                    e
                );
                continue;
            }
        };
        for root in db_roots {
            if !snap.contains_refno(root.refno()) {
                continue;
            }
            let mut seen: HashSet<RefnoEnum> = HashSet::new();
            let mut rows: Vec<RefnoEnum> = Vec::new();
            for r in snap.collect_descendants_bfs(root.refno(), &options) {
                let r = RefnoEnum::from(r);
                if r.is_valid() && seen.insert(r) {
                    rows.push(r);
                }
            }
            out.insert(root, rows);
        }
    }

    for r in missing_dbnum {
        out.entry(r).or_default();
    }

    Ok(out)
}

/// 兼容旧 dual 名称。
pub async fn query_descendants_map_by_dbnum_dual(
    tree_dir: impl AsRef<Path>,
    roots: &[RefnoEnum],
    nouns: &[&str],
    include_self: bool,
) -> anyhow::Result<HashMap<RefnoEnum, Vec<RefnoEnum>>> {
    query_descendants_map_by_dbnum(tree_dir, roots, nouns, include_self).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_group_by_dbnum_keeps_roots() {
        let r1: RefnoEnum = "24381/1".into();
        let r2: RefnoEnum = "24381/2".into();
        let r3: RefnoEnum = "9304/3".into();

        let mut m: HashMap<RefnoEnum, u32> = HashMap::new();
        m.insert(r1, 1112);
        m.insert(r2, 1112);
        m.insert(r3, 7997);

        let grouped = group_by_dbnum(&[r1, r2, r3], |r| Ok(*m.get(&r).unwrap())).unwrap();
        assert_eq!(grouped.get(&1112).unwrap().len(), 2);
        assert_eq!(grouped.get(&7997).unwrap().len(), 1);
    }

    #[test]
    fn test_group_by_dbnum_best_effort_missing_does_not_drop_others() {
        let r1: RefnoEnum = "24381/1".into();
        let r2: RefnoEnum = "24381/2".into();
        let r3: RefnoEnum = "9304/3".into();

        let (grouped, missing) = group_by_dbnum_best_effort(&[r1, r2, r3], |r| match r {
            x if x == r1 => Some(1112),
            x if x == r2 => None,
            x if x == r3 => Some(7997),
            _ => None,
        });

        assert_eq!(grouped.get(&1112).unwrap(), &vec![r1]);
        assert_eq!(grouped.get(&7997).unwrap(), &vec![r3]);
        assert_eq!(missing, vec![r2]);
    }

    #[tokio::test]
    async fn test_query_descendants_map_empty() {
        let m = query_descendants_map_by_dbnum(
            std::path::PathBuf::from("output/does-not-matter"),
            &[],
            &["FOO"],
            false,
        )
        .await
        .unwrap();
        assert!(m.is_empty());
    }
}
