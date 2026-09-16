//! spec 011 诊断:dump .tree 文件中指定 refno 的存在性、父链与生成目标命中情况。
//!
//! 用法:
//!   cargo run --example dump_tree_node --features rvm-import -- <tree_file> <ref0> <ref1...>

use aios_core::RefU64;
use aios_core::pdms_types::{
    GNERAL_LOOP_OWNER_NOUN_NAMES, GNERAL_PRIM_NOUN_NAMES, USE_CATE_NOUN_NAMES,
};
use aios_core::tool::db_tool::{db1_dehash, db1_hash};
use aios_core::tree_query::{TreeIndex, TreeQueryFilter, TreeQueryOptions};
use std::collections::{HashMap, HashSet};

fn main() -> anyhow::Result<()> {
    let args: Vec<String> = std::env::args().collect();
    if args.len() < 4 {
        eprintln!("用法: dump_tree_node <tree_file> <ref0> <ref1> [ref1 ...]");
        std::process::exit(2);
    }
    let tree_path = &args[1];
    let ref0: u64 = args[2].parse()?;

    let index = TreeIndex::load_from_path(tree_path)?;
    println!("tree: {}", tree_path);
    println!("roots: {:?}", index.roots());

    let root_ref1: u64 = args[3].parse()?;
    let root = RefU64::from((ref0 << 32) | root_ref1);
    let target_nouns = default_target_nouns();
    let target_hashes: HashSet<u32> = target_nouns.iter().map(|n| db1_hash(n)).collect();
    let grouped = if index.contains_refno(root) {
        let options = TreeQueryOptions {
            include_self: true,
            max_depth: None,
            filter: TreeQueryFilter {
                noun_hashes: Some(target_hashes.clone()),
                ..Default::default()
            },
            prune_on_match: false,
        };
        index.collect_descendants_bfs_grouped(root, &options)
    } else {
        HashMap::new()
    };
    let grouped_hits: HashSet<RefU64> = grouped
        .values()
        .flat_map(|refnos| refnos.iter().copied())
        .collect();
    println!(
        "diagnostic_root: {} grouped_target_nouns={} grouped_refnos={}",
        format_refno(root),
        target_nouns.len(),
        grouped_hits.len()
    );
    print_grouped_summary(&grouped);

    for ref1_str in &args[3..] {
        let ref1: u64 = ref1_str.parse()?;
        let refno = RefU64::from((ref0 << 32) | ref1);
        let exists = index.contains_refno(refno);
        println!("\n== refno {ref0}/{ref1} (u64={}) exists={exists}", refno.0);
        if !exists {
            continue;
        }
        if let Some(meta) = index.node_meta(refno) {
            let noun = db1_dehash(meta.noun);
            println!(
                "   noun: {} ({}) category={:?} grouped_hit={}",
                noun,
                meta.noun,
                noun_category(meta.noun),
                grouped_hits.contains(&refno)
            );
        }
        // 父链:用 ancestors 查询(树内,root→parent 顺序)。
        let anc_options = TreeQueryOptions {
            include_self: false,
            max_depth: None,
            filter: TreeQueryFilter::default(),
            prune_on_match: false,
        };
        let ancestors = index.collect_ancestors_root_to_parent(refno, &anc_options);
        println!(
            "   ancestors({}): {:?}",
            ancestors.len(),
            ancestors
                .iter()
                .map(|r| format!("{}_{}", r.0 >> 32, r.0 & 0xFFFFFFFF))
                .collect::<Vec<_>>()
        );
        // 子节点。
        let options = TreeQueryOptions {
            include_self: false,
            max_depth: Some(1),
            filter: TreeQueryFilter::default(),
            prune_on_match: false,
        };
        let children = index.collect_descendants_bfs(refno, &options);
        println!(
            "   children({}): {:?}",
            children.len(),
            children
                .iter()
                .take(20)
                .map(|r| format!("{}_{}", r.0 >> 32, r.0 & 0xFFFFFFFF))
                .collect::<Vec<_>>()
        );
    }
    Ok(())
}

fn default_target_nouns() -> Vec<&'static str> {
    let mut seen = HashSet::new();
    let mut nouns = Vec::new();
    for noun in GNERAL_LOOP_OWNER_NOUN_NAMES
        .iter()
        .chain(GNERAL_PRIM_NOUN_NAMES.iter())
        .chain(USE_CATE_NOUN_NAMES.iter())
    {
        if seen.insert(*noun) {
            nouns.push(*noun);
        }
    }
    nouns
}

fn noun_category(noun_hash: u32) -> Option<&'static str> {
    if GNERAL_PRIM_NOUN_NAMES
        .iter()
        .any(|noun| db1_hash(noun) == noun_hash)
    {
        Some("prim")
    } else if USE_CATE_NOUN_NAMES
        .iter()
        .any(|noun| db1_hash(noun) == noun_hash)
    {
        Some("cate")
    } else if GNERAL_LOOP_OWNER_NOUN_NAMES
        .iter()
        .any(|noun| db1_hash(noun) == noun_hash)
    {
        Some("loop")
    } else {
        None
    }
}

fn print_grouped_summary(grouped: &HashMap<u32, Vec<RefU64>>) {
    let mut items: Vec<_> = grouped
        .iter()
        .map(|(noun_hash, refnos)| (db1_dehash(*noun_hash), *noun_hash, refnos.len()))
        .collect();
    items.sort_by(|a, b| a.0.cmp(&b.0));
    println!("grouped_summary:");
    for (noun, noun_hash, count) in items {
        println!("   {noun} ({noun_hash}): {count}");
    }
}

fn format_refno(refno: RefU64) -> String {
    format!("{}_{}", refno.0 >> 32, refno.0 & 0xFFFFFFFF)
}
