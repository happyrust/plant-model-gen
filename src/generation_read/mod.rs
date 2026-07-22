//! 模型生成输入的版本化、存储无关读取契约。
//!
//! SQL 只能出现在具体 adapter 中；生成算法只依赖本模块公开的领域能力。

mod catalog;
mod error;
mod factory;
mod hierarchy;
mod surreal;
mod traits;
mod types;

pub use catalog::{CatalogClosure, CatalogResolver, CatalogResolverConfig};
pub use error::{GenerationReadError, GenerationReadResult};
pub use factory::{open_generation_read_session, resolve_input_version_manifest};
pub use hierarchy::{HierarchyNode, HierarchyQuery, HierarchySnapshot};
pub use surreal::{SurrealVersionedReadBackend, SurrealVersionedReadSession};
pub use traits::{
    AttributeRead, CatalogGraphRead, ElementRead, GenerationReadBackend, HierarchyRead,
    TransformRead, VersionedReadSession,
};
pub(crate) use types::hash_serializable;
pub use types::{
    AttributeReference, AttributeSet, AttributeValue, BatchLookup, CatalogNode, DataVersion,
    ElementQuery, ElementSnapshot, GenerationReadBackendKind, HierarchyRow, InputVersionManifest,
    SessionMetricsSnapshot, TransformSnapshot, decode_attribute_set_payload,
    encode_attribute_set_payload,
};

#[cfg(test)]
mod boundary_tests {
    use std::path::Path;

    #[test]
    fn active_generation_pipeline_has_no_global_input_reads() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("src/fast_model/gen_model");
        let files = [
            "orchestrator.rs",
            "gen_pipeline.rs",
            "loop_processor.rs",
            "loop_model.rs",
            "prim_processor.rs",
            "prim_model.rs",
            "cate_processor.rs",
            "cata_model.rs",
        ];
        let forbidden = [
            "aios_core::get_named_attmap(",
            "aios_core::query_single_by_paths(",
            "transform_cache::get_world_transform_cache_first",
            "HierView::load(",
            "query_provider::",
            "project_primary_db",
            ".query_response(",
            "\"SELECT ",
            "\"INSERT ",
            "\"UPDATE ",
            "\"DELETE ",
            "\"RELATE ",
        ];
        for file in files {
            let path = root.join(file);
            let source = std::fs::read_to_string(&path)
                .unwrap_or_else(|error| panic!("failed to read {}: {error}", path.display()));
            for pattern in forbidden {
                assert!(
                    !source.contains(pattern),
                    "{} contains forbidden global input read `{pattern}`",
                    path.display()
                );
            }
        }
    }
}
