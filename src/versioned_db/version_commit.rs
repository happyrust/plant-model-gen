use std::collections::BTreeMap;
use std::future::Future;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use aios_core::{SurrealQueryExt, project_primary_db};
use anyhow::anyhow;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use surrealdb::types::{Datetime, SurrealValue};
use thiserror::Error;

const DEFAULT_LEASE_SECS: u64 = 15 * 60;
static OWNER_SEQUENCE: AtomicU64 = AtomicU64::new(1);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VersionCommitSource {
    IncrementalBaseline,
    Incremental,
}

impl VersionCommitSource {
    fn as_str(self) -> &'static str {
        match self {
            Self::IncrementalBaseline => "incremental_baseline",
            Self::Incremental => "incremental",
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct VersionCommitCounts {
    pub pe_rows: usize,
    pub att_rows: usize,
    pub uda_rows: usize,
    pub delete_count: usize,
    pub dbnum_info_updates: usize,
    /// specs/023：本批写入的 pe_owner 边行数（INSERT RELATION 行；serde default 兼容旧记录）
    #[serde(default)]
    pub pe_owner_rows: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VersionCommitRequest {
    pub dbnum: u32,
    pub from_sesno: u32,
    pub to_sesno: u32,
    pub source: VersionCommitSource,
    pub fingerprint: String,
    #[serde(default)]
    pub source_hash: Option<String>,
    #[serde(default)]
    pub expected_counts: Option<VersionCommitCounts>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VersionCommitOutcome {
    pub dbnum: u32,
    pub from_sesno: u32,
    pub to_sesno: u32,
    pub source: VersionCommitSource,
    pub fingerprint: String,
    pub anchored_at: String,
    pub counts: VersionCommitCounts,
    pub idempotent: bool,
    pub recovered: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelGenAnchor {
    pub dbnum: u32,
    pub sesno: u32,
    pub source: String,
    pub anchored_at: String,
    pub note: String,
}

#[derive(Debug, Deserialize, SurrealValue)]
struct CurrentSesnoRow {
    dbnum: i64,
    #[serde(default)]
    sesno: Option<i64>,
}

#[derive(Debug, Error)]
pub enum VersionCommitError {
    #[error("version commit storage error: {0}")]
    Storage(#[source] anyhow::Error),
    #[error("dbnum={dbnum} is already held by another version commit")]
    LeaseBusy { dbnum: u32 },
    #[error(
        "dbnum={dbnum} has pending version commit sesno={pending_sesno}; recover it before committing sesno={requested_sesno}"
    )]
    PendingCommit {
        dbnum: u32,
        pending_sesno: u32,
        requested_sesno: u32,
    },
    #[error(
        "dbnum={dbnum} incremental range {requested_from}..={requested_to} does not connect to committed watermark {watermark}（增量区间与 Committed Watermark 不衔接，禁止带洞锚定；请从 watermark 续传或对该 dbnum 全量重灌）"
    )]
    ContinuityGap {
        dbnum: u32,
        watermark: u32,
        requested_from: u32,
        requested_to: u32,
    },
    #[error(
        "immutable anchor conflict for dbnum={dbnum} sesno={sesno}: existing fingerprint={existing}, requested fingerprint={requested}"
    )]
    FingerprintConflict {
        dbnum: u32,
        sesno: u32,
        existing: String,
        requested: String,
    },
    #[error("legacy anchor for dbnum={dbnum} sesno={sesno} has no fingerprint and is read-only")]
    LegacyAnchor { dbnum: u32, sesno: u32 },
    #[error(
        "version commit count mismatch for dbnum={dbnum} sesno={sesno}: expected={expected:?}, actual={actual:?}"
    )]
    CountMismatch {
        dbnum: u32,
        sesno: u32,
        expected: VersionCommitCounts,
        actual: VersionCommitCounts,
    },
    #[error("version commit apply failed for dbnum={dbnum} sesno={sesno}: {detail}")]
    ApplyFailed {
        dbnum: u32,
        sesno: u32,
        detail: String,
    },
    #[error(
        "no matching commit_pending exists for recovery dbnum={dbnum} sesno={sesno} fingerprint={fingerprint}"
    )]
    RecoveryNotFound {
        dbnum: u32,
        sesno: u32,
        fingerprint: String,
    },
}

type CommitResult<T> = std::result::Result<T, VersionCommitError>;

#[derive(Debug, Deserialize, SurrealValue)]
struct ExistingAnchor {
    #[serde(default)]
    fingerprint: Option<String>,
    anchored_at: String,
    #[serde(default)]
    pe_rows: Option<i64>,
    #[serde(default)]
    att_rows: Option<i64>,
    #[serde(default)]
    uda_rows: Option<i64>,
    #[serde(default)]
    delete_count: Option<i64>,
    #[serde(default)]
    dbnum_info_updates: Option<i64>,
    #[serde(default)]
    pe_owner_rows: Option<i64>,
}

#[derive(Debug, Deserialize, SurrealValue)]
struct PendingRow {
    to_sesno: i64,
}

#[derive(Debug, Deserialize, SurrealValue)]
struct RecoveryRow {
    fingerprint: String,
    status: String,
}

pub fn compute_commit_fingerprint<'a>(
    dbnum: u32,
    from_sesno: u32,
    to_sesno: u32,
    source: VersionCommitSource,
    source_hash: Option<&str>,
    normalized_operations: impl IntoIterator<Item = &'a str>,
) -> String {
    let mut hasher = Sha256::new();
    hasher.update(b"version-commit-v1\0");
    hasher.update(dbnum.to_le_bytes());
    hasher.update(from_sesno.to_le_bytes());
    hasher.update(to_sesno.to_le_bytes());
    hasher.update(source.as_str().as_bytes());
    hasher.update([0]);
    hasher.update(source_hash.unwrap_or_default().as_bytes());
    hasher.update([0]);
    for operation in normalized_operations {
        let normalized = operation.split_whitespace().collect::<Vec<_>>().join(" ");
        hasher.update((normalized.len() as u64).to_le_bytes());
        hasher.update(normalized.as_bytes());
    }
    hex::encode(hasher.finalize())
}

pub async fn commit_version<F, Fut>(
    request: VersionCommitRequest,
    apply: F,
) -> CommitResult<VersionCommitOutcome>
where
    F: FnOnce() -> Fut,
    Fut: Future<Output = anyhow::Result<VersionCommitCounts>>,
{
    commit_version_inner(request, false, apply).await
}

pub async fn recover_version_commit<F, Fut>(
    request: VersionCommitRequest,
    apply: F,
) -> CommitResult<VersionCommitOutcome>
where
    F: FnOnce() -> Fut,
    Fut: Future<Output = anyhow::Result<VersionCommitCounts>>,
{
    commit_version_inner(request, true, apply).await
}

async fn commit_version_inner<F, Fut>(
    request: VersionCommitRequest,
    recovered: bool,
    apply: F,
) -> CommitResult<VersionCommitOutcome>
where
    F: FnOnce() -> Fut,
    Fut: Future<Output = anyhow::Result<VersionCommitCounts>>,
{
    validate_request(&request)?;
    ensure_version_commit_schema().await?;

    if let Some(outcome) = existing_idempotent_outcome(&request, recovered).await? {
        return Ok(outcome);
    }

    let lease = acquire_dbnum_lease(request.dbnum, DEFAULT_LEASE_SECS).await?;
    let result = commit_while_leased(&request, recovered, apply).await;
    if let Err(error) = release_dbnum_lease(&lease).await {
        log::warn!(
            "version commit lease release failed(dbnum={} owner={}): {}",
            lease.dbnum,
            lease.owner,
            error
        );
    }
    result
}

fn validate_request(request: &VersionCommitRequest) -> CommitResult<()> {
    if request.source != VersionCommitSource::Incremental {
        return Err(VersionCommitError::Storage(anyhow!(
            "incremental_baseline is reserved for the internal pre-apply handshake"
        )));
    }
    if request.dbnum == 0 {
        return Err(VersionCommitError::Storage(anyhow!(
            "dbnum must be non-zero"
        )));
    }
    if request.to_sesno == 0 || request.to_sesno < request.from_sesno {
        return Err(VersionCommitError::Storage(anyhow!(
            "invalid sesno range {}..={}",
            request.from_sesno,
            request.to_sesno
        )));
    }
    if request.fingerprint.trim().is_empty() {
        return Err(VersionCommitError::Storage(anyhow!(
            "commit fingerprint must not be empty"
        )));
    }
    Ok(())
}

async fn commit_while_leased<F, Fut>(
    request: &VersionCommitRequest,
    recovered: bool,
    apply: F,
) -> CommitResult<VersionCommitOutcome>
where
    F: FnOnce() -> Fut,
    Fut: Future<Output = anyhow::Result<VersionCommitCounts>>,
{
    if let Some(outcome) = existing_idempotent_outcome(request, recovered).await? {
        return Ok(outcome);
    }
    if recovered {
        require_matching_pending(request).await?;
    } else {
        reject_continuity_gap(request).await?;
    }
    reject_pending_commit(request, recovered).await?;
    if !recovered {
        ensure_incremental_baseline_before_apply(request).await?;
    }
    mark_commit_preparing(request).await?;

    let counts = match apply().await {
        Ok(counts) => counts,
        Err(error) => {
            let detail = error.to_string();
            mark_commit_pending(request, &detail).await?;
            return Err(VersionCommitError::ApplyFailed {
                dbnum: request.dbnum,
                sesno: request.to_sesno,
                detail,
            });
        }
    };

    if let Some(expected) = request.expected_counts.as_ref()
        && expected != &counts
    {
        let detail = format!("expected={expected:?}, actual={counts:?}");
        mark_commit_pending(request, &detail).await?;
        return Err(VersionCommitError::CountMismatch {
            dbnum: request.dbnum,
            sesno: request.to_sesno,
            expected: expected.clone(),
            actual: counts,
        });
    }

    if let Err(error) = publish_authority_after_apply(request).await {
        mark_commit_pending(request, &error.to_string()).await?;
        return Err(VersionCommitError::Storage(error));
    }

    let anchored_at = match create_immutable_anchor(request, &counts).await {
        Ok(value) => value,
        Err(error) => {
            mark_commit_pending(request, &error.to_string()).await?;
            return Err(error);
        }
    };
    mark_commit_committed(request, &counts, &anchored_at).await?;

    Ok(VersionCommitOutcome {
        dbnum: request.dbnum,
        from_sesno: request.from_sesno,
        to_sesno: request.to_sesno,
        source: request.source,
        fingerprint: request.fingerprint.clone(),
        anchored_at,
        counts,
        idempotent: false,
        recovered,
    })
}

// specs/027（ADR-0007）：DuckLake 权威链退役后，apply 后不再向外部权威发布，
// Surreal MVCC + sesno 锚点即版本单源。
async fn publish_authority_after_apply(_request: &VersionCommitRequest) -> anyhow::Result<()> {
    Ok(())
}

pub async fn ensure_version_commit_schema() -> CommitResult<()> {
    super::database::ensure_sesno_version_anchor_schema()
        .await
        .map_err(VersionCommitError::Storage)?;
    let sql = r#"
DEFINE TABLE IF NOT EXISTS version_commit_state SCHEMAFULL;
DEFINE FIELD IF NOT EXISTS dbnum ON TABLE version_commit_state TYPE int;
DEFINE FIELD IF NOT EXISTS from_sesno ON TABLE version_commit_state TYPE int;
DEFINE FIELD IF NOT EXISTS to_sesno ON TABLE version_commit_state TYPE int;
DEFINE FIELD OVERWRITE source ON TABLE version_commit_state TYPE string ASSERT $value IN ['full', 'incremental_baseline', 'incremental'];
DEFINE FIELD IF NOT EXISTS fingerprint ON TABLE version_commit_state TYPE string;
DEFINE FIELD IF NOT EXISTS source_hash ON TABLE version_commit_state TYPE option<string>;
DEFINE FIELD IF NOT EXISTS status ON TABLE version_commit_state TYPE string ASSERT $value IN ['preparing', 'pending', 'committed'];
DEFINE FIELD IF NOT EXISTS pe_rows ON TABLE version_commit_state TYPE int;
DEFINE FIELD IF NOT EXISTS att_rows ON TABLE version_commit_state TYPE int;
DEFINE FIELD IF NOT EXISTS uda_rows ON TABLE version_commit_state TYPE int;
DEFINE FIELD IF NOT EXISTS delete_count ON TABLE version_commit_state TYPE int;
DEFINE FIELD IF NOT EXISTS dbnum_info_updates ON TABLE version_commit_state TYPE int;
DEFINE FIELD IF NOT EXISTS pe_owner_rows ON TABLE version_commit_state TYPE int DEFAULT 0;
DEFINE FIELD IF NOT EXISTS anchored_at ON TABLE version_commit_state TYPE option<datetime>;
DEFINE FIELD IF NOT EXISTS last_error ON TABLE version_commit_state TYPE option<string>;
DEFINE FIELD IF NOT EXISTS created_at ON TABLE version_commit_state TYPE datetime DEFAULT time::now();
DEFINE FIELD IF NOT EXISTS updated_at ON TABLE version_commit_state TYPE datetime DEFAULT time::now();
DEFINE INDEX IF NOT EXISTS idx_version_commit_state_dbnum_status ON TABLE version_commit_state FIELDS dbnum, status;

DEFINE TABLE IF NOT EXISTS version_commit_lease SCHEMAFULL;
DEFINE FIELD IF NOT EXISTS dbnum ON TABLE version_commit_lease TYPE int;
DEFINE FIELD IF NOT EXISTS owner ON TABLE version_commit_lease TYPE string;
DEFINE FIELD IF NOT EXISTS expires_at ON TABLE version_commit_lease TYPE datetime;

DEFINE FIELD IF NOT EXISTS from_sesno ON TABLE sesno_version_anchor TYPE option<int>;
DEFINE FIELD IF NOT EXISTS fingerprint ON TABLE sesno_version_anchor TYPE option<string>;
DEFINE FIELD IF NOT EXISTS source_hash ON TABLE sesno_version_anchor TYPE option<string>;
DEFINE FIELD IF NOT EXISTS pe_rows ON TABLE sesno_version_anchor TYPE option<int>;
DEFINE FIELD IF NOT EXISTS att_rows ON TABLE sesno_version_anchor TYPE option<int>;
DEFINE FIELD IF NOT EXISTS uda_rows ON TABLE sesno_version_anchor TYPE option<int>;
DEFINE FIELD IF NOT EXISTS delete_count ON TABLE sesno_version_anchor TYPE option<int>;
DEFINE FIELD IF NOT EXISTS dbnum_info_updates ON TABLE sesno_version_anchor TYPE option<int>;
DEFINE FIELD IF NOT EXISTS pe_owner_rows ON TABLE sesno_version_anchor TYPE option<int>;
"#;
    project_primary_db()
        .query(sql)
        .await
        .map_err(|error| VersionCommitError::Storage(error.into()))?
        .check()
        .map_err(|error| VersionCommitError::Storage(error.into()))?;
    Ok(())
}

async fn existing_idempotent_outcome(
    request: &VersionCommitRequest,
    recovered: bool,
) -> CommitResult<Option<VersionCommitOutcome>> {
    let sql = format!(
        "SELECT fingerprint, type::string(anchored_at) AS anchored_at, \
         pe_rows, att_rows, uda_rows, delete_count, dbnum_info_updates, pe_owner_rows \
         FROM sesno_version_anchor:[{}, {}, '{}'];",
        request.dbnum,
        request.to_sesno,
        request.source.as_str()
    );
    let mut response = project_primary_db()
        .query(sql)
        .await
        .map_err(|error| VersionCommitError::Storage(error.into()))?
        .check()
        .map_err(|error| VersionCommitError::Storage(error.into()))?;
    let rows: Vec<ExistingAnchor> = response
        .take(0)
        .map_err(|error| VersionCommitError::Storage(error.into()))?;
    let Some(existing) = rows.into_iter().next() else {
        return Ok(None);
    };
    let Some(fingerprint) = existing.fingerprint else {
        return Err(VersionCommitError::LegacyAnchor {
            dbnum: request.dbnum,
            sesno: request.to_sesno,
        });
    };
    if fingerprint != request.fingerprint {
        return Err(VersionCommitError::FingerprintConflict {
            dbnum: request.dbnum,
            sesno: request.to_sesno,
            existing: fingerprint,
            requested: request.fingerprint.clone(),
        });
    }

    let counts = VersionCommitCounts {
        pe_rows: existing.pe_rows.unwrap_or_default().max(0) as usize,
        att_rows: existing.att_rows.unwrap_or_default().max(0) as usize,
        uda_rows: existing.uda_rows.unwrap_or_default().max(0) as usize,
        delete_count: existing.delete_count.unwrap_or_default().max(0) as usize,
        dbnum_info_updates: existing.dbnum_info_updates.unwrap_or_default().max(0) as usize,
        pe_owner_rows: existing.pe_owner_rows.unwrap_or_default().max(0) as usize,
    };
    mark_commit_committed(request, &counts, &existing.anchored_at).await?;
    Ok(Some(VersionCommitOutcome {
        dbnum: request.dbnum,
        from_sesno: request.from_sesno,
        to_sesno: request.to_sesno,
        source: request.source,
        fingerprint: request.fingerprint.clone(),
        anchored_at: existing.anchored_at,
        counts,
        idempotent: true,
        recovered,
    }))
}

/// specs/022 加固：增量提交必须与 Committed Watermark 衔接，把"带洞锚定"
/// 从静默成功变成显式失败（`ContinuityGap`）。
///
/// 规则（仅 `source=incremental` 且非 recover 路径，调用方保证）：
/// - `watermark == 0`（从未全量解析，无锚点也无 legacy 记录）放行——首个增量
///   提交的合法性由调用方保证；
/// - `request.to_sesno <= watermark` 放行——落后区间重放要么命中已有锚点走
///   幂等/冲突分支，要么是 legacy 站点 dbnum_info_table 水位虚高场景，
///   不在本门禁职责内（门禁只拦"跳空"）；
/// - `request.from_sesno > watermark + 1` → `ContinuityGap`（间隙未采集）；
///   `from_sesno <= watermark + 1` 的重叠重放合法。
///
/// baseline 由普通 incremental 在 apply 前、同一 lease 内幂等创建；recover 路径
/// 已由 `require_matching_pending` 的 fingerprint 匹配把关，且存在 legacy 站点
/// `dbnum_info_table` 在半次 apply 中被推进导致回退水位虚高的边界，故 recover
/// 跳过门禁。
async fn reject_continuity_gap(request: &VersionCommitRequest) -> CommitResult<()> {
    if request.source != VersionCommitSource::Incremental {
        return Ok(());
    }
    let watermark = committed_watermark(request.dbnum)
        .await
        .map_err(VersionCommitError::Storage)?;
    if watermark == 0 || request.to_sesno <= watermark {
        return Ok(());
    }
    if request.from_sesno > watermark + 1 {
        return Err(VersionCommitError::ContinuityGap {
            dbnum: request.dbnum,
            watermark,
            requested_from: request.from_sesno,
            requested_to: request.to_sesno,
        });
    }
    Ok(())
}

/// 首次增量的 pre-apply handshake。
///
/// 初始化只写当前态与 model_gen 基线，不写数据锚点。第一个 incremental 在实际
/// 业务 mutation 前，把当时 `dbnum_info_table` 水位固化为完整前态；legacy
/// full/incremental 历史则保持原链，不回填 baseline。
async fn ensure_incremental_baseline_before_apply(
    request: &VersionCommitRequest,
) -> CommitResult<()> {
    if request.source != VersionCommitSource::Incremental {
        return Ok(());
    }
    let sql = format!(
        "SELECT VALUE count() FROM sesno_version_anchor \
         WHERE dbnum = {} AND source IN ['full', 'incremental_baseline', 'incremental'] GROUP ALL;",
        request.dbnum
    );
    let existing = match project_primary_db()
        .query_take::<Vec<surrealdb::types::Value>>(sql, 0)
        .await
    {
        Ok(values) => match values.into_iter().next() {
            Some(value) => optional_u32_from_value(value, "data anchor count")
                .map_err(VersionCommitError::Storage)?
                .unwrap_or_default(),
            None => 0,
        },
        Err(error) if error.to_string().contains("does not exist") => 0,
        Err(error) => return Err(VersionCommitError::Storage(error.into())),
    };
    if existing > 0 {
        return Ok(());
    }

    let baseline_sesno = committed_watermark(request.dbnum)
        .await
        .map_err(VersionCommitError::Storage)?;
    if baseline_sesno == 0 {
        return Err(VersionCommitError::Storage(anyhow!(
            "dbnum={} has no initialized dbnum_info watermark; import the complete current state before the first incremental",
            request.dbnum
        )));
    }
    let fingerprint_input = format!(
        "incremental-baseline-v1:{}:{}",
        request.dbnum, baseline_sesno
    );
    let fingerprint = compute_commit_fingerprint(
        request.dbnum,
        baseline_sesno,
        baseline_sesno,
        VersionCommitSource::IncrementalBaseline,
        None,
        [fingerprint_input.as_str()],
    );
    let sql = format!(
        "CREATE ONLY sesno_version_anchor:[{}, {}, 'incremental_baseline'] SET \
         dbnum = {}, sesno = {}, from_sesno = {}, source = 'incremental_baseline', \
         fingerprint = $fingerprint, source_hash = NONE, \
         pe_rows = 0, att_rows = 0, uda_rows = 0, delete_count = 0, \
         dbnum_info_updates = 0, pe_owner_rows = 0, anchored_at = time::now(), \
         note = 'complete pre-apply state captured before first incremental';",
        request.dbnum, baseline_sesno, request.dbnum, baseline_sesno, baseline_sesno
    );
    project_primary_db()
        .query(sql)
        .bind(("fingerprint", fingerprint))
        .await
        .map_err(|error| VersionCommitError::Storage(error.into()))?
        .check()
        .map_err(|error| VersionCommitError::Storage(error.into()))?;
    Ok(())
}

async fn require_matching_pending(request: &VersionCommitRequest) -> CommitResult<()> {
    let sql = format!(
        "SELECT fingerprint, status FROM version_commit_state:[{}, {}];",
        request.dbnum, request.to_sesno
    );
    let mut response = project_primary_db()
        .query(sql)
        .await
        .map_err(|error| VersionCommitError::Storage(error.into()))?
        .check()
        .map_err(|error| VersionCommitError::Storage(error.into()))?;
    let rows: Vec<RecoveryRow> = response
        .take(0)
        .map_err(|error| VersionCommitError::Storage(error.into()))?;
    let matches = rows.into_iter().any(|row| {
        row.fingerprint == request.fingerprint
            && matches!(row.status.as_str(), "preparing" | "pending")
    });
    if !matches {
        return Err(VersionCommitError::RecoveryNotFound {
            dbnum: request.dbnum,
            sesno: request.to_sesno,
            fingerprint: request.fingerprint.clone(),
        });
    }
    Ok(())
}

async fn reject_pending_commit(
    request: &VersionCommitRequest,
    recovering: bool,
) -> CommitResult<()> {
    let fingerprint_clause = if recovering {
        " AND fingerprint != $fingerprint"
    } else {
        ""
    };
    let sql = format!(
        "SELECT to_sesno FROM version_commit_state \
         WHERE dbnum = {} AND status IN ['preparing', 'pending']{} \
         ORDER BY to_sesno ASC LIMIT 1;",
        request.dbnum, fingerprint_clause
    );
    let query = project_primary_db()
        .query(sql)
        .bind(("fingerprint", request.fingerprint.clone()));
    let mut response = query
        .await
        .map_err(|error| VersionCommitError::Storage(error.into()))?
        .check()
        .map_err(|error| VersionCommitError::Storage(error.into()))?;
    let rows: Vec<PendingRow> = response
        .take(0)
        .map_err(|error| VersionCommitError::Storage(error.into()))?;
    if let Some(row) = rows.into_iter().next() {
        return Err(VersionCommitError::PendingCommit {
            dbnum: request.dbnum,
            pending_sesno: row.to_sesno as u32,
            requested_sesno: request.to_sesno,
        });
    }
    Ok(())
}

async fn mark_commit_preparing(request: &VersionCommitRequest) -> CommitResult<()> {
    let sql = format!(
        "UPSERT version_commit_state:[{}, {}] SET \
         dbnum = {}, from_sesno = {}, to_sesno = {}, source = $source, \
         fingerprint = $fingerprint, source_hash = $source_hash, \
         status = 'preparing', pe_rows = 0, att_rows = 0, uda_rows = 0, \
         delete_count = 0, dbnum_info_updates = 0, pe_owner_rows = 0, anchored_at = NONE, \
         last_error = NONE, updated_at = time::now();",
        request.dbnum, request.to_sesno, request.dbnum, request.from_sesno, request.to_sesno
    );
    checked_bound_query(sql, request, None, None).await
}

async fn mark_commit_pending(request: &VersionCommitRequest, detail: &str) -> CommitResult<()> {
    let sql = format!(
        "UPDATE version_commit_state:[{}, {}] SET \
         status = 'pending', last_error = $last_error, updated_at = time::now();",
        request.dbnum, request.to_sesno
    );
    checked_bound_query(sql, request, Some(detail), None).await
}

async fn mark_commit_committed(
    request: &VersionCommitRequest,
    counts: &VersionCommitCounts,
    anchored_at: &str,
) -> CommitResult<()> {
    let sql = format!(
        "UPSERT version_commit_state:[{}, {}] SET \
         dbnum = {}, from_sesno = {}, to_sesno = {}, source = $source, \
         fingerprint = $fingerprint, source_hash = $source_hash, \
         status = 'committed', pe_rows = {}, att_rows = {}, uda_rows = {}, \
         delete_count = {}, dbnum_info_updates = {}, pe_owner_rows = {}, \
         anchored_at = <datetime>$anchored_at, \
         last_error = NONE, updated_at = time::now();",
        request.dbnum,
        request.to_sesno,
        request.dbnum,
        request.from_sesno,
        request.to_sesno,
        counts.pe_rows,
        counts.att_rows,
        counts.uda_rows,
        counts.delete_count,
        counts.dbnum_info_updates,
        counts.pe_owner_rows
    );
    checked_bound_query(sql, request, None, Some(anchored_at)).await
}

async fn checked_bound_query(
    sql: String,
    request: &VersionCommitRequest,
    last_error: Option<&str>,
    anchored_at: Option<&str>,
) -> CommitResult<()> {
    let mut query = project_primary_db()
        .query(sql)
        .bind(("source", request.source.as_str().to_string()))
        .bind(("fingerprint", request.fingerprint.clone()))
        .bind(("source_hash", request.source_hash.clone()));
    if let Some(last_error) = last_error {
        query = query.bind(("last_error", last_error.to_string()));
    }
    if let Some(anchored_at) = anchored_at {
        query = query.bind(("anchored_at", anchored_at.to_string()));
    }
    query
        .await
        .map_err(|error| VersionCommitError::Storage(error.into()))?
        .check()
        .map_err(|error| VersionCommitError::Storage(error.into()))?;
    Ok(())
}

async fn create_immutable_anchor(
    request: &VersionCommitRequest,
    counts: &VersionCommitCounts,
) -> CommitResult<String> {
    let sql = format!(
        "CREATE ONLY sesno_version_anchor:[{}, {}, '{}'] SET \
         dbnum = {}, sesno = {}, from_sesno = {}, source = $source, \
         fingerprint = $fingerprint, source_hash = $source_hash, \
         pe_rows = {}, att_rows = {}, uda_rows = {}, delete_count = {}, \
         dbnum_info_updates = {}, pe_owner_rows = {}, \
         anchored_at = time::now() RETURN anchored_at;",
        request.dbnum,
        request.to_sesno,
        request.source.as_str(),
        request.dbnum,
        request.to_sesno,
        request.from_sesno,
        counts.pe_rows,
        counts.att_rows,
        counts.uda_rows,
        counts.delete_count,
        counts.dbnum_info_updates,
        counts.pe_owner_rows
    );
    let mut response = project_primary_db()
        .query(sql)
        .bind(("source", request.source.as_str().to_string()))
        .bind(("fingerprint", request.fingerprint.clone()))
        .bind(("source_hash", request.source_hash.clone()))
        .await
        .map_err(|error| VersionCommitError::Storage(error.into()))?
        .check()
        .map_err(|error| VersionCommitError::Storage(error.into()))?;
    let anchored_at: Option<Datetime> = response
        .take((0, "anchored_at"))
        .map_err(|error| VersionCommitError::Storage(error.into()))?;
    anchored_at
        .map(|value| value.to_string())
        .ok_or_else(|| VersionCommitError::Storage(anyhow!("anchor returned no anchored_at")))
}

/// 发布模型生成完成锚点。
///
/// 仅在调用方确认全部模型写入和已启用的后处理成功后调用。同一
/// `(dbnum, sesno)` 成功重跑会刷新 `anchored_at`，失败路径不得调用。
pub async fn write_model_gen_anchor(dbnum: u32, sesno: u32) -> anyhow::Result<ModelGenAnchor> {
    write_model_gen_anchor_with_note(dbnum, sesno, "model generation completed").await
}

pub async fn write_model_gen_anchor_with_note(
    dbnum: u32,
    sesno: u32,
    note: &str,
) -> anyhow::Result<ModelGenAnchor> {
    crate::versioned_db::database::ensure_sesno_version_anchor_schema().await?;
    let sql = format!(
        "UPSERT sesno_version_anchor:[{dbnum}, {sesno}, 'model_gen'] SET \
         dbnum = {dbnum}, sesno = {sesno}, source = 'model_gen', \
         anchored_at = time::now(), note = $note \
         RETURN anchored_at;"
    );
    let mut response = project_primary_db()
        .query(sql)
        .bind(("note", note.to_string()))
        .await?
        .check()?;
    let anchored_at: Option<Datetime> = response.take((0, "anchored_at"))?;
    let anchored_at = anchored_at
        .map(|value| value.to_string())
        .ok_or_else(|| anyhow!("model_gen anchor returned no anchored_at"))?;
    Ok(ModelGenAnchor {
        dbnum,
        sesno,
        source: "model_gen".to_string(),
        anchored_at,
        note: note.to_string(),
    })
}

/// 读取指定 dbnum 在 `dbnum_info_table` 中的当前 sesno。
pub async fn current_dbnum_sesnos(dbnums: &[u32]) -> anyhow::Result<BTreeMap<u32, u32>> {
    if dbnums.is_empty() {
        return Ok(BTreeMap::new());
    }
    let ids = dbnums
        .iter()
        .map(u32::to_string)
        .collect::<Vec<_>>()
        .join(",");
    let sql = format!(
        "SELECT dbnum, math::max(sesno) AS sesno FROM dbnum_info_table \
         WHERE dbnum IN [{ids}] GROUP BY dbnum;"
    );
    let mut response = project_primary_db().query(sql).await?.check()?;
    let rows: Vec<CurrentSesnoRow> = response.take(0)?;
    let mut out = BTreeMap::new();
    for row in rows {
        let Ok(dbnum) = u32::try_from(row.dbnum) else {
            continue;
        };
        let Some(sesno) = row.sesno.and_then(|value| u32::try_from(value).ok()) else {
            continue;
        };
        out.insert(dbnum, sesno);
    }
    Ok(out)
}

async fn all_current_dbnum_sesnos() -> anyhow::Result<BTreeMap<u32, u32>> {
    let sql = "SELECT dbnum, math::max(sesno) AS sesno FROM dbnum_info_table GROUP BY dbnum;";
    let mut response = project_primary_db().query(sql).await?.check()?;
    let rows: Vec<CurrentSesnoRow> = response.take(0)?;
    let mut out = BTreeMap::new();
    for row in rows {
        let Ok(dbnum) = u32::try_from(row.dbnum) else {
            continue;
        };
        let Some(sesno) = row.sesno.and_then(|value| u32::try_from(value).ok()) else {
            continue;
        };
        out.insert(dbnum, sesno);
    }
    Ok(out)
}

/// 在一次完整/手动模型生成及其后处理全部成功后，为参与 dbnum 发布 `model_gen` 锚点。
///
/// 增量管线不能使用本函数：它必须按本次已提交 data anchor 的实际结束 sesno 发布。
pub async fn publish_model_gen_anchors_after_generation(
    db_option: &crate::options::DbOptionExt,
    generation_success: bool,
    stage: &str,
    allow_all_when_unscoped: bool,
) -> anyhow::Result<Vec<ModelGenAnchor>> {
    if !generation_success
        || !db_option.use_surrealdb
        || !db_option.model_writer_mode.writes_to_surreal()
        || db_option.gen_model_dry_run
    {
        return Ok(Vec::new());
    }

    let mut requested_dbnums = db_option.inner.manual_db_nums.clone().unwrap_or_default();
    requested_dbnums.sort_unstable();
    requested_dbnums.dedup();
    let sesnos = if requested_dbnums.is_empty() && allow_all_when_unscoped {
        all_current_dbnum_sesnos().await?
    } else if requested_dbnums.is_empty() {
        anyhow::bail!("{stage} 成功但无法确定参与 dbnum，拒绝发布 model_gen 锚点");
    } else {
        let resolved = current_dbnum_sesnos(&requested_dbnums).await?;
        for dbnum in &requested_dbnums {
            if !resolved.contains_key(dbnum) {
                anyhow::bail!(
                    "{stage} 成功但 dbnum_info_table 缺少 dbnum={dbnum} 当前 sesno，拒绝发布部分 model_gen 锚点"
                );
            }
        }
        resolved
    };
    if sesnos.is_empty() {
        anyhow::bail!("{stage} 成功但 dbnum_info_table 中没有可锚定的 dbnum/sesno");
    }

    let mut anchors = Vec::with_capacity(sesnos.len());
    for (dbnum, sesno) in sesnos {
        let anchor = write_model_gen_anchor(dbnum, sesno).await?;
        println!(
            "✅ model_gen 锚点已发布: stage={} dbnum={} sesno={} anchored_at={}",
            stage, anchor.dbnum, anchor.sesno, anchor.anchored_at
        );
        anchors.push(anchor);
    }
    Ok(anchors)
}

/// Live/manual generation is only valid while building an initialization
/// staging database. A staging database has no business anchors yet; once any
/// legacy/data/model anchor exists, every subsequent model mutation must bind
/// an existing data anchor through catch-up or controlled repair.
pub async fn ensure_live_generation_allowed(
    db_option: &crate::options::DbOptionExt,
    operation: &str,
) -> anyhow::Result<()> {
    if !db_option.versioned_storage {
        return Ok(());
    }
    let sql = "SELECT VALUE count() FROM sesno_version_anchor \
               WHERE source IN ['full', 'incremental_baseline', 'incremental', 'model_gen'] GROUP ALL;";
    let existing = match project_primary_db()
        .query_take::<Vec<surrealdb::types::Value>>(sql, 0)
        .await
    {
        Ok(counts) => match counts.into_iter().next() {
            Some(value) => {
                optional_u32_from_value(value, "business anchor count")?.unwrap_or_default()
            }
            None => 0,
        },
        Err(error) if error.to_string().contains("does not exist") => 0,
        Err(error) => return Err(error.into()),
    };
    if existing > 0 {
        anyhow::bail!(
            "{operation} is disabled for a Ready versioned site; use `model-version catch-up` \
             for continuous debt or `model-version catch-up --allow-full-regen` for explicit \
             controlled repair"
        );
    }
    Ok(())
}

#[derive(Debug)]
struct VersionCommitLease {
    dbnum: u32,
    owner: String,
}

async fn acquire_dbnum_lease(dbnum: u32, lease_secs: u64) -> CommitResult<VersionCommitLease> {
    let owner = next_owner();
    let sql = format!(
        r#"
BEGIN TRANSACTION;
LET $current = (SELECT owner, expires_at FROM version_commit_lease:{dbnum});
IF array::len($current) > 0
    AND $current[0].expires_at > time::now()
    AND $current[0].owner != $owner {{
    THROW "VERSION_COMMIT_LEASE_BUSY";
}};
UPSERT version_commit_lease:{dbnum} SET
    dbnum = {dbnum},
    owner = $owner,
    expires_at = time::now() + duration::from_secs({lease_secs});
COMMIT TRANSACTION;
"#
    );
    let result = project_primary_db()
        .query(sql)
        .bind(("owner", owner.clone()))
        .await
        .map_err(|error| VersionCommitError::Storage(error.into()))?
        .check();
    if let Err(error) = result {
        let message = error.to_string();
        if message.contains("VERSION_COMMIT_LEASE_BUSY") {
            return Err(VersionCommitError::LeaseBusy { dbnum });
        }
        return Err(VersionCommitError::Storage(error.into()));
    }
    Ok(VersionCommitLease { dbnum, owner })
}

async fn release_dbnum_lease(lease: &VersionCommitLease) -> anyhow::Result<()> {
    project_primary_db()
        .query(format!(
            "DELETE version_commit_lease:{} WHERE owner = $owner;",
            lease.dbnum
        ))
        .bind(("owner", lease.owner.clone()))
        .await?
        .check()?;
    Ok(())
}

fn next_owner() -> String {
    let millis = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis();
    let sequence = OWNER_SEQUENCE.fetch_add(1, Ordering::Relaxed);
    format!("{}-{millis}-{sequence}", std::process::id())
}

/// specs/022 Committed Watermark：某 dbnum 已发布 Version Anchor 的最高 sesno。
///
/// 增量采集的唯一合法起点（见 CONTEXT.md）：优先读 `sesno_version_anchor`；
/// 对早于锚定机制的存量库回退 `dbnum_info_table` 的 max sesno；两者皆无返回 0
/// （表示该 dbnum 从未全量解析过，不应做增量）。
///
/// 有意不读 `dbnum_info_table` 优先：Commit Pending 时该表可能领先锚点，
/// 以它为准会静默跳过半写区间；以锚点为准则重试同一区间并被 PendingCommit 拒绝，
/// 直到人工 `--recover-pending`。
///
/// 它同时是 `commit_while_leased` 连续性门禁（`ContinuityGap`）的基准水位，
/// 增量提交的 `from_sesno` 必须与该值衔接（`<= watermark + 1`）。
pub async fn committed_watermark(dbnum: u32) -> anyhow::Result<u32> {
    let sql = format!(
        "math::max(array::flatten([SELECT VALUE sesno FROM sesno_version_anchor \
             WHERE dbnum = {dbnum} AND source IN ['full', 'incremental_baseline', 'incremental']]));\n\
         math::max(array::flatten([SELECT VALUE sesno FROM dbnum_info_table WHERE dbnum = {dbnum}]));"
    );
    let mut response = project_primary_db().query(sql).await?.check()?;
    // 语句级取值：表不存在（例如从未跑过版本提交的存量站点没有
    // `sesno_version_anchor`）按"无记录"处理，其余语句错误照常上抛。
    let anchored = match response.take::<surrealdb::types::Value>(0) {
        Ok(value) => optional_u32_from_value(value, "data anchor sesno")?,
        Err(error) if error.to_string().contains("does not exist") => None,
        Err(error) => return Err(error.into()),
    };
    if let Some(sesno) = anchored.filter(|sesno| *sesno > 0) {
        return Ok(sesno);
    }
    let legacy = match response.take::<surrealdb::types::Value>(1) {
        Ok(value) => optional_u32_from_value(value, "legacy watermark sesno")?,
        Err(error) if error.to_string().contains("does not exist") => None,
        Err(error) => return Err(error.into()),
    };
    Ok(legacy.unwrap_or_default())
}

pub(super) fn optional_u32_from_value(
    value: surrealdb::types::Value,
    field: &str,
) -> anyhow::Result<Option<u32>> {
    use surrealdb::types::{Number, Value};

    let integer = match value {
        Value::None | Value::Null => return Ok(None),
        Value::Number(Number::Int(value)) => value,
        Value::Number(Number::Float(value)) if !value.is_finite() => return Ok(None),
        Value::Number(Number::Float(value)) if value.fract() == 0.0 => value as i64,
        Value::Number(Number::Decimal(value)) => value
            .to_string()
            .parse::<i64>()
            .map_err(|_| anyhow::anyhow!("{field} 不是整数: {value}"))?,
        other => anyhow::bail!("{field} 不是整数: {other:?}"),
    };
    Ok(Some(u32::try_from(integer).map_err(|_| {
        anyhow::anyhow!("{field} 超出 u32 范围: {integer}")
    })?))
}
