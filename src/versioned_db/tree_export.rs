//! tree_export - 场景树导出
//! 将解析的层级结构导出为 .tree 文件

use aios_core::RefU64;
use aios_core::db::DbBasicData;
use indextree::Arena;
use std::collections::HashMap;
use std::path::Path;

/// 树节点元数据（本地定义，用于解析期收集）
#[derive(Debug, Clone, Default)]
pub struct TreeNodeMeta {
    pub refno: RefU64,
    pub owner: RefU64,
    pub noun: u32,
    pub cata_hash: Option<u64>,
}

/// 导出树文件
pub fn export_tree_file(
    dbnum: u32,
    _db_basic: &DbBasicData,
    tree_nodes: &HashMap<RefU64, TreeNodeMeta>,
    children_map: &HashMap<RefU64, Vec<RefU64>>,
    output_dir: &Path,
) -> anyhow::Result<()> {
    use aios_core::tree_query::{TreeFile, TreeNodeMeta as CoreTreeNodeMeta};
    use std::fs;

    if tree_nodes.is_empty() {
        return Ok(());
    }

    // 构建 indextree Arena
    let mut arena: Arena<CoreTreeNodeMeta> = Arena::new();
    let mut id_map: HashMap<RefU64, indextree::NodeId> = HashMap::new();

    // 第一遍：创建所有节点
    for (refno, meta) in tree_nodes {
        let core_meta = CoreTreeNodeMeta {
            refno: *refno,
            owner: meta.owner,
            noun: meta.noun,
            cata_hash: meta.cata_hash.clone(),
        };
        let node_id = arena.new_node(core_meta);
        id_map.insert(*refno, node_id);
    }

    // 第二遍：直接用 children_map 按顺序 append（保持原始顺序）
    for (parent_refno, children) in children_map {
        if let Some(&parent_id) = id_map.get(parent_refno) {
            for child_refno in children {
                if let Some(&child_id) = id_map.get(child_refno) {
                    parent_id.append(child_id, &mut arena);
                }
            }
        }
    }

    // 找到根节点
    let root_refno = tree_nodes
        .iter()
        .find(|(refno, meta)| meta.owner == **refno)
        .map(|(refno, _)| *refno)
        .unwrap_or_default();

    let tree_file = TreeFile {
        dbnum,
        root_refno,
        arena,
    };

    // 确保目录存在
    fs::create_dir_all(output_dir)?;

    // 保存文件
    let path = output_dir.join(format!("{}.tree", dbnum));
    tree_file.save(&path)?;

    log::info!(
        "[tree_export] 导出 {} 节点到 {}",
        tree_nodes.len(),
        path.display()
    );

    Ok(())
}
