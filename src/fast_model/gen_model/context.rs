use crate::options::DbOptionExt;
use std::collections::BTreeMap;
use std::sync::Arc;

use super::config::GenerationContract;
use crate::generation_read::{
    AttributeSet, CatalogNode, CatalogResolver, CatalogResolverConfig, HierarchySnapshot,
    TransformSnapshot, VersionedReadSession,
};

/// Noun 处理上下文，包含所有处理过程需要的配置信息
pub struct GenerationReadContext {
    pub session: Arc<dyn VersionedReadSession>,
    pub hierarchy: Arc<HierarchySnapshot>,
    pub catalog: Arc<CatalogResolver>,
    pub attributes: Arc<BTreeMap<aios_core::RefnoEnum, AttributeSet>>,
    pub catalog_nodes: Arc<BTreeMap<aios_core::RefnoEnum, CatalogNode>>,
    pub transforms: Arc<BTreeMap<aios_core::RefnoEnum, TransformSnapshot>>,
}

impl GenerationReadContext {
    pub async fn load(session: Arc<dyn VersionedReadSession>) -> anyhow::Result<Arc<Self>> {
        let hierarchy =
            HierarchySnapshot::load(Arc::clone(&session), &session.manifest().dbnums()).await?;
        Self::from_hierarchy(session, hierarchy).await
    }

    pub async fn load_for_refnos(
        session: Arc<dyn VersionedReadSession>,
        refnos: &[aios_core::RefnoEnum],
    ) -> anyhow::Result<Arc<Self>> {
        if refnos.is_empty() {
            return Self::load(session).await;
        }
        let hierarchy = HierarchySnapshot::load_for_refnos(Arc::clone(&session), refnos).await?;
        Self::from_hierarchy(session, hierarchy).await
    }

    async fn from_hierarchy(
        session: Arc<dyn VersionedReadSession>,
        hierarchy: HierarchySnapshot,
    ) -> anyhow::Result<Arc<Self>> {
        let hierarchy_refnos = hierarchy.all_refnos();
        let direct_catalog_nodes = session
            .load_catalog_nodes(&hierarchy_refnos)
            .await?
            .require_all("generation.preload.catalog_nodes")?;
        let catalog_seeds = direct_catalog_nodes
            .values()
            .flat_map(|node| node.outbound.iter().map(|edge| edge.target))
            .filter(aios_core::RefnoEnum::is_valid)
            .collect::<Vec<_>>();
        let catalog_resolver =
            CatalogResolver::new(Arc::clone(&session), CatalogResolverConfig::default());
        let catalog_closure = match catalog_resolver.resolve(&catalog_seeds).await {
            #[cfg(all(feature = "sqlite-index", feature = "surreal-save"))]
            Err(crate::generation_read::GenerationReadError::MissingRequiredData {
                capability: "catalog.nodes",
                refnos,
            }) if crate::data_interface::cata_closure::cata_closure_sync_mode()
                == crate::data_interface::cata_closure::CataClosureSyncMode::Manifest =>
            {
                crate::data_interface::cata_closure::ensure_cata_refnos_parsed(
                    &refnos.iter().map(|refno| refno.refno()).collect::<Vec<_>>(),
                )
                .await?;
                CatalogResolver::new(Arc::clone(&session), CatalogResolverConfig::default())
                    .allow_missing_nodes()
                    .resolve(&catalog_seeds)
                    .await?
            }
            result => result?,
        };
        let mut refnos = hierarchy_refnos
            .into_iter()
            .collect::<std::collections::BTreeSet<_>>();
        refnos.extend(catalog_closure.ordered_refnos);
        let refnos = refnos.into_iter().collect::<Vec<_>>();
        let (attributes, catalog_nodes, transforms) = tokio::try_join!(
            session.load_attribute_sets(&refnos),
            session.load_catalog_nodes(&refnos),
            session.load_transforms(&refnos),
        )?;
        // 层级中允许存在只有 PE/UDA、没有基础 ATT 的连接标记；真正消费属性时仍会强校验。
        let attributes = Arc::new(attributes.found);
        let catalog_nodes =
            Arc::new(catalog_nodes.require_all("generation.preload.catalog_nodes")?);
        // 并非每个容器节点都具有 transform；缺失在具体消费点按 required 语义失败。
        let transforms = Arc::new(transforms.found);
        let catalog = CatalogResolver::with_preloaded_nodes(
            Arc::clone(&session),
            CatalogResolverConfig::default(),
            Arc::clone(&catalog_nodes),
        );
        Ok(Arc::new(Self {
            session,
            hierarchy: Arc::new(hierarchy),
            catalog: Arc::new(catalog),
            attributes,
            catalog_nodes,
            transforms,
        }))
    }
}

/// Noun 处理上下文，包含所有处理过程需要的配置信息
#[derive(Clone)]
pub struct NounProcessContext {
    pub db_option: Arc<DbOptionExt>,
    pub batch_size: usize,
    pub batch_concurrency: usize,
    pub generation_read: Arc<GenerationReadContext>,
    pub(crate) generation_contract: Arc<GenerationContract>,
}

impl NounProcessContext {
    /// 创建新的处理上下文
    ///
    /// # Arguments
    /// * `db_option` - 数据库配置
    /// * `batch_size` - 每批次处理的数量
    /// * `batch_concurrency` - 批次处理的并发数（自动限制最小为1）
    pub fn new(
        db_option: Arc<DbOptionExt>,
        generation_read: Arc<GenerationReadContext>,
        generation_contract: Arc<GenerationContract>,
        batch_size: usize,
        batch_concurrency: usize,
    ) -> Self {
        Self {
            db_option,
            batch_size,
            batch_concurrency: batch_concurrency.max(1),
            generation_read,
            generation_contract,
        }
    }
}
