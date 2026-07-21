use std::collections::BTreeSet;
use std::sync::Arc;

use aios_core::{RefnoEnum, Transform};
use async_trait::async_trait;
use serde::Serialize;

use super::error::{GenerationReadError, GenerationReadResult};
use super::traits::{
    AttributeRead, CatalogGraphRead, ElementRead, GenerationReadBackend, HierarchyRead,
    TransformRead, VersionedReadSession,
};
use super::types::{
    AttributeSet, BatchLookup, CatalogNode, ElementQuery, ElementSnapshot,
    GenerationReadBackendKind, HierarchyRow, InputVersionManifest, SessionMetricsSnapshot,
    TransformSnapshot, hash_serializable,
};

pub const TRANSFORM_ABS_TOLERANCE: f32 = 1.0e-5;

pub struct ComparingVersionedReadBackend {
    primary: Arc<dyn GenerationReadBackend>,
    secondary: Arc<dyn GenerationReadBackend>,
}

impl ComparingVersionedReadBackend {
    pub fn new(
        primary: Arc<dyn GenerationReadBackend>,
        secondary: Arc<dyn GenerationReadBackend>,
    ) -> GenerationReadResult<Self> {
        if primary.backend_kind() == GenerationReadBackendKind::Compare
            || secondary.backend_kind() == GenerationReadBackendKind::Compare
        {
            return Err(GenerationReadError::ParityMismatch {
                capability: "session.open",
                detail: "compare backend 不能嵌套".to_string(),
            });
        }
        if primary.backend_kind() == secondary.backend_kind() {
            return Err(GenerationReadError::ParityMismatch {
                capability: "session.open",
                detail: format!(
                    "compare backend 必须不同，当前均为 {}",
                    primary.backend_kind().as_str()
                ),
            });
        }
        Ok(Self { primary, secondary })
    }
}

pub struct ComparingVersionedReadSession {
    manifest: Arc<InputVersionManifest>,
    primary: Arc<dyn VersionedReadSession>,
    secondary: Arc<dyn VersionedReadSession>,
}

#[async_trait]
impl GenerationReadBackend for ComparingVersionedReadBackend {
    fn backend_kind(&self) -> GenerationReadBackendKind {
        GenerationReadBackendKind::Compare
    }

    async fn open_session(
        &self,
        manifest: Arc<InputVersionManifest>,
    ) -> GenerationReadResult<Arc<dyn VersionedReadSession>> {
        let (primary, secondary) = tokio::try_join!(
            self.primary.open_session(Arc::clone(&manifest)),
            self.secondary.open_session(Arc::clone(&manifest))
        )?;
        if primary.manifest() != secondary.manifest() {
            return Err(mismatch(
                "session.manifest",
                primary.manifest(),
                secondary.manifest(),
            ));
        }
        Ok(Arc::new(ComparingVersionedReadSession {
            manifest,
            primary,
            secondary,
        }))
    }
}

impl VersionedReadSession for ComparingVersionedReadSession {
    fn manifest(&self) -> &InputVersionManifest {
        &self.manifest
    }

    fn backend_kind(&self) -> GenerationReadBackendKind {
        GenerationReadBackendKind::Compare
    }

    fn metrics(&self) -> SessionMetricsSnapshot {
        let mut combined = self.primary.metrics();
        merge_metrics(&mut combined, self.secondary.metrics(), "secondary.");
        combined
    }
}

#[async_trait]
impl ElementRead for ComparingVersionedReadSession {
    async fn load_elements(
        &self,
        refnos: &[RefnoEnum],
    ) -> GenerationReadResult<BatchLookup<ElementSnapshot>> {
        let (primary, secondary) = tokio::try_join!(
            self.primary.load_elements(refnos),
            self.secondary.load_elements(refnos)
        )?;
        require_equal("element.load", &primary, &secondary)?;
        Ok(primary)
    }

    async fn query_elements(
        &self,
        query: &ElementQuery,
    ) -> GenerationReadResult<Vec<ElementSnapshot>> {
        let (primary, secondary) = tokio::try_join!(
            self.primary.query_elements(query),
            self.secondary.query_elements(query)
        )?;
        require_equal("element.query", &primary, &secondary)?;
        Ok(primary)
    }
}

#[async_trait]
impl AttributeRead for ComparingVersionedReadSession {
    async fn load_attribute_sets(
        &self,
        refnos: &[RefnoEnum],
    ) -> GenerationReadResult<BatchLookup<AttributeSet>> {
        let (primary, secondary) = tokio::try_join!(
            self.primary.load_attribute_sets(refnos),
            self.secondary.load_attribute_sets(refnos)
        )?;
        require_equal("attribute.load", &primary, &secondary)?;
        Ok(primary)
    }
}

#[async_trait]
impl HierarchyRead for ComparingVersionedReadSession {
    async fn load_hierarchy_rows(&self, dbnums: &[u32]) -> GenerationReadResult<Vec<HierarchyRow>> {
        let (primary, secondary) = tokio::try_join!(
            self.primary.load_hierarchy_rows(dbnums),
            self.secondary.load_hierarchy_rows(dbnums)
        )?;
        require_equal("hierarchy.load", &primary, &secondary)?;
        Ok(primary)
    }
}

#[async_trait]
impl CatalogGraphRead for ComparingVersionedReadSession {
    async fn load_catalog_nodes(
        &self,
        refnos: &[RefnoEnum],
    ) -> GenerationReadResult<BatchLookup<CatalogNode>> {
        let (primary, secondary) = tokio::try_join!(
            self.primary.load_catalog_nodes(refnos),
            self.secondary.load_catalog_nodes(refnos)
        )?;
        require_equal("catalog.load", &primary, &secondary)?;
        Ok(primary)
    }
}

#[async_trait]
impl TransformRead for ComparingVersionedReadSession {
    async fn load_transforms(
        &self,
        refnos: &[RefnoEnum],
    ) -> GenerationReadResult<BatchLookup<TransformSnapshot>> {
        let (primary, secondary) = tokio::try_join!(
            self.primary.load_transforms(refnos),
            self.secondary.load_transforms(refnos)
        )?;
        require_transforms_equal(&primary, &secondary)?;
        Ok(primary)
    }
}

fn require_transforms_equal(
    primary: &BatchLookup<TransformSnapshot>,
    secondary: &BatchLookup<TransformSnapshot>,
) -> GenerationReadResult<()> {
    let equal = primary.missing == secondary.missing
        && primary.found.len() == secondary.found.len()
        && primary.found.iter().all(|(refno, left)| {
            secondary.found.get(refno).is_some_and(|right| {
                left.refno == right.refno
                    && left.dbnum == right.dbnum
                    && optional_transform_approx_eq(left.local.as_ref(), right.local.as_ref())
                    && transform_approx_eq(&left.world, &right.world)
            })
        });
    if equal {
        Ok(())
    } else {
        Err(GenerationReadError::ParityMismatch {
            capability: "transform.load",
            detail: format!(
                "transform tolerance={} primary_hash={} secondary_hash={}",
                TRANSFORM_ABS_TOLERANCE,
                hash_serializable(primary),
                hash_serializable(secondary)
            ),
        })
    }
}

fn optional_transform_approx_eq(left: Option<&Transform>, right: Option<&Transform>) -> bool {
    match (left, right) {
        (None, None) => true,
        (Some(left), Some(right)) => transform_approx_eq(left, right),
        _ => false,
    }
}

fn transform_approx_eq(left: &Transform, right: &Transform) -> bool {
    (left.translation - right.translation).abs().max_element() <= TRANSFORM_ABS_TOLERANCE
        && (left.scale - right.scale).abs().max_element() <= TRANSFORM_ABS_TOLERANCE
        && (1.0 - left.rotation.dot(right.rotation).abs()).abs() <= TRANSFORM_ABS_TOLERANCE
}

fn require_equal<T>(
    capability: &'static str,
    primary: &T,
    secondary: &T,
) -> GenerationReadResult<()>
where
    T: PartialEq + Serialize,
{
    if primary == secondary {
        Ok(())
    } else {
        Err(mismatch(capability, primary, secondary))
    }
}

fn mismatch(
    capability: &'static str,
    primary: &impl Serialize,
    secondary: &impl Serialize,
) -> GenerationReadError {
    GenerationReadError::ParityMismatch {
        capability,
        detail: format!(
            "primary_hash={} secondary_hash={}",
            hash_serializable(primary),
            hash_serializable(secondary)
        ),
    }
}

fn merge_metrics(
    target: &mut SessionMetricsSnapshot,
    source: SessionMetricsSnapshot,
    prefix: &str,
) {
    merge_metric_map(&mut target.backend_calls, source.backend_calls, prefix);
    merge_metric_map(&mut target.requested_keys, source.requested_keys, prefix);
    merge_metric_map(&mut target.returned_rows, source.returned_rows, prefix);
    merge_metric_map(&mut target.elapsed_micros, source.elapsed_micros, prefix);
}

fn merge_metric_map(
    target: &mut std::collections::BTreeMap<String, u64>,
    source: std::collections::BTreeMap<String, u64>,
    prefix: &str,
) {
    for (name, count) in source {
        target.insert(format!("{prefix}{name}"), count);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::generation_read::{AttributeValue, DataVersion};
    use aios_core::Transform;

    #[derive(Clone)]
    struct FixtureBackend {
        kind: GenerationReadBackendKind,
        element_name: &'static str,
    }

    struct FixtureSession {
        kind: GenerationReadBackendKind,
        element_name: &'static str,
        manifest: Arc<InputVersionManifest>,
    }

    #[async_trait]
    impl GenerationReadBackend for FixtureBackend {
        fn backend_kind(&self) -> GenerationReadBackendKind {
            self.kind
        }

        async fn open_session(
            &self,
            manifest: Arc<InputVersionManifest>,
        ) -> GenerationReadResult<Arc<dyn VersionedReadSession>> {
            Ok(Arc::new(FixtureSession {
                kind: self.kind,
                element_name: self.element_name,
                manifest,
            }))
        }
    }

    impl VersionedReadSession for FixtureSession {
        fn manifest(&self) -> &InputVersionManifest {
            &self.manifest
        }

        fn backend_kind(&self) -> GenerationReadBackendKind {
            self.kind
        }

        fn metrics(&self) -> SessionMetricsSnapshot {
            SessionMetricsSnapshot::default()
        }
    }

    impl FixtureSession {
        fn fixture_element(&self, refno: RefnoEnum) -> ElementSnapshot {
            ElementSnapshot {
                refno,
                dbnum: 1,
                owner: refno,
                noun: "EQUI".to_string(),
                name: self.element_name.to_string(),
                has_children: false,
            }
        }

        fn is_missing(refno: RefnoEnum) -> bool {
            refno == RefnoEnum::from("9/9")
        }
    }

    #[async_trait]
    impl ElementRead for FixtureSession {
        async fn load_elements(
            &self,
            refnos: &[RefnoEnum],
        ) -> GenerationReadResult<BatchLookup<ElementSnapshot>> {
            Ok(BatchLookup::from_found(
                refnos,
                refnos
                    .iter()
                    .copied()
                    .filter(|refno| !Self::is_missing(*refno))
                    .map(|refno| (refno, self.fixture_element(refno))),
            ))
        }

        async fn query_elements(
            &self,
            _query: &ElementQuery,
        ) -> GenerationReadResult<Vec<ElementSnapshot>> {
            Ok(vec![self.fixture_element(RefnoEnum::from("1/1"))])
        }
    }

    #[async_trait]
    impl AttributeRead for FixtureSession {
        async fn load_attribute_sets(
            &self,
            refnos: &[RefnoEnum],
        ) -> GenerationReadResult<BatchLookup<AttributeSet>> {
            Ok(BatchLookup::from_found(
                refnos,
                refnos
                    .iter()
                    .copied()
                    .filter(|refno| !Self::is_missing(*refno))
                    .map(|refno| {
                        (
                            refno,
                            AttributeSet::new(
                                refno,
                                [(
                                    "NAME".to_string(),
                                    AttributeValue::String(self.element_name.to_string()),
                                )]
                                .into_iter()
                                .collect(),
                            ),
                        )
                    }),
            ))
        }
    }

    #[async_trait]
    impl HierarchyRead for FixtureSession {
        async fn load_hierarchy_rows(
            &self,
            _dbnums: &[u32],
        ) -> GenerationReadResult<Vec<HierarchyRow>> {
            Ok(vec![HierarchyRow {
                dbnum: 1,
                parent: RefnoEnum::from("1/1"),
                child: RefnoEnum::from("1/2"),
                ordinal: 0,
            }])
        }
    }

    #[async_trait]
    impl CatalogGraphRead for FixtureSession {
        async fn load_catalog_nodes(
            &self,
            refnos: &[RefnoEnum],
        ) -> GenerationReadResult<BatchLookup<CatalogNode>> {
            Ok(BatchLookup::from_found(
                refnos,
                refnos
                    .iter()
                    .copied()
                    .filter(|refno| !Self::is_missing(*refno))
                    .map(|refno| {
                        (
                            refno,
                            CatalogNode {
                                refno,
                                dbnum: 1,
                                db_type: "CATA".to_string(),
                                noun: "SCOM".to_string(),
                                owner: refno,
                                children: Vec::new(),
                                outbound: Vec::new(),
                            },
                        )
                    }),
            ))
        }
    }

    #[async_trait]
    impl TransformRead for FixtureSession {
        async fn load_transforms(
            &self,
            refnos: &[RefnoEnum],
        ) -> GenerationReadResult<BatchLookup<TransformSnapshot>> {
            Ok(BatchLookup::from_found(
                refnos,
                refnos
                    .iter()
                    .copied()
                    .filter(|refno| !Self::is_missing(*refno))
                    .map(|refno| {
                        (
                            refno,
                            TransformSnapshot {
                                refno,
                                dbnum: 1,
                                local: None,
                                world: Transform::IDENTITY,
                            },
                        )
                    }),
            ))
        }
    }

    fn fixture_manifest() -> Arc<InputVersionManifest> {
        Arc::new(
            InputVersionManifest::new(
                1,
                1,
                [DataVersion {
                    dbnum: 1,
                    sesno: 1,
                    commit_fingerprint: "fixture".to_string(),
                }],
            )
            .expect("manifest"),
        )
    }

    #[tokio::test]
    async fn compare_session_accepts_equal_adapters_and_rejects_semantic_drift() {
        let matching = ComparingVersionedReadBackend::new(
            Arc::new(FixtureBackend {
                kind: GenerationReadBackendKind::Surreal,
                element_name: "same",
            }),
            Arc::new(FixtureBackend {
                kind: GenerationReadBackendKind::DuckLake,
                element_name: "same",
            }),
        )
        .expect("compare backend")
        .open_session(fixture_manifest())
        .await
        .expect("session");
        let requested = [
            RefnoEnum::from("1/1"),
            RefnoEnum::from("9/9"),
            RefnoEnum::from("1/1"),
        ];
        let elements = matching
            .load_elements(&requested)
            .await
            .expect("equal adapters");
        assert_eq!(elements.missing, vec![RefnoEnum::from("9/9")]);
        matching
            .query_elements(&ElementQuery::default())
            .await
            .expect("equal element query");
        matching
            .load_attribute_sets(&requested)
            .await
            .expect("equal attributes");
        matching
            .load_hierarchy_rows(&[1, 1])
            .await
            .expect("equal hierarchy");
        matching
            .load_catalog_nodes(&requested)
            .await
            .expect("equal catalog");
        matching
            .load_transforms(&requested)
            .await
            .expect("equal transforms");

        let drifting = ComparingVersionedReadBackend::new(
            Arc::new(FixtureBackend {
                kind: GenerationReadBackendKind::Surreal,
                element_name: "primary",
            }),
            Arc::new(FixtureBackend {
                kind: GenerationReadBackendKind::DuckLake,
                element_name: "secondary",
            }),
        )
        .expect("compare backend")
        .open_session(fixture_manifest())
        .await
        .expect("session");
        assert!(matches!(
            drifting.load_elements(&[RefnoEnum::from("1/1")]).await,
            Err(GenerationReadError::ParityMismatch {
                capability: "element.load",
                ..
            })
        ));
    }

    #[test]
    fn parity_diagnostic_is_canonical_and_capability_scoped() {
        let primary = vec!["A", "B"];
        let secondary = vec!["A", "C"];
        let error =
            require_equal("fixture.elements", &primary, &secondary).expect_err("must differ");
        match error {
            GenerationReadError::ParityMismatch { capability, detail } => {
                assert_eq!(capability, "fixture.elements");
                assert!(detail.contains("primary_hash="));
                assert!(detail.contains("secondary_hash="));
                assert!(!detail.contains(&format!(
                    "primary_hash={} secondary_hash={}",
                    hash_serializable(&secondary),
                    hash_serializable(&primary)
                )));
            }
            other => panic!("unexpected error: {other}"),
        }
    }

    #[test]
    fn transform_comparison_uses_one_explicit_tolerance() {
        let mut near = Transform::IDENTITY;
        near.translation.x = TRANSFORM_ABS_TOLERANCE * 0.5;
        assert!(transform_approx_eq(&Transform::IDENTITY, &near));

        let mut far = Transform::IDENTITY;
        far.translation.x = TRANSFORM_ABS_TOLERANCE * 2.0;
        assert!(!transform_approx_eq(&Transform::IDENTITY, &far));
    }

    #[test]
    fn compare_metrics_keep_backend_identity() {
        let mut combined = SessionMetricsSnapshot {
            backend_calls: [("element.load".to_string(), 2)].into_iter().collect(),
            ..SessionMetricsSnapshot::default()
        };
        merge_metrics(
            &mut combined,
            SessionMetricsSnapshot {
                backend_calls: [("element.load".to_string(), 3)].into_iter().collect(),
                requested_keys: [("element.load".to_string(), 10)].into_iter().collect(),
                returned_rows: [("element.load".to_string(), 9)].into_iter().collect(),
                elapsed_micros: [("element.load".to_string(), 42)].into_iter().collect(),
            },
            "secondary.",
        );
        assert_eq!(combined.backend_calls["element.load"], 2);
        assert_eq!(combined.backend_calls["secondary.element.load"], 3);
        assert_eq!(combined.requested_keys["secondary.element.load"], 10);
        assert_eq!(combined.returned_rows["secondary.element.load"], 9);
        assert_eq!(combined.elapsed_micros["secondary.element.load"], 42);
    }
}
