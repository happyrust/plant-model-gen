use anyhow::{Result, anyhow};
use once_cell::sync::OnceCell;
use surrealdb::Surreal;
use surrealdb::engine::remote::ws::{Client, Ws};
use surrealdb::opt::auth::Root;

use aios_core::options::{DbConnMode, DbOption};

static REVIEW_PRIMARY_DB: OnceCell<Surreal<Client>> = OnceCell::new();

pub async fn init_review_primary_db(db_option: &DbOption) -> Result<()> {
    if REVIEW_PRIMARY_DB.get().is_some() {
        return Ok(());
    }

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

    let _ = REVIEW_PRIMARY_DB.set(db);
    Ok(())
}

/// 为高并发/批量操作提供“独立连接”的 DB（不复用全局 `review_primary_db`）。
///
/// 说明：部分 Surreal WS 客户端在高并发下可能出现内部 channel 堵塞；这里按需新建连接以隔离请求。
pub async fn fresh_review_db() -> Result<Surreal<Client>> {
    let db_option = aios_core::get_db_option();
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

    let db_option = aios_core::get_db_option();
    aios_core::use_ns_db_compat(
        review_primary_db(),
        &db_option.surreal_ns,
        &db_option.project_name,
    )
    .await?;

    Ok(())
}

pub fn review_primary_db() -> &'static Surreal<Client> {
    REVIEW_PRIMARY_DB
        .get()
        .expect("review_primary_db 尚未初始化")
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
    let db = fresh_review_db().await?;
    db.query(
        r#"
        DEFINE TABLE OVERWRITE review_workflow_history SCHEMAFULL;
        DEFINE FIELD OVERWRITE task_id ON review_workflow_history TYPE string;
        DEFINE FIELD OVERWRITE node ON review_workflow_history TYPE string;
        DEFINE FIELD OVERWRITE action ON review_workflow_history TYPE string;
        DEFINE FIELD OVERWRITE operator_id ON review_workflow_history TYPE option<string>;
        DEFINE FIELD OVERWRITE operator_name ON review_workflow_history TYPE option<string>;
        DEFINE FIELD OVERWRITE timestamp ON review_workflow_history TYPE option<datetime> DEFAULT time::now();
        DEFINE FIELD OVERWRITE comment ON review_workflow_history TYPE option<string>;
        DEFINE FIELD OVERWRITE form_id ON review_workflow_history TYPE option<string>;
        DEFINE FIELD OVERWRITE target_node ON review_workflow_history TYPE option<string>;
        DEFINE FIELD OVERWRITE actor_id ON review_workflow_history TYPE option<string>;
        DEFINE FIELD OVERWRITE actor_role ON review_workflow_history TYPE option<string>;
        DEFINE FIELD OVERWRITE actor_name ON review_workflow_history TYPE option<string>;
        DEFINE FIELD OVERWRITE source ON review_workflow_history TYPE option<string>;
        DEFINE FIELD OVERWRITE created_at ON review_workflow_history TYPE option<datetime>;
        DEFINE INDEX OVERWRITE idx_workflow_task ON review_workflow_history FIELDS task_id;
        DEFINE INDEX OVERWRITE idx_workflow_form ON review_workflow_history FIELDS form_id;

        DEFINE FIELD OVERWRITE form_id ON review_history TYPE option<string>;
        DEFINE FIELD OVERWRITE source ON review_history TYPE option<string>;
        DEFINE FIELD OVERWRITE created_at ON review_history TYPE option<datetime>;
        "#,
    )
    .await?
    .check()?;
    tracing::info!(
        "[REVIEW_DB.schema] review_workflow_history / review_history schema 已升级到 RUS-244 形态（actor_id/source/target_node/form_id/created_at + 旧字段改为 option）"
    );
    Ok(())
}

pub async fn ensure_review_workflow_history_schema() -> Result<()> {
    REVIEW_WORKFLOW_HISTORY_SCHEMA_READY
        .get_or_try_init(|| async { ensure_review_workflow_history_schema_inner().await })
        .await?;
    Ok(())
}
