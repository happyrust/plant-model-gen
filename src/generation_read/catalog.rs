use std::collections::{BTreeMap, BTreeSet, HashSet};
use std::sync::Arc;

use aios_core::RefnoEnum;
use serde::{Deserialize, Serialize};

use super::error::{GenerationReadError, GenerationReadResult};
use super::traits::VersionedReadSession;
use super::types::{CatalogNode, hash_serializable};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CatalogResolverConfig {
    pub resolver_contract_version: u16,
    pub max_rounds: usize,
    pub include_owner_chain: bool,
    pub catalog_db_types: BTreeSet<String>,
    pub excluded_nouns: BTreeSet<String>,
    pub no_children_nouns: BTreeSet<String>,
    pub precise_children_nouns: BTreeSet<String>,
}

impl Default for CatalogResolverConfig {
    fn default() -> Self {
        Self {
            resolver_contract_version: 2,
            max_rounds: 128,
            include_owner_chain: true,
            catalog_db_types: ["CATA", "SCHE", "DICT", "PROP"]
                .into_iter()
                .map(str::to_string)
                .collect(),
            excluded_nouns: ["DTEXT", "PTCA", "SPINE"]
                .into_iter()
                .map(str::to_string)
                .collect(),
            no_children_nouns: ["STRU", "PJOI", "SFIT"]
                .into_iter()
                .map(str::to_string)
                .collect(),
            precise_children_nouns: [
                "FULL", "TOPD", "TMPL", "PBOR", "CYLI", "GMSE", "NGMS", "PTSE", "PSTR", "SPRO",
                "DTSE",
            ]
            .into_iter()
            .map(str::to_string)
            .collect(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CatalogClosure {
    pub authoritative_snapshot_id: u64,
    pub resolver_contract_version: u16,
    pub cache_key: String,
    pub ordered_refnos: Vec<RefnoEnum>,
    pub by_dbnum: BTreeMap<u32, BTreeSet<RefnoEnum>>,
    pub rounds: usize,
}

pub struct CatalogResolver {
    session: Arc<dyn VersionedReadSession>,
    config: CatalogResolverConfig,
    preloaded_nodes: Option<Arc<BTreeMap<RefnoEnum, CatalogNode>>>,
    allow_missing_nodes: bool,
}

impl CatalogResolver {
    pub fn new(session: Arc<dyn VersionedReadSession>, config: CatalogResolverConfig) -> Self {
        Self {
            session,
            config,
            preloaded_nodes: None,
            allow_missing_nodes: false,
        }
    }

    pub fn with_preloaded_nodes(
        session: Arc<dyn VersionedReadSession>,
        config: CatalogResolverConfig,
        nodes: Arc<BTreeMap<RefnoEnum, CatalogNode>>,
    ) -> Self {
        Self {
            session,
            config,
            preloaded_nodes: Some(nodes),
            allow_missing_nodes: false,
        }
    }

    pub fn allow_missing_nodes(mut self) -> Self {
        self.allow_missing_nodes = true;
        self
    }

    pub fn config(&self) -> &CatalogResolverConfig {
        &self.config
    }

    pub async fn resolve(&self, seeds: &[RefnoEnum]) -> GenerationReadResult<CatalogClosure> {
        let mut scheduled = HashSet::new();
        let mut frontier = Vec::new();
        for seed in seeds {
            if seed.is_valid() && scheduled.insert(*seed) {
                frontier.push(*seed);
            }
        }

        let seed_hash = hash_serializable(&frontier);
        let snapshot_id = self.session.manifest().authoritative_snapshot_id;
        let cache_key = hash_serializable(&(
            snapshot_id,
            &seed_hash,
            self.config.resolver_contract_version,
        ));

        let mut ordered_refnos = Vec::new();
        let mut by_dbnum: BTreeMap<u32, BTreeSet<RefnoEnum>> = BTreeMap::new();
        let mut rounds = 0usize;

        while !frontier.is_empty() {
            if rounds >= self.config.max_rounds {
                return Err(GenerationReadError::InvalidCatalog(format!(
                    "闭包超过最大轮数 {}，剩余 frontier={}",
                    self.config.max_rounds,
                    frontier.len()
                )));
            }
            rounds += 1;

            let nodes = if let Some(preloaded) = &self.preloaded_nodes {
                let lookup = super::types::BatchLookup::from_found(
                    &frontier,
                    frontier.iter().filter_map(|refno| {
                        preloaded.get(refno).cloned().map(|node| (*refno, node))
                    }),
                );
                if self.allow_missing_nodes {
                    lookup.found
                } else {
                    lookup.require_all("catalog.nodes")?
                }
            } else {
                let lookup = self.session.load_catalog_nodes(&frontier).await?;
                if self.allow_missing_nodes {
                    lookup.found
                } else {
                    lookup.require_all("catalog.nodes")?
                }
            };
            let mut next = Vec::new();

            for refno in &frontier {
                let Some(node) = nodes.get(refno) else {
                    if self.allow_missing_nodes {
                        continue;
                    }
                    return Err(GenerationReadError::InvalidCatalog(format!(
                        "adapter 未返回已声明存在的节点 {refno}"
                    )));
                };
                if !self.is_catalog_node(node) {
                    continue;
                }

                ordered_refnos.push(node.refno);
                by_dbnum.entry(node.dbnum).or_default().insert(node.refno);

                if self.config.include_owner_chain
                    && node.owner.is_valid()
                    && node.owner != node.refno
                {
                    schedule(node.owner, &mut scheduled, &mut next);
                }

                let noun = node.noun.to_ascii_uppercase();
                if self.config.excluded_nouns.contains(&noun) {
                    continue;
                }

                let mut outbound = node.outbound.clone();
                outbound.sort_unstable_by(|left, right| {
                    (&left.attribute_name, left.ordinal, left.target).cmp(&(
                        &right.attribute_name,
                        right.ordinal,
                        right.target,
                    ))
                });
                for edge in outbound {
                    if edge.target.is_valid() {
                        schedule(edge.target, &mut scheduled, &mut next);
                    }
                }

                if self.config.no_children_nouns.contains(&noun)
                    || !self.config.precise_children_nouns.contains(&noun)
                {
                    continue;
                }
                for child in &node.children {
                    if child.is_valid() {
                        schedule(*child, &mut scheduled, &mut next);
                    }
                }
            }

            frontier = next;
        }

        Ok(CatalogClosure {
            authoritative_snapshot_id: snapshot_id,
            resolver_contract_version: self.config.resolver_contract_version,
            cache_key,
            ordered_refnos,
            by_dbnum,
            rounds,
        })
    }

    fn is_catalog_node(&self, node: &CatalogNode) -> bool {
        self.config
            .catalog_db_types
            .contains(&node.db_type.to_ascii_uppercase())
    }
}

fn schedule(refno: RefnoEnum, scheduled: &mut HashSet<RefnoEnum>, next: &mut Vec<RefnoEnum>) {
    if scheduled.insert(refno) {
        next.push(refno);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::generation_read::{
        AttributeRead, AttributeReference, AttributeSet, BatchLookup, CatalogGraphRead,
        DataVersion, ElementQuery, ElementRead, ElementSnapshot, GenerationReadBackendKind,
        HierarchyRead, HierarchyRow, InputVersionManifest, SessionMetricsSnapshot, TransformRead,
        TransformSnapshot,
    };
    use async_trait::async_trait;

    struct FixtureSession {
        manifest: InputVersionManifest,
    }

    impl FixtureSession {
        fn new(snapshot_id: u64) -> Self {
            Self {
                manifest: InputVersionManifest::new(
                    snapshot_id,
                    1,
                    [DataVersion {
                        dbnum: 1,
                        sesno: 1,
                        commit_fingerprint: "catalog-fixture".to_string(),
                    }],
                )
                .expect("fixture manifest"),
            }
        }
    }

    impl VersionedReadSession for FixtureSession {
        fn manifest(&self) -> &InputVersionManifest {
            &self.manifest
        }

        fn backend_kind(&self) -> GenerationReadBackendKind {
            GenerationReadBackendKind::Surreal
        }

        fn metrics(&self) -> SessionMetricsSnapshot {
            SessionMetricsSnapshot::default()
        }
    }

    #[async_trait]
    impl ElementRead for FixtureSession {
        async fn load_elements(
            &self,
            _refnos: &[RefnoEnum],
        ) -> GenerationReadResult<BatchLookup<ElementSnapshot>> {
            unreachable!("catalog fixture uses preloaded nodes")
        }

        async fn query_elements(
            &self,
            _query: &ElementQuery,
        ) -> GenerationReadResult<Vec<ElementSnapshot>> {
            unreachable!("catalog fixture uses preloaded nodes")
        }
    }

    #[async_trait]
    impl AttributeRead for FixtureSession {
        async fn load_attribute_sets(
            &self,
            _refnos: &[RefnoEnum],
        ) -> GenerationReadResult<BatchLookup<AttributeSet>> {
            unreachable!("catalog fixture uses preloaded nodes")
        }
    }

    #[async_trait]
    impl HierarchyRead for FixtureSession {
        async fn load_hierarchy_rows(
            &self,
            _dbnums: &[u32],
        ) -> GenerationReadResult<Vec<HierarchyRow>> {
            unreachable!("catalog fixture uses preloaded nodes")
        }
    }

    #[async_trait]
    impl CatalogGraphRead for FixtureSession {
        async fn load_catalog_nodes(
            &self,
            _refnos: &[RefnoEnum],
        ) -> GenerationReadResult<BatchLookup<CatalogNode>> {
            unreachable!("catalog fixture uses preloaded nodes")
        }
    }

    #[async_trait]
    impl TransformRead for FixtureSession {
        async fn load_transforms(
            &self,
            _refnos: &[RefnoEnum],
        ) -> GenerationReadResult<BatchLookup<TransformSnapshot>> {
            unreachable!("catalog fixture uses preloaded nodes")
        }
    }

    fn node(
        refno: &str,
        db_type: &str,
        noun: &str,
        owner: &str,
        children: Vec<&str>,
        outbound: Vec<(&str, u32, &str)>,
    ) -> CatalogNode {
        let refno = RefnoEnum::from(refno);
        CatalogNode {
            refno,
            dbnum: 1,
            db_type: db_type.to_string(),
            noun: noun.to_string(),
            owner: RefnoEnum::from(owner),
            children: children.into_iter().map(RefnoEnum::from).collect(),
            outbound: outbound
                .into_iter()
                .map(|(attribute_name, ordinal, target)| AttributeReference {
                    dbnum: 1,
                    source: refno,
                    attribute_name: attribute_name.to_string(),
                    target: RefnoEnum::from(target),
                    ordinal,
                })
                .collect(),
        }
    }

    fn fixture_nodes() -> Arc<BTreeMap<RefnoEnum, CatalogNode>> {
        Arc::new(
            [
                node(
                    "1/1",
                    "CATA",
                    "FULL",
                    "1/2",
                    vec!["1/5", "1/6"],
                    vec![("BREF", 1, "1/3"), ("AREF", 0, "1/4")],
                ),
                node("1/2", "CATA", "SCOM", "1/2", vec![], vec![]),
                node("1/3", "CATA", "GMSE", "1/3", vec![], vec![]),
                node("1/4", "CATA", "SCOM", "1/4", vec![], vec![]),
                node("1/5", "CATA", "SCOM", "1/5", vec![], vec![]),
                node("1/6", "DESI", "EQUI", "1/6", vec![], vec![]),
            ]
            .into_iter()
            .map(|node| (node.refno, node))
            .collect(),
        )
    }

    #[tokio::test]
    async fn closure_order_and_cache_key_are_snapshot_bound() {
        let nodes = fixture_nodes();
        let first = CatalogResolver::with_preloaded_nodes(
            Arc::new(FixtureSession::new(1)),
            CatalogResolverConfig::default(),
            Arc::clone(&nodes),
        )
        .resolve(&[RefnoEnum::from("1/1")])
        .await
        .expect("first closure");

        assert_eq!(
            first.ordered_refnos,
            ["1/1", "1/2", "1/4", "1/3", "1/5"]
                .map(RefnoEnum::from)
                .to_vec()
        );
        assert_eq!(first.rounds, 2);

        let second = CatalogResolver::with_preloaded_nodes(
            Arc::new(FixtureSession::new(2)),
            CatalogResolverConfig::default(),
            nodes,
        )
        .resolve(&[RefnoEnum::from("1/1")])
        .await
        .expect("second closure");
        assert_eq!(first.ordered_refnos, second.ordered_refnos);
        assert_ne!(first.cache_key, second.cache_key);
    }

    #[tokio::test]
    async fn missing_scheduled_catalog_node_fails_closed() {
        let mut nodes = fixture_nodes().as_ref().clone();
        nodes.remove(&RefnoEnum::from("1/4"));
        let result = CatalogResolver::with_preloaded_nodes(
            Arc::new(FixtureSession::new(1)),
            CatalogResolverConfig::default(),
            Arc::new(nodes),
        )
        .resolve(&[RefnoEnum::from("1/1")])
        .await;
        assert!(matches!(
            result,
            Err(GenerationReadError::MissingRequiredData { .. })
        ));
    }
}
