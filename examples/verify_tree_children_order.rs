//! 验证 tree 文件中的 children 顺序是否正确
//!
//! 用法：
//!   TREE_PATH="output/scene_tree/7997.tree" PARENT_REFNO="7997/1234" cargo run --release --example verify_tree_children_order

use anyhow::{Context, Result};
use aios_core::tree_query::TreeFile;
use aios_core::RefU64;
use std::env;
use std::path::PathBuf;

fn main() -> Result<()> {
    let tree_path_str = env::var("TREE_PATH")
        .unwrap_or_else(|_| "output/scene_tree/7997.tree".to_string());
    let tree_path = PathBuf::from(&tree_path_str);

    println!("加载 tree 文件: {}", tree_path.display());

    let tree_file = TreeFile::load(&tree_path)
        .context(format!("无法加载 tree 文件: {}", tree_path.display()))?;

    println!("dbnum: {}", tree_file.dbnum);
    println!("root_refno: {}", tree_file.root_refno);
    println!("arena 节点数: {}", tree_file.arena.count());

    // 如果指定了 PARENT_REFNO，显示其 children
    if let Ok(parent_str) = env::var("PARENT_REFNO") {
        let parts: Vec<&str> = parent_str.split(&['/', '_'][..]).collect();
        if parts.len() == 2 {
            let dbnum: u32 = parts[0].parse().unwrap_or(0);
            let refno: u32 = parts[1].parse().unwrap_or(0);
            let parent_refno = RefU64::from_two_nums(dbnum, refno);

            println!("\n查找 {} 的 children:", parent_refno);

            // 遍历 arena 找到这个节点
            for node in tree_file.arena.iter() {
                if node.get().refno == parent_refno {
                    println!("找到节点: {} (noun={})", parent_refno, node.get().noun);

                    // 获取 children
                    let node_id = tree_file.arena.get_node_id(node).unwrap();
                    let children: Vec<_> = node_id.children(&tree_file.arena)
                        .map(|child_id| {
                            let child = tree_file.arena.get(child_id).unwrap().get();
                            child.refno
                        })
                        .collect();

                    println!("children 数量: {}", children.len());
                    for (i, child_refno) in children.iter().enumerate() {
                        println!("  [{}] {}", i, child_refno);
                    }
                    break;
                }
            }
        }
    } else {
        // 显示前 10 个节点作为示例
        println!("\n前 10 个节点:");
        for (i, node) in tree_file.arena.iter().enumerate().take(10) {
            let meta = node.get();
            println!("  [{}] refno={} owner={} noun={}", i, meta.refno, meta.owner, meta.noun);
        }

        // 找到一个有多个 children 的 BRAN 节点作为示例
        println!("\n寻找有多个 children 的节点 (BRAN=23):");
        for node in tree_file.arena.iter() {
            let meta = node.get();
            if meta.noun == 23 { // BRAN
                let node_id = tree_file.arena.get_node_id(node).unwrap();
                let children_count = node_id.children(&tree_file.arena).count();
                if children_count >= 3 {
                    println!("  {} (children={})", meta.refno, children_count);

                    // 显示 children
                    let children: Vec<_> = node_id.children(&tree_file.arena)
                        .map(|child_id| {
                            let child = tree_file.arena.get(child_id).unwrap().get();
                            child.refno
                        })
                        .collect();
                    for (i, child_refno) in children.iter().enumerate().take(5) {
                        println!("    [{}] {}", i, child_refno);
                    }
                    if children.len() > 5 {
                        println!("    ... 共 {} 个", children.len());
                    }
                    break;
                }
            }
        }
    }

    Ok(())
}
