use std::collections::{BTreeMap, BTreeSet, HashMap};

use aios_core::pdms_types::CATE_NEG_NOUN_NAMES;
use aios_core::{NamedAttrMap, RefnoEnum, Transform};

use super::context::GenerationReadContext;

async fn load_attributes_on_demand(
    read: &GenerationReadContext,
    refnos: &[RefnoEnum],
) -> anyhow::Result<BTreeMap<RefnoEnum, crate::generation_read::AttributeSet>> {
    let loaded = read.session.load_attribute_sets(refnos).await?;
    match loaded.require_all("generation.attributes") {
        Ok(found) => Ok(found),
        Err(error) => {
            #[cfg(all(feature = "sqlite-index", feature = "surreal-save"))]
            if matches!(
                &error,
                crate::generation_read::GenerationReadError::MissingRequiredData { .. }
            ) && crate::data_interface::cata_closure::cata_closure_sync_mode()
                == crate::data_interface::cata_closure::CataClosureSyncMode::Manifest
            {
                let _guard = read.runtime_attribute_load_lock.lock().await;
                let mut found = refnos
                    .iter()
                    .filter_map(|refno| {
                        read.runtime_attributes
                            .get(refno)
                            .map(|attributes| (*refno, attributes.clone()))
                    })
                    .collect::<BTreeMap<_, _>>();
                let missing = refnos
                    .iter()
                    .copied()
                    .filter(|refno| !found.contains_key(refno))
                    .collect::<Vec<_>>();
                let outcome =
                    crate::data_interface::cata_closure::ensure_cata_refnos_parsed(
                        &missing.iter().map(|refno| refno.refno()).collect::<Vec<_>>(),
                    )
                    .await?;
                for refno in missing {
                    let named = outcome.attributes.get(&refno.refno()).ok_or_else(|| {
                        anyhow::anyhow!(
                            "按需解析后仍缺少 CATA 属性: {} (parsed={}, missing={})",
                            refno,
                            outcome.parsed,
                            outcome.missing
                        )
                    })?;
                    let attributes = crate::generation_read::AttributeSet::from_named_attr_map(
                        refno,
                        named,
                    );
                    read.runtime_attributes.insert(refno, attributes.clone());
                    found.insert(refno, attributes);
                }
                return Ok(found);
            }
            Err(error.into())
        }
    }
}

pub async fn get_named_attmap(
    read: &GenerationReadContext,
    refno: RefnoEnum,
) -> anyhow::Result<NamedAttrMap> {
    if let Some(attributes) = read.attributes.get(&refno) {
        return Ok(attributes.to_named_attr_map());
    }
    if let Some(attributes) = read.runtime_attributes.get(&refno) {
        return Ok(attributes.to_named_attr_map());
    }
    Ok(load_attributes_on_demand(read, &[refno])
        .await?
        .remove(&refno)
        .expect("require_all checked refno")
        .to_named_attr_map())
}

pub async fn get_named_attmaps(
    read: &GenerationReadContext,
    refnos: &[RefnoEnum],
) -> anyhow::Result<BTreeMap<RefnoEnum, NamedAttrMap>> {
    let mut found = refnos
        .iter()
        .filter_map(|refno| {
            read.attributes
                .get(refno)
                .cloned()
                .or_else(|| {
                    read.runtime_attributes
                        .get(refno)
                        .map(|attributes| attributes.clone())
                })
                .map(|attributes| (*refno, attributes))
        })
        .collect::<BTreeMap<_, _>>();
    let missing = refnos
        .iter()
        .copied()
        .filter(|refno| !found.contains_key(refno))
        .collect::<Vec<_>>();
    if !missing.is_empty() {
        found.extend(load_attributes_on_demand(read, &missing).await?);
    }
    Ok(found
        .into_iter()
        .map(|(refno, attributes)| (refno, attributes.to_named_attr_map()))
        .collect())
}

pub async fn get_world_transforms(
    read: &GenerationReadContext,
    refnos: &[RefnoEnum],
) -> anyhow::Result<BTreeMap<RefnoEnum, Transform>> {
    let mut found = refnos
        .iter()
        .filter_map(|refno| {
            read.transforms
                .get(refno)
                .cloned()
                .map(|transform| (*refno, transform.world))
        })
        .collect::<BTreeMap<_, _>>();
    let missing = refnos
        .iter()
        .copied()
        .filter(|refno| !found.contains_key(refno))
        .collect::<Vec<_>>();
    if !missing.is_empty() {
        found.extend(
            super::transform_cache::get_world_transforms_cache_first_batch(None, &missing).await?,
        );
    }
    Ok(
        crate::generation_read::BatchLookup::from_found(refnos, found)
            .require_all("generation.transforms")?,
    )
}

pub async fn get_world_transform(
    read: &GenerationReadContext,
    refno: RefnoEnum,
) -> anyhow::Result<Transform> {
    let mut transforms = get_world_transforms(read, &[refno]).await?;
    transforms
        .remove(&refno)
        .ok_or_else(|| anyhow::anyhow!("transform adapter omitted refno={refno}"))
}

pub fn get_children(read: &GenerationReadContext, refno: RefnoEnum) -> Vec<RefnoEnum> {
    read.hierarchy
        .node(refno)
        .map(|_| read.hierarchy.children_of(refno).to_vec())
        .or_else(|| {
            read.catalog_nodes
                .get(&refno)
                .map(|node| node.children.clone())
        })
        .unwrap_or_default()
}

pub fn get_type_name(read: &GenerationReadContext, refno: RefnoEnum) -> anyhow::Result<String> {
    read.hierarchy
        .node(refno)
        .map(|node| node.noun.clone())
        .ok_or_else(|| anyhow::anyhow!("hierarchy missing refno={refno}"))
}

pub fn get_descendants_by_types(
    read: &GenerationReadContext,
    root: RefnoEnum,
    nouns: &[&str],
    max_depth: Option<usize>,
    include_self: bool,
) -> anyhow::Result<Vec<RefnoEnum>> {
    read.hierarchy
        .descendants(
            &[root],
            &crate::generation_read::HierarchyQuery {
                include_self,
                nouns: nouns.iter().map(|noun| noun.to_string()).collect(),
                max_depth,
                prune_on_match: false,
            },
        )
        .map_err(anyhow::Error::new)
}

pub fn get_multi_descendants_by_types(
    read: &GenerationReadContext,
    roots: &[RefnoEnum],
    nouns: &[&str],
    include_self: bool,
) -> anyhow::Result<Vec<RefnoEnum>> {
    read.hierarchy
        .descendants(
            roots,
            &crate::generation_read::HierarchyQuery {
                include_self,
                nouns: nouns.iter().map(|noun| noun.to_string()).collect(),
                max_depth: None,
                prune_on_match: false,
            },
        )
        .map_err(anyhow::Error::new)
}

pub fn get_ancestors_by_types(
    read: &GenerationReadContext,
    start: RefnoEnum,
    nouns: &[&str],
) -> anyhow::Result<Vec<RefnoEnum>> {
    let nouns: BTreeSet<String> = nouns.iter().map(|noun| noun.to_ascii_uppercase()).collect();
    let mut current = start;
    let mut visited = BTreeSet::new();
    let mut result = Vec::new();
    while let Some(parent) = read.hierarchy.parent_of(current) {
        if !visited.insert(parent) {
            anyhow::bail!("hierarchy ancestor cycle at refno={parent}");
        }
        let node = read
            .hierarchy
            .node(parent)
            .ok_or_else(|| anyhow::anyhow!("hierarchy missing ancestor refno={parent}"))?;
        if nouns.is_empty() || nouns.contains(&node.noun.to_ascii_uppercase()) {
            result.push(parent);
        }
        current = parent;
    }
    Ok(result)
}

pub async fn get_descendant_attmaps(
    read: &GenerationReadContext,
    root: RefnoEnum,
    nouns: &[&str],
) -> anyhow::Result<Vec<NamedAttrMap>> {
    let refnos = get_descendants_by_types(read, root, nouns, None, false)?;
    let mut attributes = get_named_attmaps(read, &refnos).await?;
    Ok(refnos
        .into_iter()
        .filter_map(|refno| attributes.remove(&refno))
        .collect())
}

pub async fn find_reference_sources(
    read: &GenerationReadContext,
    target: RefnoEnum,
    labels: &[&str],
) -> anyhow::Result<Vec<RefnoEnum>> {
    let labels: BTreeSet<String> = labels.iter().map(|label| label.to_string()).collect();
    let mut sources: Vec<_> = read
        .catalog_nodes
        .values()
        .filter(|node| {
            node.outbound.iter().any(|reference| {
                labels.contains(&reference.attribute_name) && reference.target == target
            })
        })
        .map(|node| node.refno)
        .collect();
    sources.sort_unstable();
    sources.dedup();
    Ok(sources)
}

pub async fn follow_reference_path(
    read: &GenerationReadContext,
    start: RefnoEnum,
    labels: &[&str],
) -> anyhow::Result<Option<RefnoEnum>> {
    let mut current = start;
    for label in labels {
        let Some(node) = read.catalog_nodes.get(&current) else {
            return Ok(None);
        };
        let Some(next) = node
            .outbound
            .iter()
            .filter(|reference| reference.attribute_name.eq_ignore_ascii_case(label))
            .min_by_key(|reference| reference.ordinal)
            .map(|reference| reference.target)
        else {
            return Ok(None);
        };
        current = next;
    }
    Ok(Some(current))
}

pub async fn first_outbound_reference(
    read: &GenerationReadContext,
    start: RefnoEnum,
    labels: &[&str],
) -> anyhow::Result<Option<RefnoEnum>> {
    let Some(node) = read.catalog_nodes.get(&start) else {
        return Ok(None);
    };
    Ok(labels.iter().find_map(|label| {
        node.outbound
            .iter()
            .filter(|reference| reference.attribute_name.eq_ignore_ascii_case(label))
            .min_by_key(|reference| reference.ordinal)
            .map(|reference| reference.target)
    }))
}

pub async fn get_pos_neg_map(
    read: &GenerationReadContext,
    roots: &[RefnoEnum],
) -> anyhow::Result<HashMap<RefnoEnum, Vec<RefnoEnum>>> {
    let mut refnos = read.hierarchy.descendants(
        roots,
        &crate::generation_read::HierarchyQuery {
            include_self: true,
            nouns: BTreeSet::new(),
            max_depth: None,
            prune_on_match: false,
        },
    )?;
    refnos.sort_unstable();
    refnos.dedup();
    let negative_nouns: BTreeSet<&str> = CATE_NEG_NOUN_NAMES.iter().copied().collect();
    let mut result: HashMap<RefnoEnum, Vec<RefnoEnum>> = HashMap::new();
    for refno in refnos {
        let Some(node) = read.hierarchy.node(refno) else {
            continue;
        };
        if negative_nouns.contains(node.noun.as_str()) && node.owner.is_valid() {
            result.entry(node.owner).or_default().push(node.refno);
        }
    }
    for negatives in result.values_mut() {
        negatives.sort_unstable();
        negatives.dedup();
    }
    Ok(result)
}

pub fn dbnums(read: &GenerationReadContext) -> BTreeSet<u32> {
    read.session.manifest().versions.keys().copied().collect()
}
