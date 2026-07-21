use std::collections::{BTreeMap, BTreeSet, HashSet, VecDeque};
use std::sync::Arc;

use aios_core::RefnoEnum;
use serde::{Deserialize, Serialize};

use super::error::{GenerationReadError, GenerationReadResult};
use super::traits::VersionedReadSession;
use super::types::{ElementQuery, ElementSnapshot, HierarchyRow};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HierarchyNode {
    pub refno: RefnoEnum,
    pub dbnum: u32,
    pub owner: RefnoEnum,
    pub noun: String,
    pub name: String,
}

impl From<ElementSnapshot> for HierarchyNode {
    fn from(value: ElementSnapshot) -> Self {
        Self {
            refno: value.refno,
            dbnum: value.dbnum,
            owner: value.owner,
            noun: value.noun,
            name: value.name,
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct HierarchyQuery {
    pub include_self: bool,
    pub nouns: BTreeSet<String>,
    pub max_depth: Option<usize>,
    pub prune_on_match: bool,
}

#[derive(Debug, Clone)]
pub struct HierarchySnapshot {
    snapshot_id: u64,
    nodes: BTreeMap<RefnoEnum, HierarchyNode>,
    children: BTreeMap<RefnoEnum, Vec<RefnoEnum>>,
    parent: BTreeMap<RefnoEnum, RefnoEnum>,
    roots: Vec<RefnoEnum>,
    noun_index: BTreeMap<String, Vec<RefnoEnum>>,
}

impl HierarchySnapshot {
    pub async fn load(
        session: Arc<dyn VersionedReadSession>,
        dbnums: &[u32],
    ) -> GenerationReadResult<Self> {
        let query = ElementQuery {
            dbnums: dbnums.iter().copied().collect(),
            ..ElementQuery::default()
        };
        let (elements, rows) = tokio::try_join!(
            session.query_elements(&query),
            session.load_hierarchy_rows(dbnums)
        )?;
        Self::from_parts(session.manifest().authoritative_snapshot_id, elements, rows)
    }

    pub fn from_parts(
        snapshot_id: u64,
        elements: Vec<ElementSnapshot>,
        rows: Vec<HierarchyRow>,
    ) -> GenerationReadResult<Self> {
        let mut nodes: BTreeMap<RefnoEnum, HierarchyNode> = BTreeMap::new();
        for element in elements {
            let refno = element.refno;
            if nodes.insert(refno, element.into()).is_some() {
                return Err(GenerationReadError::InvalidHierarchy(format!(
                    "重复节点 {refno}"
                )));
            }
        }

        let mut ordered_edges: BTreeMap<RefnoEnum, Vec<(u32, RefnoEnum)>> = BTreeMap::new();
        let mut parent = BTreeMap::new();
        let mut ordinals: BTreeSet<(RefnoEnum, u32)> = BTreeSet::new();

        for row in rows {
            if !nodes.contains_key(&row.parent) || !nodes.contains_key(&row.child) {
                return Err(GenerationReadError::InvalidHierarchy(format!(
                    "边端点缺失: {} -> {}",
                    row.parent, row.child
                )));
            }
            if row.parent == row.child {
                return Err(GenerationReadError::InvalidHierarchy(format!(
                    "节点不能是自己的 child: {}",
                    row.parent
                )));
            }
            if !ordinals.insert((row.parent, row.ordinal)) {
                return Err(GenerationReadError::InvalidHierarchy(format!(
                    "parent={} 的 ordinal={} 重复",
                    row.parent, row.ordinal
                )));
            }
            if let Some(existing) = parent.insert(row.child, row.parent)
                && existing != row.parent
            {
                return Err(GenerationReadError::InvalidHierarchy(format!(
                    "child={} 同时属于 parent={} 和 {}",
                    row.child, existing, row.parent
                )));
            }
            ordered_edges
                .entry(row.parent)
                .or_default()
                .push((row.ordinal, row.child));
        }

        let children = ordered_edges
            .into_iter()
            .map(|(parent, mut children)| {
                children.sort_unstable_by_key(|(ordinal, child)| (*ordinal, *child));
                (
                    parent,
                    children.into_iter().map(|(_, child)| child).collect(),
                )
            })
            .collect();

        let mut roots: Vec<_> = nodes
            .keys()
            .filter(|refno| !parent.contains_key(refno))
            .copied()
            .collect();
        roots.sort_unstable();

        let mut noun_index: BTreeMap<String, Vec<RefnoEnum>> = BTreeMap::new();
        for node in nodes.values() {
            noun_index
                .entry(node.noun.to_ascii_uppercase())
                .or_default()
                .push(node.refno);
        }
        for refnos in noun_index.values_mut() {
            refnos.sort_unstable();
        }

        let snapshot = Self {
            snapshot_id,
            nodes,
            children,
            parent,
            roots,
            noun_index,
        };
        snapshot.validate_acyclic()?;
        Ok(snapshot)
    }

    pub fn snapshot_id(&self) -> u64 {
        self.snapshot_id
    }

    pub fn node(&self, refno: RefnoEnum) -> Option<&HierarchyNode> {
        self.nodes.get(&refno)
    }

    pub fn roots(&self) -> &[RefnoEnum] {
        &self.roots
    }

    pub fn all_refnos(&self) -> Vec<RefnoEnum> {
        self.nodes.keys().copied().collect()
    }

    pub fn children_of(&self, refno: RefnoEnum) -> &[RefnoEnum] {
        self.children.get(&refno).map(Vec::as_slice).unwrap_or(&[])
    }

    pub fn parent_of(&self, refno: RefnoEnum) -> Option<RefnoEnum> {
        self.parent.get(&refno).copied()
    }

    pub fn refnos_by_noun(&self, noun: &str) -> &[RefnoEnum] {
        self.noun_index
            .get(&noun.to_ascii_uppercase())
            .map(Vec::as_slice)
            .unwrap_or(&[])
    }

    pub fn refnos_by_noun_in_dbnums(&self, noun: &str, dbnums: &BTreeSet<u32>) -> Vec<RefnoEnum> {
        self.refnos_by_noun(noun)
            .iter()
            .copied()
            .filter(|refno| {
                dbnums.is_empty()
                    || self
                        .nodes
                        .get(refno)
                        .is_some_and(|node| dbnums.contains(&node.dbnum))
            })
            .collect()
    }

    pub fn ancestors(&self, refno: RefnoEnum) -> GenerationReadResult<Vec<RefnoEnum>> {
        if !self.nodes.contains_key(&refno) {
            return Err(GenerationReadError::MissingRequiredData {
                capability: "hierarchy.node",
                refnos: vec![refno],
            });
        }
        let mut current = refno;
        let mut visited = HashSet::new();
        let mut out = Vec::new();
        while let Some(parent) = self.parent_of(current) {
            if !visited.insert(parent) {
                return Err(GenerationReadError::InvalidHierarchy(format!(
                    "ancestor 环包含 {parent}"
                )));
            }
            out.push(parent);
            current = parent;
        }
        out.reverse();
        Ok(out)
    }

    pub fn descendants(
        &self,
        roots: &[RefnoEnum],
        query: &HierarchyQuery,
    ) -> GenerationReadResult<Vec<RefnoEnum>> {
        let missing: Vec<_> = roots
            .iter()
            .filter(|refno| !self.nodes.contains_key(refno))
            .copied()
            .collect();
        if !missing.is_empty() {
            return Err(GenerationReadError::MissingRequiredData {
                capability: "hierarchy.roots",
                refnos: missing,
            });
        }

        let nouns: BTreeSet<_> = query
            .nouns
            .iter()
            .map(|noun| noun.to_ascii_uppercase())
            .collect();
        let mut queue = VecDeque::new();
        let mut visited = HashSet::new();
        for root in roots {
            queue.push_back((*root, 0usize));
        }

        let mut out = Vec::new();
        while let Some((refno, depth)) = queue.pop_front() {
            if !visited.insert(refno) {
                continue;
            }
            let node = &self.nodes[&refno];
            let selected = (query.include_self || depth > 0)
                && (nouns.is_empty() || nouns.contains(&node.noun.to_ascii_uppercase()));
            if selected {
                out.push(refno);
            }

            if selected && query.prune_on_match {
                continue;
            }
            if query.max_depth.is_some_and(|max_depth| depth >= max_depth) {
                continue;
            }
            for child in self.children_of(refno) {
                queue.push_back((*child, depth + 1));
            }
        }
        Ok(out)
    }

    fn validate_acyclic(&self) -> GenerationReadResult<()> {
        for refno in self.nodes.keys() {
            let mut current = *refno;
            let mut visited = HashSet::new();
            while let Some(parent) = self.parent_of(current) {
                if !visited.insert(parent) {
                    return Err(GenerationReadError::InvalidHierarchy(format!(
                        "检测到 owner 环，起点={refno} 重复节点={parent}"
                    )));
                }
                current = parent;
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn refno(value: &str) -> RefnoEnum {
        RefnoEnum::from(value)
    }

    fn element(value: &str, owner: &str, noun: &str) -> ElementSnapshot {
        ElementSnapshot {
            refno: refno(value),
            dbnum: 1,
            owner: refno(owner),
            noun: noun.to_string(),
            name: value.to_string(),
            has_children: false,
        }
    }

    #[test]
    fn descendants_preserve_root_bfs_and_child_ordinal() {
        let root = refno("1/1");
        let first = refno("1/2");
        let second = refno("1/3");
        let grandchild = refno("1/4");
        let snapshot = HierarchySnapshot::from_parts(
            7,
            vec![
                element("1/1", "1/1", "ROOT"),
                element("1/2", "1/1", "NODE"),
                element("1/3", "1/1", "NODE"),
                element("1/4", "1/2", "LEAF"),
            ],
            vec![
                HierarchyRow {
                    dbnum: 1,
                    parent: root,
                    child: second,
                    ordinal: 1,
                },
                HierarchyRow {
                    dbnum: 1,
                    parent: first,
                    child: grandchild,
                    ordinal: 0,
                },
                HierarchyRow {
                    dbnum: 1,
                    parent: root,
                    child: first,
                    ordinal: 0,
                },
            ],
        )
        .expect("valid hierarchy");

        let actual = snapshot
            .descendants(&[root], &HierarchyQuery::default())
            .expect("descendants");
        assert_eq!(actual, vec![first, second, grandchild]);
    }

    #[test]
    fn prune_on_match_stops_below_selected_node() {
        let root = refno("1/1");
        let target = refno("1/2");
        let nested_target = refno("1/3");
        let snapshot = HierarchySnapshot::from_parts(
            8,
            vec![
                element("1/1", "1/1", "ROOT"),
                element("1/2", "1/1", "TARGET"),
                element("1/3", "1/2", "TARGET"),
            ],
            vec![
                HierarchyRow {
                    dbnum: 1,
                    parent: root,
                    child: target,
                    ordinal: 0,
                },
                HierarchyRow {
                    dbnum: 1,
                    parent: target,
                    child: nested_target,
                    ordinal: 0,
                },
            ],
        )
        .expect("valid hierarchy");
        let query = HierarchyQuery {
            nouns: ["target".to_string()].into_iter().collect(),
            prune_on_match: true,
            ..HierarchyQuery::default()
        };

        assert_eq!(
            snapshot.descendants(&[root], &query).expect("descendants"),
            vec![target]
        );
    }

    #[test]
    fn cycle_is_rejected_fail_closed() {
        let error = HierarchySnapshot::from_parts(
            9,
            vec![element("1/1", "1/2", "NODE"), element("1/2", "1/1", "NODE")],
            vec![
                HierarchyRow {
                    dbnum: 1,
                    parent: refno("1/1"),
                    child: refno("1/2"),
                    ordinal: 0,
                },
                HierarchyRow {
                    dbnum: 1,
                    parent: refno("1/2"),
                    child: refno("1/1"),
                    ordinal: 0,
                },
            ],
        )
        .expect_err("cycle must fail");
        assert!(matches!(error, GenerationReadError::InvalidHierarchy(_)));
    }
}
