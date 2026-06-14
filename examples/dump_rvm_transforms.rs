use anyhow::{Context, Result};
use rvm_rs::parse_rvm;
use rvm_rs::store::Store;
use rvm_rs::store::node::{NodeId, NodeKind};
use std::collections::VecDeque;
use std::path::Path;

fn walk(store: &Store, node_id: NodeId, path: &mut VecDeque<String>, filter: &str) -> Result<()> {
    let node = store
        .get_node(node_id)
        .with_context(|| format!("missing node {}", node_id.0))?;

    match &node.kind {
        NodeKind::File(file) => {
            path.push_back(store.get_string(file.info).to_string());
        }
        NodeKind::Model(model) => {
            path.push_back(store.get_string(model.name).to_string());
        }
        NodeKind::Group(group) => {
            path.push_back(store.get_string(group.name).to_string());
            let joined = path.iter().cloned().collect::<Vec<_>>().join("/");
            if joined.contains(filter) {
                println!(
                    "GROUP path={joined} translation={:?} bbox={:?}",
                    group.translation, group.bbox_world
                );
                let mut geometry_id = group.first_geometry;
                while let Some(id) = geometry_id {
                    let geometry = store
                        .get_geometry(id)
                        .with_context(|| format!("missing geometry {}", id.0))?;
                    println!(
                        "  GEO kind={:?} translation={:?} matrix={:?} bbox_world={:?}",
                        geometry.kind,
                        geometry.transform.translation,
                        geometry.transform.matrix3,
                        geometry.bbox_world
                    );
                    geometry_id = geometry.next;
                }
            }
        }
    }

    let mut child = node.first_child;
    while let Some(child_id) = child {
        let child_node = store
            .get_node(child_id)
            .with_context(|| format!("missing child {}", child_id.0))?;
        walk(store, child_id, path, filter)?;
        child = child_node.next;
    }
    path.pop_back();
    Ok(())
}

fn main() -> Result<()> {
    let args = std::env::args().collect::<Vec<_>>();
    anyhow::ensure!(
        args.len() >= 3,
        "usage: dump_rvm_transforms <rvm-file> <path-filter>"
    );
    let bytes = std::fs::read(Path::new(&args[1]))?;
    let mut store = Store::new();
    parse_rvm(&bytes, &mut store)?;
    for &root in store.roots() {
        walk(&store, root, &mut VecDeque::new(), &args[2])?;
    }
    Ok(())
}
