//! DuckLake 权威数据版本库及 SurrealDB 版本化读副本绑定。

pub mod bootstrap;
pub mod replica;
pub mod schema;

#[cfg(feature = "generation-read-ducklake")]
pub mod authority;
#[cfg(feature = "generation-read-ducklake")]
pub mod legacy_bridge;
#[cfg(feature = "generation-read-ducklake")]
pub mod model_unit_commit;
#[cfg(feature = "generation-read-ducklake")]
pub mod parse_staging;

#[cfg(feature = "generation-read-ducklake")]
pub use authority::{
    AuthorityCommit, AuthorityCommitOutcome, AuthorityDbVersion, DbCatalogEntry, DuckLakeAuthority,
    DuckLakeConfig, DuckLakeExtensionConfig, VersionStoreElement,
};
pub use bootstrap::{
    BootstrapOptions, BootstrapReport, BootstrapSource, BootstrapState,
    SurrealCurrentStateBootstrapSource, bootstrap_current_state, resolve_bootstrap_dbnum_sesnos,
};
#[cfg(feature = "generation-read-ducklake")]
pub use bootstrap::{bootstrap_current_state_with_options, bootstrap_state};
#[cfg(feature = "generation-read-ducklake")]
pub use legacy_bridge::{
    LegacyAuthorityPublishReport, LegacyAuthorityPublishRequest, publish_legacy_applied_state,
};
#[cfg(feature = "generation-read-ducklake")]
pub use model_unit_commit::{ModelUnitCommit, ModelUnitCommitOutcome, ModelUnitImpactKind};
#[cfg(feature = "generation-read-ducklake")]
pub use parse_staging::{
    DuckLakeParseStager, ParseStageCounts, ParseStageState, ParseStageVersion, ParseWriteReport,
    ParsedFactBatch, ParsedPlineFact, SealedParseStage, StagedParsePayload,
    StagingTransformFactSource,
};
pub use replica::{ReplicaApplyBatch, ReplicaSnapshotBinding, SurrealReplicaStore};
