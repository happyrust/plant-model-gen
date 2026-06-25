use crate::version_management::types::ModelReleaseFile;
use anyhow::Context;
use sha2::{Digest, Sha256};
use std::fs::File;
use std::io::Read;
use std::path::Path;

pub fn sha256_file(path: &Path) -> anyhow::Result<String> {
    let mut file = File::open(path)
        .with_context(|| format!("open file for sha256 failed: {}", path.display()))?;
    let mut hasher = Sha256::new();
    let mut buffer = [0u8; 64 * 1024];

    loop {
        let bytes = file
            .read(&mut buffer)
            .with_context(|| format!("read file for sha256 failed: {}", path.display()))?;
        if bytes == 0 {
            break;
        }
        hasher.update(&buffer[..bytes]);
    }

    Ok(hex::encode(hasher.finalize()))
}

pub fn sha256_bytes(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    hex::encode(hasher.finalize())
}

pub fn package_hash(files: &[ModelReleaseFile]) -> anyhow::Result<String> {
    let mut stable_rows = files
        .iter()
        .map(|file| {
            serde_json::json!({
                "logical_name": file.logical_name,
                "relative_path": file.relative_path,
                "bytes": file.bytes,
                "sha256": file.sha256,
                "rows": file.rows,
                "required": file.required,
            })
        })
        .collect::<Vec<_>>();
    stable_rows.sort_by(|a, b| {
        let a_name = a
            .get("logical_name")
            .and_then(|value| value.as_str())
            .unwrap_or_default();
        let b_name = b
            .get("logical_name")
            .and_then(|value| value.as_str())
            .unwrap_or_default();
        a_name.cmp(b_name)
    });
    let bytes = serde_json::to_vec(&stable_rows).context("serialize package hash payload")?;
    Ok(sha256_bytes(&bytes))
}
