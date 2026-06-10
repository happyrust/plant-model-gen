use anyhow::{Result, anyhow};
use sha2::{Digest, Sha256};
use std::future::{Future, IntoFuture};
use std::time::Instant;
use surrealdb::IndexedResults;
use surrealdb::Surreal;
use surrealdb::engine::remote::ws::{Client, Ws};
use surrealdb::opt::auth::Root;
use tokio::time::{Duration, timeout};

use aios_core::options::{DbConnMode, DbOption};

static REVIEW_PRIMARY_DB: tokio::sync::OnceCell<Surreal<Client>> =
    tokio::sync::OnceCell::const_new();
static REVIEW_SCHEMA_MIGRATIONS_READY: tokio::sync::OnceCell<()> =
    tokio::sync::OnceCell::const_new();
static REVIEW_QUERY_INDEXES_READY: tokio::sync::OnceCell<()> = tokio::sync::OnceCell::const_new();

const REVIEW_CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
const REVIEW_QUERY_TIMEOUT: Duration = Duration::from_secs(5);
const REVIEW_DDL_TIMEOUT: Duration = Duration::from_secs(30);
const REVIEW_SLOW_QUERY_WARN: Duration = Duration::from_millis(1000);

struct ReviewSchemaMigration {
    version: &'static str,
    name: &'static str,
    sql: &'static str,
}

const REVIEW_SCHEMA_MIGRATIONS: &[ReviewSchemaMigration] = &[
    ReviewSchemaMigration {
        version: "20260610_001",
        name: "review_core_schema",
        sql: include_str!(
            "../../rs_surreal/review/migrations/20260610_001_review_core_schema.surql"
        ),
    },
    ReviewSchemaMigration {
        version: "20260610_002",
        name: "api_request_log",
        sql: include_str!("../../rs_surreal/review/migrations/20260610_002_api_request_log.surql"),
    },
];

const REVIEW_SCHEMA_MIGRATIONS_TABLE_SQL: &str = r#"
    DEFINE TABLE IF NOT EXISTS review_schema_migrations SCHEMAFULL;
    DEFINE FIELD OVERWRITE version ON review_schema_migrations TYPE string;
    DEFINE FIELD OVERWRITE name ON review_schema_migrations TYPE string;
    DEFINE FIELD OVERWRITE checksum ON review_schema_migrations TYPE string;
    DEFINE FIELD OVERWRITE applied_at ON review_schema_migrations TYPE datetime DEFAULT time::now();
    DEFINE INDEX IF NOT EXISTS idx_review_schema_migrations_version ON TABLE review_schema_migrations FIELDS version UNIQUE;
"#;

fn review_schema_migration_checksum(sql: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(sql.as_bytes());
    hex::encode(hasher.finalize())
}

async fn with_review_timeout<T, F>(
    operation: &'static str,
    timeout_after: Duration,
    fut: F,
) -> Result<T>
where
    F: Future<Output = Result<T>>,
{
    let started = Instant::now();
    match timeout(timeout_after, fut).await {
        Ok(Ok(value)) => {
            let elapsed = started.elapsed();
            if elapsed >= REVIEW_SLOW_QUERY_WARN {
                tracing::warn!(
                    "[REVIEW_DB.slow] operation={} elapsed_ms={} timeout_ms={}",
                    operation,
                    elapsed.as_millis(),
                    timeout_after.as_millis()
                );
            }
            Ok(value)
        }
        Ok(Err(error)) => {
            tracing::warn!(
                "[REVIEW_DB.error] operation={} elapsed_ms={} error={}",
                operation,
                started.elapsed().as_millis(),
                error
            );
            Err(error)
        }
        Err(_) => {
            tracing::warn!(
                "[REVIEW_DB.timeout] operation={} elapsed_ms={} timeout_ms={}",
                operation,
                started.elapsed().as_millis(),
                timeout_after.as_millis()
            );
            Err(anyhow!(
                "{} 超时（{}ms）",
                operation,
                timeout_after.as_millis()
            ))
        }
    }
}

pub async fn await_review_query<F>(operation: &'static str, fut: F) -> Result<IndexedResults>
where
    F: IntoFuture<Output = std::result::Result<IndexedResults, surrealdb::Error>>,
{
    with_review_timeout(operation, REVIEW_QUERY_TIMEOUT, async {
        fut.into_future().await.map_err(anyhow::Error::from)
    })
    .await
}

pub async fn await_review_query_long<F>(operation: &'static str, fut: F) -> Result<IndexedResults>
where
    F: IntoFuture<Output = std::result::Result<IndexedResults, surrealdb::Error>>,
{
    with_review_timeout(operation, Duration::from_secs(8), async {
        fut.into_future().await.map_err(anyhow::Error::from)
    })
    .await
}

pub(crate) async fn await_review_ddl<F>(operation: &'static str, fut: F) -> Result<IndexedResults>
where
    F: IntoFuture<Output = std::result::Result<IndexedResults, surrealdb::Error>>,
{
    with_review_timeout(operation, REVIEW_DDL_TIMEOUT, async {
        fut.into_future().await.map_err(anyhow::Error::from)
    })
    .await
}

pub async fn init_review_primary_db(db_option: &DbOption) -> Result<()> {
    let db_option = db_option.clone();
    REVIEW_PRIMARY_DB
        .get_or_try_init(|| async move {
            with_review_timeout(
                "review.init_primary_db",
                REVIEW_CONNECT_TIMEOUT,
                async move { open_review_db_from_option(&db_option).await },
            )
            .await
        })
        .await?;
    Ok(())
}

/// Return a per-request review DB session without opening a new WebSocket.
pub async fn review_db_session() -> Result<Surreal<Client>> {
    ensure_review_primary_db_context().await?;
    Ok(review_primary_db().clone())
}

/// Compatibility wrapper for existing review code.
/// The returned value is a cloned SurrealDB session, not a new physical connection.
pub async fn fresh_review_db() -> Result<Surreal<Client>> {
    review_db_session().await
}

async fn open_review_db_from_option(db_option: &DbOption) -> Result<Surreal<Client>> {
    let surreal_cfg = db_option.effective_surrealdb();
    if surreal_cfg.mode != DbConnMode::Ws {
        return Err(anyhow!(
            "review_primary_db 仅支持 surrealdb.mode=ws，当前为 {}",
            surreal_cfg.mode.as_str()
        ));
    }

    let address = format!(
        "{}:{}",
        if surreal_cfg.ip == "localhost" {
            "127.0.0.1"
        } else {
            surreal_cfg.ip.as_str()
        },
        surreal_cfg.port
    );

    let db = Surreal::new::<Ws>(address.as_str()).await?;
    db.signin(Root {
        username: surreal_cfg.user.clone(),
        password: surreal_cfg.password.clone(),
    })
    .await?;
    aios_core::use_ns_db_compat(&db, &db_option.surreal_ns, &db_option.project_name).await?;

    Ok(db)
}

pub async fn ensure_review_primary_db_context() -> Result<()> {
    if REVIEW_PRIMARY_DB.get().is_none() {
        init_review_primary_db(&aios_core::get_db_option()).await?;
    }
    Ok(())
}

pub fn review_primary_db() -> &'static Surreal<Client> {
    REVIEW_PRIMARY_DB
        .get()
        .expect("review_primary_db 尚未初始化")
}

async fn ensure_review_schema_migrations_inner() -> Result<()> {
    let db = fresh_review_db().await?;
    await_review_ddl(
        "review.ensure_schema_migrations_table",
        db.query(REVIEW_SCHEMA_MIGRATIONS_TABLE_SQL),
    )
    .await?
    .check()?;

    for migration in REVIEW_SCHEMA_MIGRATIONS {
        let checksum = review_schema_migration_checksum(migration.sql);
        await_review_ddl(
            "review.check_schema_migration",
            db.query(
                r#"
                LET $existing = SELECT VALUE checksum FROM review_schema_migrations
                    WHERE version = $version
                    LIMIT 1;

                IF array::len($existing) > 0 AND $existing[0] != $checksum {
                    THROW "REVIEW_SCHEMA_MIGRATION_CHECKSUM_MISMATCH";
                };
                "#,
            )
            .bind(("version", migration.version.to_string()))
            .bind(("checksum", checksum.clone())),
        )
        .await?
        .check()?;

        await_review_ddl("review.apply_schema_migration", db.query(migration.sql))
            .await?
            .check()?;

        await_review_ddl(
            "review.record_schema_migration",
            db.query(
                r#"
                UPSERT type::record('review_schema_migrations', $version) CONTENT {
                    version: $version,
                    name: $name,
                    checksum: $checksum,
                    applied_at: time::now()
                };
                "#,
            )
            .bind(("version", migration.version.to_string()))
            .bind(("name", migration.name.to_string()))
            .bind(("checksum", checksum)),
        )
        .await?
        .check()?;

        tracing::info!(
            "[REVIEW_DB.schema] migration 已确认 version={} name={}",
            migration.version,
            migration.name
        );
    }

    Ok(())
}

pub async fn ensure_review_schema_migrations() -> Result<()> {
    REVIEW_SCHEMA_MIGRATIONS_READY
        .get_or_try_init(|| async { ensure_review_schema_migrations_inner().await })
        .await?;
    Ok(())
}

pub fn review_schema_migrations_ready() -> bool {
    REVIEW_SCHEMA_MIGRATIONS_READY.get().is_some()
}

async fn ensure_review_query_indexes_inner() -> Result<()> {
    ensure_review_schema_migrations().await?;
    tracing::info!(
        "[REVIEW_DB.schema] review 查询索引已由 schema migration 确认（tasks/records/comments/attachments/form_model）"
    );
    Ok(())
}

pub async fn ensure_review_query_indexes() -> Result<()> {
    REVIEW_QUERY_INDEXES_READY
        .get_or_try_init(|| async { ensure_review_query_indexes_inner().await })
        .await?;
    Ok(())
}

pub fn review_query_indexes_ready() -> bool {
    REVIEW_QUERY_INDEXES_READY.get().is_some()
}

// ============================================================================
// review_workflow_history schema 升级（RUS-244 fix 配套）
//
// 旧字段 operator_id / operator_name / timestamp 是 SCHEMAFULL 必填 string/datetime；
// RUS-244 fix 改写 history 时不再传这些旧字段，仅传 actor_id/actor_role/actor_name
// 等新字段，导致 SCHEMAFULL 表 silently 拒绝写入（生产 regression）。
// 此函数在每次需要写 history 前先 await，把旧字段改为 option<...> + 加新字段定义。
// 用 OnceCell 保证只执行一次（成功后所有调用直接返回，无开销）。
// ============================================================================
static REVIEW_WORKFLOW_HISTORY_SCHEMA_READY: tokio::sync::OnceCell<()> =
    tokio::sync::OnceCell::const_new();

async fn ensure_review_workflow_history_schema_inner() -> Result<()> {
    ensure_review_schema_migrations().await?;
    tracing::info!(
        "[REVIEW_DB.schema] review_workflow_history / review_history schema 已由 schema migration 确认（actor_id/source/target_node/form_id/created_at + 旧字段改为 option）"
    );
    Ok(())
}

pub async fn ensure_review_workflow_history_schema() -> Result<()> {
    REVIEW_WORKFLOW_HISTORY_SCHEMA_READY
        .get_or_try_init(|| async { ensure_review_workflow_history_schema_inner().await })
        .await?;
    Ok(())
}

pub fn review_workflow_history_schema_ready() -> bool {
    REVIEW_WORKFLOW_HISTORY_SCHEMA_READY.get().is_some()
}

pub async fn warm_review_schema() -> Result<()> {
    ensure_review_schema_migrations().await?;
    ensure_review_query_indexes().await?;
    ensure_review_workflow_history_schema().await?;
    Ok(())
}
