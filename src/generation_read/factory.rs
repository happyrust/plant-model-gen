use std::path::Path;
use std::sync::Arc;

use crate::options::{DbOptionExt, GenerationReadBackendMode};
use crate::version_store::SurrealReplicaStore;

use super::compare::ComparingVersionedReadBackend;
use super::error::{GenerationReadError, GenerationReadResult};
use super::surreal::SurrealVersionedReadBackend;
use super::traits::{GenerationReadBackend, VersionedReadSession};
use super::types::InputVersionManifest;

pub async fn open_generation_read_session(
    options: &DbOptionExt,
) -> GenerationReadResult<Arc<dyn VersionedReadSession>> {
    options
        .validate_generation_read_features()
        .map_err(|error| GenerationReadError::BackendQuery {
            backend: options.generation_read_backend.as_str(),
            operation: "config.validate",
            message: error.to_string(),
        })?;

    let manifest = Arc::new(resolve_input_version_manifest(options).await?);
    let surreal: Arc<dyn GenerationReadBackend> = Arc::new(SurrealVersionedReadBackend::new(
        SurrealReplicaStore::default(),
    ));

    match options.generation_read_backend {
        GenerationReadBackendMode::Surreal => surreal.open_session(manifest).await,
        GenerationReadBackendMode::DuckLake => {
            let backend = ducklake_backend(options).await?;
            backend.open_session(manifest).await
        }
        GenerationReadBackendMode::Compare => {
            let ducklake = ducklake_backend(options).await?;
            let compare = ComparingVersionedReadBackend::new(surreal, ducklake)?;
            compare.open_session(manifest).await
        }
    }
}

pub async fn resolve_input_version_manifest(
    options: &DbOptionExt,
) -> GenerationReadResult<InputVersionManifest> {
    if let Some(path) = options.generation_input_manifest.as_deref() {
        let bytes =
            std::fs::read(Path::new(path)).map_err(|error| GenerationReadError::BackendQuery {
                backend: options.generation_read_backend.as_str(),
                operation: "manifest.read",
                message: format!("{}: {error}", Path::new(path).display()),
            })?;
        let manifest: InputVersionManifest =
            serde_json::from_slice(&bytes).map_err(|error| GenerationReadError::BackendQuery {
                backend: options.generation_read_backend.as_str(),
                operation: "manifest.decode",
                message: error.to_string(),
            })?;
        manifest.verify_hash()?;
        return Ok(manifest);
    }

    // DuckLake 是唯一权威版本源。即使选择 Surreal adapter，也必须先解析
    // DuckLake latest（或使用调用方显式传入的权威 manifest），随后由
    // Surreal open_session 校验水位/binding；不得把副本自身 latest 当权威 latest。
    resolve_ducklake_latest_manifest(options).await
}

#[cfg(feature = "generation-read-ducklake")]
async fn ducklake_backend(
    options: &DbOptionExt,
) -> GenerationReadResult<Arc<dyn GenerationReadBackend>> {
    use super::ducklake::DuckLakeVersionedReadBackend;
    use crate::version_store::DuckLakeAuthority;

    let config = options.ducklake_config();
    let pool_size = options.duckdb_pool_size;
    let authority = tokio::task::spawn_blocking(move || DuckLakeAuthority::open(config))
        .await
        .map_err(|error| GenerationReadError::BackendQuery {
            backend: "ducklake",
            operation: "authority.open.join",
            message: error.to_string(),
        })?
        .map_err(|error| GenerationReadError::BackendQuery {
            backend: "ducklake",
            operation: "authority.open",
            message: error.to_string(),
        })?;
    let backend = DuckLakeVersionedReadBackend::new(authority, pool_size).map_err(|error| {
        GenerationReadError::BackendQuery {
            backend: "ducklake",
            operation: "backend.create",
            message: error.to_string(),
        }
    })?;
    Ok(Arc::new(backend))
}

#[cfg(not(feature = "generation-read-ducklake"))]
async fn ducklake_backend(
    _options: &DbOptionExt,
) -> GenerationReadResult<Arc<dyn GenerationReadBackend>> {
    Err(GenerationReadError::BackendQuery {
        backend: "ducklake",
        operation: "backend.create",
        message: "binary missing generation-read-ducklake feature".to_string(),
    })
}

#[cfg(feature = "generation-read-ducklake")]
async fn resolve_ducklake_latest_manifest(
    options: &DbOptionExt,
) -> GenerationReadResult<InputVersionManifest> {
    use crate::version_store::DuckLakeAuthority;

    let config = options.ducklake_config();
    tokio::task::spawn_blocking(move || {
        let authority = DuckLakeAuthority::open(config)?;
        let snapshot_id = authority.latest_snapshot_id()?;
        authority.read_manifest(snapshot_id)
    })
    .await
    .map_err(|error| GenerationReadError::BackendQuery {
        backend: "ducklake",
        operation: "manifest.latest.join",
        message: error.to_string(),
    })?
    .map_err(|error| GenerationReadError::BackendQuery {
        backend: "ducklake",
        operation: "manifest.latest",
        message: error.to_string(),
    })
}

#[cfg(not(feature = "generation-read-ducklake"))]
async fn resolve_ducklake_latest_manifest(
    _options: &DbOptionExt,
) -> GenerationReadResult<InputVersionManifest> {
    Err(GenerationReadError::BackendQuery {
        backend: "ducklake",
        operation: "manifest.latest",
        message: "binary missing generation-read-ducklake feature".to_string(),
    })
}
