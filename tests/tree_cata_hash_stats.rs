use aios_core::tree_query::{TreeQueryFilter, TreeQueryOptions};
use aios_core::RefU64;
use aios_database::fast_model::gen_model::tree_index_manager::TreeIndexManager;
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;

fn resolve_tree_dir() -> Option<PathBuf> {
    std::env::var("TREE_DIR").ok().map(PathBuf::from)
}

fn resolve_dbnum() -> u32 {
    std::env::var("TREE_DBNUM")
        .ok()
        .and_then(|v| v.parse::<u32>().ok())
        .unwrap_or(1112)
}

fn resolve_print_all() -> bool {
    std::env::var("TREE_PRINT_ALL")
        .ok()
        .and_then(|v| v.parse::<bool>().ok())
        .unwrap_or(true)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PrintMode {
    All,
    Duplicates,
    Summary,
}

fn resolve_print_mode() -> PrintMode {
    if let Ok(mode) = std::env::var("TREE_PRINT_MODE") {
        match mode.trim().to_lowercase().as_str() {
            "summary" => return PrintMode::Summary,
            "duplicates" | "dup" | "dups" => return PrintMode::Duplicates,
            "all" => return PrintMode::All,
            _ => {}
        }
    }
    if resolve_print_all() {
        PrintMode::All
    } else {
        PrintMode::Duplicates
    }
}

#[test]
fn test_tree_cata_hash_stats() {
    let dbnum = resolve_dbnum();
    let manager = if let Some(tree_dir) = resolve_tree_dir() {
        TreeIndexManager::new(tree_dir, vec![dbnum])
    } else {
        TreeIndexManager::with_default_dir(vec![dbnum])
    };
    let tree_dir = manager.tree_dir();
    assert!(
        tree_dir.exists(),
        "tree dir not found: {}",
        tree_dir.display()
    );

    let index = manager
        .load_index(dbnum)
        .unwrap_or_else(|e| panic!("load tree index failed: {e}"));

    let mut visited = HashSet::new();
    let mut bfs_refnos: Vec<RefU64> = Vec::new();
    let options = TreeQueryOptions {
        include_self: true,
        max_depth: None,
        filter: TreeQueryFilter::default(),
    };

    for &root in index.roots() {
        for refno in index.collect_descendants_bfs(root, &options) {
            if visited.insert(refno) {
                bfs_refnos.push(refno);
            }
        }
    }

    let mut stats: HashMap<String, (usize, RefU64)> = HashMap::new();
    let mut cata_nodes = 0usize;
    for refno in bfs_refnos.iter().copied() {
        if let Some(meta) = index.node_meta(refno) {
            if let Some(hash) = meta.cata_hash {
                cata_nodes += 1;
                stats
                    .entry(hash)
                    .and_modify(|entry| entry.0 += 1)
                    .or_insert((1, refno));
            }
        }
    }

    let mut rows: Vec<(String, usize, RefU64)> = stats
        .into_iter()
        .map(|(hash, (count, first_refno))| (hash, count, first_refno))
        .collect();
    rows.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));

    println!("tree dbnum: {}", dbnum);
    println!("total nodes (bfs): {}", bfs_refnos.len());
    println!("nodes with cata_hash: {}", cata_nodes);
    println!("distinct cata_hash: {}", rows.len());
    let duplicates = rows.iter().filter(|(_, count, _)| *count > 1).count();
    let max_count = rows.iter().map(|(_, count, _)| *count).max().unwrap_or(0);
    println!("duplicate cata_hash (>1): {}", duplicates);
    println!("max count per cata_hash: {}", max_count);

    match resolve_print_mode() {
        PrintMode::Summary => {
            println!("print mode: summary");
        }
        PrintMode::All => {
            println!("print mode: all");
            for (hash, count, first_refno) in rows {
                println!("cata_hash={hash}, count={count}, first_refno={first_refno}");
            }
        }
        PrintMode::Duplicates => {
            println!("print mode: duplicates");
            if duplicates == 0 {
                println!("no duplicate cata_hash found (all counts == 1)");
            } else {
                for (hash, count, first_refno) in rows.into_iter().filter(|(_, count, _)| *count > 1) {
                    println!("cata_hash={hash}, count={count}, first_refno={first_refno}");
                }
            }
        }
    }
}
