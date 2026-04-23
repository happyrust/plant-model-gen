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

pub async fn ensure_review_primary_db_context() -> Result<()> {
    if REVIEW_PRIMARY_DB.get().is_none() {
        init_review_primary_db(aios_core::get_db_option()).await?;
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
