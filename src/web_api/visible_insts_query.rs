//! Task 2.1（`docs/plans/2026-07-30-model-load-performance-optimization.md` §5 阶段 2）：
//! `visible-insts` 的层级遍历改走 `pe_owner` 子树图查询。
//!
//! 旧路径 [`crate::fast_model::query_compat::query_deep_visible_inst_refnos`] 第一件事是
//! `get_or_load_pe_snapshot(dbnum)`，按 dbnum 把整张 `pe` 表装进内存建树（方案 P0）：
//! 点一个 EQUI 和点整个 SITE 的代价完全一样。本模块用 `PeOwnerTreeStore` 重写等价逻辑，
//! 成本只随**被点的那棵子树**走。
//!
//! **离线生成管线不动**：`query_compat` 及其快照仍服务批量生成（那个场景确实要遍历全库，
//! 一次装载摊到整轮生成上划算），本模块只接管交互式 web 接口。
//!
//! 语义等价性是硬约束（方案 §4.6.1）。四条分支与旧实现逐条对齐，差异点均在下方注释标出。

use std::collections::HashSet;

use aios_core::RefnoEnum;
use aios_core::pdms_types::VISBILE_GEO_NOUNS;
use aios_core::tool::db_tool::db1_hash;
use once_cell::sync::Lazy;

use crate::versioned_db::pe_owner_tree::PeOwnerTreeStore;

const BRAN_HANG_NOUNS: [&str; 2] = ["BRAN", "HANG"];

static BRAN_HASH: Lazy<u32> = Lazy::new(|| db1_hash("BRAN"));
static HANG_HASH: Lazy<u32> = Lazy::new(|| db1_hash("HANG"));
static VISIBLE_GEO_NOUN_HASHES: Lazy<HashSet<u32>> =
    Lazy::new(|| VISBILE_GEO_NOUNS.iter().map(|&n| db1_hash(n)).collect());

/// 一趟 BFS 同时收「可见几何」与「BRAN/HANG」两类目标。
///
/// 旧实现把子树走两遍（`query_visible_geo_descendants` + `query_descendants_bfs`），
/// 在快照上是纯内存遍历所以无所谓；打到 DB 上就是两倍往返，故合并成一次。
static TARGET_NOUNS: Lazy<Vec<&'static str>> = Lazy::new(|| {
    let mut nouns = VISBILE_GEO_NOUNS.to_vec();
    nouns.extend_from_slice(&BRAN_HANG_NOUNS);
    nouns
});

fn is_bran_hang(noun_hash: u32) -> bool {
    noun_hash == *BRAN_HASH || noun_hash == *HANG_HASH
}

fn sort_dedup(mut refnos: Vec<RefnoEnum>) -> Vec<RefnoEnum> {
    refnos.sort();
    refnos.dedup();
    refnos
}

/// 深度可见实例查询（交互路径）。
///
/// 与 `query_compat::query_deep_visible_inst_refnos` 返回**同一个集合**，但只遍历被点子树。
/// 输出恒经 `sort_dedup`，所以同胞顺序不参与结果 —— 两条路径的顺序差异不构成等价性风险。
pub async fn query_deep_visible_inst_refnos(refno: RefnoEnum) -> anyhow::Result<Vec<RefnoEnum>> {
    let out = query_subtree(refno).await?;
    if diff_enabled() {
        log_diff_against_snapshot(refno, &out).await;
    }
    Ok(out)
}

/// 对拍开关：`AIOS_VISIBLE_INSTS_DIFF=1` 时每次查询额外跑一遍旧快照路径比对集合，
/// **只记日志不改返回值**。默认关闭（旧路径要装全库快照，开着就等于没优化）。
///
/// 用途是方案 §4.6.1 的语义等价性验证：拿真实流量对拍，比拿一批人造 refno 跑单测更有说服力。
fn diff_enabled() -> bool {
    matches!(
        std::env::var("AIOS_VISIBLE_INSTS_DIFF")
            .ok()
            .as_deref()
            .map(str::trim),
        Some("1") | Some("on") | Some("true")
    )
}

/// 差异样本的打印上限，避免一次不一致刷爆日志。
const DIFF_SAMPLE: usize = 20;

async fn log_diff_against_snapshot(refno: RefnoEnum, fresh: &[RefnoEnum]) {
    let legacy = match crate::fast_model::query_compat::query_deep_visible_inst_refnos(refno).await
    {
        Ok(v) => sort_dedup(v),
        Err(err) => {
            log::warn!("[visible_insts_diff] refno={refno} 旧路径查询失败，无法对拍: {err}");
            return;
        }
    };
    if legacy == fresh {
        log::info!(
            "[visible_insts_diff] refno={refno} 一致 count={}",
            legacy.len()
        );
        return;
    }

    let fresh_set: HashSet<RefnoEnum> = fresh.iter().copied().collect();
    let legacy_set: HashSet<RefnoEnum> = legacy.iter().copied().collect();
    let only_fresh: Vec<String> = fresh
        .iter()
        .filter(|r| !legacy_set.contains(r))
        .take(DIFF_SAMPLE)
        .map(|r| r.to_string())
        .collect();
    let only_legacy: Vec<String> = legacy
        .iter()
        .filter(|r| !fresh_set.contains(r))
        .take(DIFF_SAMPLE)
        .map(|r| r.to_string())
        .collect();
    log::warn!(
        "[visible_insts_diff] refno={refno} 集合不一致: 子树图查询={} 快照={} \
         仅图查询有(前{}个)={:?} 仅快照有(前{}个)={:?}",
        fresh.len(),
        legacy.len(),
        DIFF_SAMPLE,
        only_fresh,
        DIFF_SAMPLE,
        only_legacy
    );
}

async fn query_subtree(refno: RefnoEnum) -> anyhow::Result<Vec<RefnoEnum>> {
    let Some(meta) = PeOwnerTreeStore::get_node_meta(refno).await? else {
        return Ok(Vec::new());
    };

    // owner 缺失归一成自身，对齐快照两条装载路径（扫表 `row.owner…unwrap_or(refno)`、
    // 种子 `if owner.0 == 0 { refno }`）。归一后 owner == refno 时元信息已在手，省一次点查。
    let owner = meta.owner.unwrap_or(refno);
    let owner_noun_hash = if owner == refno {
        Some(meta.noun_hash)
    } else {
        PeOwnerTreeStore::get_node_meta(owner)
            .await?
            .map(|m| m.noun_hash)
    };

    // 分支 1：owner 是 BRAN/HANG —— 自身即最小加载单元。
    if matches!(owner_noun_hash, Some(h) if is_bran_hang(h)) {
        return Ok(vec![refno]);
    }

    // 分支 2：自身是 BRAN/HANG —— 返回直接子节点。旧实现此处**不含自身**，保持一致。
    if is_bran_hang(meta.noun_hash) {
        return Ok(sort_dedup(PeOwnerTreeStore::query_children(refno).await?));
    }

    // 分支 3：可见几何子孙 + BRAN/HANG 子孙及其直接子节点（两者都含自身判定）。
    //
    // 走 `collect_target_refnos_grouped(prune=false)` 而不是 `query_descendants_filtered`：
    // 后者用单条递归 idiom，在"部分节点有 pe_owner 边、部分没有"的混合态下会在缺边处
    // **静默截断子树**（见 `pe_owner_tree` 模块头 D5）；前者逐层 BFS + `children_batch`，
    // 每个节点都只查询 `pe_owner`。代价是往返数约为深度，
    // 相对全库扫表可以忽略 —— 交互路径上漏元素比慢严重得多。
    let grouped =
        PeOwnerTreeStore::collect_target_refnos_grouped(&[refno], TARGET_NOUNS.as_slice(), false)
            .await?;

    let mut out: Vec<RefnoEnum> = Vec::new();
    let mut bran_hang_roots: Vec<RefnoEnum> = Vec::new();
    for (noun_hash, refnos) in grouped {
        if is_bran_hang(noun_hash) {
            bran_hang_roots.extend(refnos.iter().copied());
            out.extend(refnos);
        } else if VISIBLE_GEO_NOUN_HASHES.contains(&noun_hash) {
            out.extend(refnos);
        }
    }

    if !bran_hang_roots.is_empty() {
        // 旧实现逐个 root 串行取 children（N 次往返），这里一次批量（chunk 500）。
        for children in PeOwnerTreeStore::children_batch(&bran_hang_roots)
            .await?
            .into_values()
        {
            out.extend(children);
        }
    }

    Ok(sort_dedup(out))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bran_hang_hash_classification() {
        assert!(is_bran_hang(db1_hash("BRAN")));
        assert!(is_bran_hang(db1_hash("HANG")));
        assert!(!is_bran_hang(db1_hash("EQUI")));
    }

    /// 分支 3 靠 hash 把一趟 BFS 的结果拆成两类；两类若有交集，拆分口径就不成立。
    #[test]
    fn geo_and_bran_hang_families_are_disjoint() {
        assert!(
            !VISBILE_GEO_NOUNS.iter().any(|&n| is_bran_hang(db1_hash(n))),
            "可见几何 noun 表与 BRAN/HANG 出现重叠"
        );
    }

    /// 合并后的目标 noun 集必须同时覆盖两类，否则分支 3 会漏收一整类。
    #[test]
    fn target_nouns_cover_both_families() {
        let hashes: HashSet<u32> = TARGET_NOUNS.iter().map(|&n| db1_hash(n)).collect();
        assert!(hashes.contains(&db1_hash("BRAN")));
        assert!(hashes.contains(&db1_hash("HANG")));
        for &noun in VISBILE_GEO_NOUNS.iter() {
            assert!(
                hashes.contains(&db1_hash(noun)),
                "可见几何 noun {noun} 未进入 TARGET_NOUNS"
            );
        }
        assert_eq!(TARGET_NOUNS.len(), VISBILE_GEO_NOUNS.len() + 2);
    }

    /// 分组回填时靠 hash 区分两类，`VISIBLE_GEO_NOUN_HASHES` 必须与常量表同步。
    #[test]
    fn visible_geo_hashes_match_constant_table() {
        assert_eq!(VISIBLE_GEO_NOUN_HASHES.len(), {
            let distinct: HashSet<u32> = VISBILE_GEO_NOUNS.iter().map(|&n| db1_hash(n)).collect();
            distinct.len()
        });
    }

    #[test]
    fn sort_dedup_is_idempotent_set_semantics() {
        let a = RefnoEnum::from(aios_core::RefU64(20));
        let b = RefnoEnum::from(aios_core::RefU64(10));
        let out = sort_dedup(vec![a, b, a, b]);
        assert_eq!(out.len(), 2);
        assert!(out.windows(2).all(|w| w[0] <= w[1]));
        assert_eq!(sort_dedup(out.clone()), out);
    }
}
