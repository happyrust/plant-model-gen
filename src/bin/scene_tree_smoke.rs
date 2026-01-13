use anyhow::{anyhow, Result};
use aios_core::{get_db_option, initialize_databases, RefU64, SUL_DB, SurrealQueryExt};
use aios_database::scene_tree;

/// Scene Tree smoke test
///
/// 用途：在已导入的 PE 数据上，按指定根节点构建子树，并验证 leaves(几何叶子) 结果满足“叶子=无子节点”。
///
/// 环境变量：
/// - `SCENE_TREE_FORCE_REBUILD`：`1/true` 时清空 `scene_node/contains` 后重建
/// - `SCENE_TREE_PE_TABLE`：用于选根的 PE 表名（默认 `pe_1112`）
/// - `SCENE_TREE_ROOT_NOUN`：用于选根的 noun（默认 `SITE`）
/// - `SCENE_TREE_ROOT_REFNO`：直接指定 root refno（如 `9304_0`），优先级高于 table+noun
#[tokio::main]
async fn main() -> Result<()> {
    let db_option = get_db_option();
    initialize_databases(&db_option).await?;

    let force_rebuild = std::env::var("SCENE_TREE_FORCE_REBUILD")
        .ok()
        .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
        .unwrap_or(false);

    let root_refno_str = std::env::var("SCENE_TREE_ROOT_REFNO").ok();
    let root_refno_u64 = match root_refno_str {
        Some(s) => parse_refno_to_u64(&s).ok_or_else(|| anyhow!("invalid SCENE_TREE_ROOT_REFNO: {s}"))?,
        None => {
            let pe_table = std::env::var("SCENE_TREE_PE_TABLE").unwrap_or_else(|_| "pe_1112".to_string());
            let root_noun = std::env::var("SCENE_TREE_ROOT_NOUN").unwrap_or_else(|_| "SITE".to_string());
            let sql = format!(
                "SELECT VALUE meta::id(id) FROM {pe_table} WHERE noun = '{root_noun}' LIMIT 1"
            );
            let ids: Vec<String> = SUL_DB.query_take(&sql, 0).await?;
            let Some(id) = ids.into_iter().next() else {
                return Err(anyhow!(
                    "cannot find root by noun: table={}, noun={}",
                    pe_table,
                    root_noun
                ));
            };
            println!("[smoke] picked root id from {}: {}", pe_table, id);
            parse_refno_to_u64(&id).ok_or_else(|| anyhow!("failed to parse picked root id: {id}"))?
        }
    };

    let root_refno = aios_core::RefnoEnum::from(RefU64(root_refno_u64));

    println!(
        "[smoke] init scene_tree from root={}, force_rebuild={}",
        root_refno, force_rebuild
    );
    let init_result = scene_tree::init_scene_tree_from_root(root_refno, force_rebuild).await?;
    println!(
        "[smoke] init ok: node_count={}, relation_count={}, duration_ms={}",
        init_result.node_count, init_result.relation_count, init_result.duration_ms
    );

    // 从当前子树里抽一个非叶子节点作为 leaves 查询根
    let sql_pick_root_id = "SELECT VALUE meta::id(id) FROM scene_node WHERE is_leaf = false LIMIT 1";
    let roots: Vec<i64> = SUL_DB.query_take(sql_pick_root_id, 0).await?;
    let Some(root_id) = roots.into_iter().next() else {
        return Err(anyhow!("[smoke] no non-leaf scene_node found (tree too small)"));
    };
    println!("[smoke] picked scene_node root_id={}", root_id);

    let leaves = scene_tree::query_ungenerated_leaves(root_id).await?;
    println!("[smoke] ungenerated geo leaf count={}", leaves.len());
    if leaves.is_empty() {
        println!("[smoke] no ungenerated geo leaves under picked root (skip leaf check)");
        return Ok(());
    }

    // 抽样验证前 10 个：必须无子节点
    for leaf_id in leaves.into_iter().take(10) {
        let sql_child_count = format!("SELECT VALUE count(->contains) FROM scene_node:{leaf_id}");
        let counts: Vec<i64> = SUL_DB.query_take(&sql_child_count, 0).await?;
        let child_count = counts.into_iter().next().unwrap_or(0);
        if child_count != 0 {
            return Err(anyhow!(
                "[smoke] leaf_id={} has children: count(->contains)={}",
                leaf_id,
                child_count
            ));
        }
        println!("[smoke] leaf_id={} ok (no children)", leaf_id);
    }

    println!("[smoke] done");
    Ok(())
}

fn parse_refno_to_u64(refno: &str) -> Option<u64> {
    let parts: Vec<&str> = refno.split('_').collect();
    if parts.len() != 2 {
        return None;
    }
    let dbno: u64 = parts[0].parse().ok()?;
    let ref_num: u64 = parts[1].parse().ok()?;
    Some((dbno << 32) | ref_num)
}

