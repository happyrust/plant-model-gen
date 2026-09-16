use std::sync::Arc;

use aios_core::RefnoEnum;
use async_trait::async_trait;

use super::error::GenerationReadResult;
use super::types::{
    AttributeSet, BatchLookup, CatalogNode, ElementQuery, ElementSnapshot,
    GenerationReadBackendKind, HierarchyRow, InputVersionManifest, SessionMetricsSnapshot,
    TransformSnapshot,
};

#[async_trait]
pub trait ElementRead: Send + Sync {
    async fn load_elements(
        &self,
        refnos: &[RefnoEnum],
    ) -> GenerationReadResult<BatchLookup<ElementSnapshot>>;

    async fn query_elements(
        &self,
        query: &ElementQuery,
    ) -> GenerationReadResult<Vec<ElementSnapshot>>;
}

#[async_trait]
pub trait AttributeRead: Send + Sync {
    async fn load_attribute_sets(
        &self,
        refnos: &[RefnoEnum],
    ) -> GenerationReadResult<BatchLookup<AttributeSet>>;
}

#[async_trait]
pub trait HierarchyRead: Send + Sync {
    async fn load_hierarchy_rows(&self, dbnums: &[u32]) -> GenerationReadResult<Vec<HierarchyRow>>;
}

#[async_trait]
pub trait CatalogGraphRead: Send + Sync {
    async fn load_catalog_nodes(
        &self,
        refnos: &[RefnoEnum],
    ) -> GenerationReadResult<BatchLookup<CatalogNode>>;
}

#[async_trait]
pub trait TransformRead: Send + Sync {
    async fn load_transforms(
        &self,
        refnos: &[RefnoEnum],
    ) -> GenerationReadResult<BatchLookup<TransformSnapshot>>;
}

pub trait VersionedReadSession:
    ElementRead + AttributeRead + HierarchyRead + CatalogGraphRead + TransformRead + Send + Sync
{
    fn manifest(&self) -> &InputVersionManifest;
    fn backend_kind(&self) -> GenerationReadBackendKind;
    fn metrics(&self) -> SessionMetricsSnapshot;
}

#[async_trait]
pub trait GenerationReadBackend: Send + Sync {
    fn backend_kind(&self) -> GenerationReadBackendKind;

    async fn open_session(
        &self,
        manifest: Arc<InputVersionManifest>,
    ) -> GenerationReadResult<Arc<dyn VersionedReadSession>>;
}
