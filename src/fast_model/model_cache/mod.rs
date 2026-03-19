//! [foyer-removal] 兼容模块：model_cache 主流程已移除。
//! 仅保留少量兼容类型/接口（如 cata_resolve_cache）供旧链路复用。

pub mod cata_resolve_cache;
pub mod geom_input_cache;
pub mod mesh;
pub mod query;

/// 模型缓存上下文（桩）
pub struct ModelCacheContext;

impl ModelCacheContext {
    pub async fn try_from_db_option(
        _db_option: &crate::options::DbOptionExt,
    ) -> anyhow::Result<Option<Self>> {
        Ok(None)
    }

    pub fn cache(&self) -> &Self {
        self
    }

    pub fn cache_arc(&self) -> std::sync::Arc<Self> {
        std::sync::Arc::new(ModelCacheContext)
    }

    pub fn insert_from_shape(
        &self,
        _dbnum: u32,
        _shape_insts: &aios_core::geometry::ShapeInstancesData,
    ) {
        // [foyer-removal] 桩实现，不做任何操作
    }

    pub async fn close(&self) -> anyhow::Result<()> {
        Ok(())
    }
}
