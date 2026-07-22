//! Immutable minimum-delivery-unit model commits stored with the model tables.

use std::path::{Component, Path};
use std::str::FromStr;

use aios_core::project_primary_db;
use anyhow::Context;
use serde::{Deserialize, Serialize};
use surrealdb::types::SurrealValue;

const TABLE_NAME: &str = "model_unit_commit";
const PAYLOAD_VERSION: &str = "model-unit-commit:v1";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ModelUnitImpactKind {
    Mesh,
    Placement,
    Delivery,
    Noop,
    Tombstone,
}

impl ModelUnitImpactKind {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Mesh => "mesh",
            Self::Placement => "placement",
            Self::Delivery => "delivery",
            Self::Noop => "noop",
            Self::Tombstone => "tombstone",
        }
    }
}

impl FromStr for ModelUnitImpactKind {
    type Err = anyhow::Error;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value.trim().to_ascii_lowercase().as_str() {
            "mesh" => Ok(Self::Mesh),
            "placement" => Ok(Self::Placement),
            "delivery" => Ok(Self::Delivery),
            "noop" => Ok(Self::Noop),
            "tombstone" => Ok(Self::Tombstone),
            other => anyhow::bail!("unsupported model unit impact kind: {other}"),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModelUnitCommit {
    pub dbnum: u32,
    pub unit_refno: String,
    pub unit_noun: String,
    pub sesno: u32,
    pub impact_kind: ModelUnitImpactKind,
    /// No-op commits reuse an earlier artifact; other commits normally match `sesno`.
    pub artifact_sesno: u32,
    pub project_name: String,
    /// Path relative to `output/<project>/`; empty only for tombstones.
    pub manifest_path: String,
    /// SHA-256 of the referenced manifest; empty only for tombstones.
    pub artifact_hash: String,
    pub generated_at: String,
}

impl ModelUnitCommit {
    pub fn manifest_url(&self) -> Option<String> {
        (self.impact_kind != ModelUnitImpactKind::Tombstone).then(|| {
            format!(
                "/files/output/{}/{}",
                urlencoding::encode(&self.project_name),
                self.manifest_path.replace('\\', "/")
            )
        })
    }

    pub fn validate(&self) -> anyhow::Result<()> {
        anyhow::ensure!(self.dbnum > 0, "dbnum must be non-zero");
        anyhow::ensure!(self.sesno > 0, "sesno must be non-zero");
        anyhow::ensure!(self.artifact_sesno > 0, "artifact_sesno must be non-zero");
        anyhow::ensure!(
            self.artifact_sesno <= self.sesno,
            "artifact_sesno cannot be newer than sesno"
        );
        let mut refno_parts = self.unit_refno.split('_');
        let ref0 = refno_parts.next().unwrap_or_default();
        let ref1 = refno_parts.next().unwrap_or_default();
        anyhow::ensure!(
            !ref0.is_empty()
                && !ref1.is_empty()
                && refno_parts.next().is_none()
                && ref0.parse::<u32>().is_ok_and(|value| value > 0)
                && ref1.parse::<u32>().is_ok(),
            "unit_refno must be a normalized underscore refno"
        );
        anyhow::ensure!(
            crate::version_management::model_impact::is_delivery_unit_root_noun(&self.unit_noun),
            "unit_noun must be BRAN/HANG/EQUI/WALL/FLOOR"
        );
        anyhow::ensure!(
            !self.project_name.is_empty(),
            "project_name must not be empty"
        );
        anyhow::ensure!(
            !self.project_name.contains(['/', '\\']),
            "project_name must not contain path separators"
        );
        anyhow::ensure!(
            !self.generated_at.is_empty(),
            "generated_at must not be empty"
        );

        if self.impact_kind == ModelUnitImpactKind::Tombstone {
            anyhow::ensure!(
                self.manifest_path.is_empty(),
                "tombstone must not reference a manifest"
            );
            anyhow::ensure!(
                self.artifact_hash.is_empty(),
                "tombstone must not reference an artifact hash"
            );
        } else {
            validate_relative_manifest_path(&self.manifest_path)?;
            anyhow::ensure!(
                self.artifact_hash.len() == 64
                    && self
                        .artifact_hash
                        .bytes()
                        .all(|byte| byte.is_ascii_hexdigit()),
                "artifact_hash must be a SHA-256 hex string"
            );
        }
        if self.impact_kind == ModelUnitImpactKind::Noop {
            anyhow::ensure!(
                self.artifact_sesno < self.sesno,
                "no-op commits must reuse an earlier artifact_sesno"
            );
        }
        Ok(())
    }

    fn normalize(mut self) -> anyhow::Result<Self> {
        self.unit_refno = self.unit_refno.trim().replace('/', "_");
        self.unit_noun = self.unit_noun.trim().to_ascii_uppercase();
        self.project_name = self.project_name.trim().to_string();
        self.manifest_path = self.manifest_path.trim().replace('\\', "/");
        self.artifact_hash = self.artifact_hash.trim().to_ascii_lowercase();
        self.generated_at = self.generated_at.trim().to_string();
        self.validate()?;
        Ok(self)
    }

    fn same_payload(&self, other: &Self) -> bool {
        self.dbnum == other.dbnum
            && self.unit_refno == other.unit_refno
            && self.unit_noun == other.unit_noun
            && self.sesno == other.sesno
            && self.impact_kind == other.impact_kind
            && self.artifact_sesno == other.artifact_sesno
            && self.project_name == other.project_name
            && self.manifest_path == other.manifest_path
            && self.artifact_hash == other.artifact_hash
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModelUnitCommitOutcome {
    pub commit: ModelUnitCommit,
    pub idempotent: bool,
}

#[derive(Debug, Clone, Deserialize, SurrealValue)]
struct StoredCommitRow {
    payload_hash: String,
    payload_json: String,
}

pub async fn ensure_model_unit_commit_schema() -> anyhow::Result<()> {
    let sql = r#"
DEFINE TABLE IF NOT EXISTS model_unit_commit SCHEMAFULL
    PERMISSIONS FOR select FULL FOR create FULL FOR update NONE FOR delete NONE;
DEFINE FIELD IF NOT EXISTS dbnum ON TABLE model_unit_commit TYPE int;
DEFINE FIELD IF NOT EXISTS unit_refno ON TABLE model_unit_commit TYPE string;
DEFINE FIELD IF NOT EXISTS unit_noun ON TABLE model_unit_commit TYPE string;
DEFINE FIELD IF NOT EXISTS sesno ON TABLE model_unit_commit TYPE int;
DEFINE FIELD IF NOT EXISTS impact_kind ON TABLE model_unit_commit TYPE string
    ASSERT $value IN ['mesh', 'placement', 'delivery', 'noop', 'tombstone'];
DEFINE FIELD IF NOT EXISTS artifact_sesno ON TABLE model_unit_commit TYPE int;
DEFINE FIELD IF NOT EXISTS project_name ON TABLE model_unit_commit TYPE string;
DEFINE FIELD IF NOT EXISTS manifest_path ON TABLE model_unit_commit TYPE string;
DEFINE FIELD IF NOT EXISTS artifact_hash ON TABLE model_unit_commit TYPE string;
DEFINE FIELD IF NOT EXISTS generated_at ON TABLE model_unit_commit TYPE string;
DEFINE FIELD IF NOT EXISTS payload_hash ON TABLE model_unit_commit TYPE string;
DEFINE FIELD IF NOT EXISTS payload_json ON TABLE model_unit_commit TYPE string;
DEFINE FIELD IF NOT EXISTS created_at ON TABLE model_unit_commit TYPE datetime DEFAULT time::now();
DEFINE INDEX IF NOT EXISTS idx_model_unit_commit_identity ON TABLE model_unit_commit
    FIELDS dbnum, unit_refno, sesno UNIQUE;
DEFINE INDEX IF NOT EXISTS idx_model_unit_commit_list ON TABLE model_unit_commit
    FIELDS dbnum, unit_refno, sesno;
"#;
    project_primary_db()
        .query(sql)
        .await
        .context("define model_unit_commit schema")?
        .check()
        .context("check model_unit_commit schema statements")?;
    Ok(())
}

pub async fn commit_model_unit(commit: ModelUnitCommit) -> anyhow::Result<ModelUnitCommitOutcome> {
    ensure_model_unit_commit_schema().await?;
    let commit = commit.normalize()?;
    if let Some(existing) =
        model_unit_commit(commit.dbnum, &commit.unit_refno, commit.sesno).await?
    {
        anyhow::ensure!(
            existing.same_payload(&commit),
            "model unit commit identity already exists with different content: ({}, {}, {})",
            commit.dbnum,
            commit.unit_refno,
            commit.sesno
        );
        return Ok(ModelUnitCommitOutcome {
            commit: existing,
            idempotent: true,
        });
    }

    let payload_json = serde_json::to_string(&commit)?;
    let payload_hash = payload_hash(&commit);
    let record_id = record_id(commit.dbnum, &commit.unit_refno, commit.sesno);
    let sql = format!(
        "CREATE ONLY {record_id} SET dbnum = $dbnum, unit_refno = $unit_refno, \
         unit_noun = $unit_noun, sesno = $sesno, impact_kind = $impact_kind, \
         artifact_sesno = $artifact_sesno, project_name = $project_name, \
         manifest_path = $manifest_path, artifact_hash = $artifact_hash, \
         generated_at = $generated_at, payload_hash = $payload_hash, \
         payload_json = $payload_json, created_at = time::now();"
    );
    let create_result: anyhow::Result<()> = async {
        project_primary_db()
            .query(sql)
            .bind(("dbnum", commit.dbnum))
            .bind(("unit_refno", commit.unit_refno.clone()))
            .bind(("unit_noun", commit.unit_noun.clone()))
            .bind(("sesno", commit.sesno))
            .bind(("impact_kind", commit.impact_kind.as_str()))
            .bind(("artifact_sesno", commit.artifact_sesno))
            .bind(("project_name", commit.project_name.clone()))
            .bind(("manifest_path", commit.manifest_path.clone()))
            .bind(("artifact_hash", commit.artifact_hash.clone()))
            .bind(("generated_at", commit.generated_at.clone()))
            .bind(("payload_hash", payload_hash))
            .bind(("payload_json", payload_json))
            .await
            .context("append model_unit_commit")?
            .check()
            .context("check model_unit_commit append")?;
        Ok(())
    }
    .await;

    match create_result {
        Ok(()) => Ok(ModelUnitCommitOutcome {
            commit,
            idempotent: false,
        }),
        Err(error) => {
            if let Some(existing) =
                model_unit_commit(commit.dbnum, &commit.unit_refno, commit.sesno).await?
            {
                anyhow::ensure!(
                    existing.same_payload(&commit),
                    "model unit commit identity already exists with different content: ({}, {}, {})",
                    commit.dbnum,
                    commit.unit_refno,
                    commit.sesno
                );
                return Ok(ModelUnitCommitOutcome {
                    commit: existing,
                    idempotent: true,
                });
            }
            Err(error)
        }
    }
}

pub async fn model_unit_commit(
    dbnum: u32,
    unit_refno: &str,
    sesno: u32,
) -> anyhow::Result<Option<ModelUnitCommit>> {
    ensure_model_unit_commit_schema().await?;
    let record_id = record_id(dbnum, &normalize_refno(unit_refno), sesno);
    load_rows(format!(
        "SELECT payload_hash, payload_json FROM {record_id};"
    ))
    .await
    .map(|mut rows| rows.pop())
}

pub async fn latest_model_unit_commit(
    dbnum: u32,
    unit_refno: &str,
) -> anyhow::Result<Option<ModelUnitCommit>> {
    ensure_model_unit_commit_schema().await?;
    let unit_refno = normalize_refno(unit_refno);
    let mut response = project_primary_db()
        .query(
            "SELECT sesno, payload_hash, payload_json FROM model_unit_commit \
             WHERE dbnum = $dbnum AND unit_refno = $unit_refno \
             ORDER BY sesno DESC LIMIT 1;",
        )
        .bind(("dbnum", dbnum))
        .bind(("unit_refno", unit_refno))
        .await
        .context("query latest model_unit_commit")?
        .check()
        .context("check latest model_unit_commit query")?;
    decode_rows(
        response
            .take(0)
            .context("decode latest model_unit_commit")?,
    )
    .map(|mut rows| rows.pop())
}

pub async fn list_model_unit_commits(
    dbnum: u32,
    unit_refno: &str,
) -> anyhow::Result<Vec<ModelUnitCommit>> {
    ensure_model_unit_commit_schema().await?;
    let unit_refno = normalize_refno(unit_refno);
    let mut response = project_primary_db()
        .query(
            "SELECT sesno, payload_hash, payload_json FROM model_unit_commit \
             WHERE dbnum = $dbnum AND unit_refno = $unit_refno ORDER BY sesno DESC;",
        )
        .bind(("dbnum", dbnum))
        .bind(("unit_refno", unit_refno))
        .await
        .context("list model_unit_commit rows")?
        .check()
        .context("check model_unit_commit list query")?;
    decode_rows(response.take(0).context("decode model_unit_commit list")?)
}

async fn load_rows(sql: String) -> anyhow::Result<Vec<ModelUnitCommit>> {
    let mut response = project_primary_db()
        .query(sql)
        .await
        .context("query model_unit_commit")?
        .check()
        .context("check model_unit_commit query")?;
    decode_rows(response.take(0).context("decode model_unit_commit rows")?)
}

fn decode_rows(rows: Vec<StoredCommitRow>) -> anyhow::Result<Vec<ModelUnitCommit>> {
    rows.into_iter()
        .map(|row| {
            let commit: ModelUnitCommit = serde_json::from_str(&row.payload_json)
                .context("decode model_unit_commit payload_json")?;
            anyhow::ensure!(
                payload_hash(&commit) == row.payload_hash,
                "corrupt model_unit_commit payload hash: ({}, {}, {})",
                commit.dbnum,
                commit.unit_refno,
                commit.sesno
            );
            commit.validate()?;
            Ok(commit)
        })
        .collect()
}

fn validate_relative_manifest_path(value: &str) -> anyhow::Result<()> {
    anyhow::ensure!(!value.is_empty(), "manifest_path must not be empty");
    let path = Path::new(value);
    anyhow::ensure!(!path.is_absolute(), "manifest_path must be relative");
    anyhow::ensure!(
        path.components()
            .all(|component| matches!(component, Component::Normal(_))),
        "manifest_path must not escape the project output directory"
    );
    anyhow::ensure!(
        value.replace('\\', "/").ends_with("/manifest.json"),
        "manifest_path must point to manifest.json"
    );
    Ok(())
}

fn normalize_refno(value: &str) -> String {
    value.trim().replace('/', "_")
}

fn payload_hash(commit: &ModelUnitCommit) -> String {
    let payload = serde_json::to_vec(&(
        PAYLOAD_VERSION,
        commit.dbnum,
        &commit.unit_refno,
        &commit.unit_noun,
        commit.sesno,
        commit.impact_kind,
        commit.artifact_sesno,
        &commit.project_name,
        &commit.manifest_path,
        &commit.artifact_hash,
    ))
    .expect("serializing model unit commit identity cannot fail");
    crate::version_management::hashing::sha256_bytes(&payload)
}

fn record_id(dbnum: u32, unit_refno: &str, sesno: u32) -> String {
    let mut bytes = Vec::with_capacity(PAYLOAD_VERSION.len() + unit_refno.len() + 24);
    bytes.extend_from_slice(PAYLOAD_VERSION.as_bytes());
    bytes.extend_from_slice(&dbnum.to_le_bytes());
    bytes.extend_from_slice(unit_refno.as_bytes());
    bytes.extend_from_slice(&sesno.to_le_bytes());
    let hash = crate::version_management::hashing::sha256_bytes(&bytes);
    format!("{TABLE_NAME}:⟨{hash}⟩")
}
