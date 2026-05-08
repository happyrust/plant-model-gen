use anyhow::{Result, anyhow};
use once_cell::sync::OnceCell;
use std::sync::atomic::{AtomicUsize, Ordering};
use surrealdb::Surreal;
use surrealdb::engine::remote::ws::{Client, Ws};
use surrealdb::opt::auth::Root;

use aios_core::options::{DbConnMode, DbOption};

// 池大小默认 4 路 ws 连接；可通过 REVIEW_PRIMARY_DB_POOL_SIZE 环境变量覆盖。
// 单 ws Surreal<Client> 在高并发（如 newContext 后 plant3d-web 并发 30+ review 请求）
// 时 mpsc channel 会被打满锁死；多路连接池可线性扩展并发能力。
const DEFAULT_POOL_SIZE: usize = 4;

static REVIEW_PRIMARY_DB_POOL: OnceCell<Vec<Surreal<Client>>> = OnceCell::new();
static POOL_CURSOR: AtomicUsize = AtomicUsize::new(0);

fn review_db_address(db_option: &DbOption) -> Result<String> {
    let surreal_cfg = db_option.effective_surrealdb();
    if surreal_cfg.mode != DbConnMode::Ws {
        return Err(anyhow!(
            "review_primary_db 仅支持 surrealdb.mode=ws，当前为 {}",
            surreal_cfg.mode.as_str()
        ));
    }

    Ok(format!(
        "{}:{}",
        if surreal_cfg.ip == "localhost" {
            "127.0.0.1"
        } else {
            surreal_cfg.ip.as_str()
        },
        surreal_cfg.port
    ))
}

fn resolve_pool_size() -> usize {
    std::env::var("REVIEW_PRIMARY_DB_POOL_SIZE")
        .ok()
        .and_then(|s| s.parse::<usize>().ok())
        .filter(|&n| n > 0)
        .unwrap_or(DEFAULT_POOL_SIZE)
}

pub async fn init_review_primary_db(db_option: &DbOption) -> Result<()> {
    if REVIEW_PRIMARY_DB_POOL.get().is_some() {
        return Ok(());
    }

    let surreal_cfg = db_option.effective_surrealdb();
    let address = review_db_address(db_option)?;

    let pool_size = resolve_pool_size();
    let mut clients: Vec<Surreal<Client>> = Vec::with_capacity(pool_size);

    for idx in 0..pool_size {
        let db = Surreal::new::<Ws>(address.as_str()).await.map_err(|e| {
            anyhow!(
                "review_primary_db 连接 #{}/{} 建立失败: {}",
                idx + 1,
                pool_size,
                e
            )
        })?;

        db.signin(Root {
            username: surreal_cfg.user.clone(),
            password: surreal_cfg.password.clone(),
        })
        .await
        .map_err(|e| {
            anyhow!(
                "review_primary_db #{}/{} signin 失败: {}",
                idx + 1,
                pool_size,
                e
            )
        })?;

        aios_core::use_ns_db_compat(&db, &db_option.surreal_ns, &db_option.project_name)
            .await
            .map_err(|e| {
                anyhow!(
                    "review_primary_db #{}/{} use ns/db 失败: {}",
                    idx + 1,
                    pool_size,
                    e
                )
            })?;

        clients.push(db);
    }

    let _ = REVIEW_PRIMARY_DB_POOL.set(clients);
    Ok(())
}

pub async fn fresh_review_db() -> Result<Surreal<Client>> {
    let db_option = aios_core::get_db_option();
    let surreal_cfg = db_option.effective_surrealdb();
    let address = review_db_address(&db_option)?;

    let db = Surreal::new::<Ws>(address.as_str())
        .await
        .map_err(|e| anyhow!("review_primary_db fresh 连接建立失败: {}", e))?;

    db.signin(Root {
        username: surreal_cfg.user.clone(),
        password: surreal_cfg.password.clone(),
    })
    .await
    .map_err(|e| anyhow!("review_primary_db fresh signin 失败: {}", e))?;

    aios_core::use_ns_db_compat(&db, &db_option.surreal_ns, &db_option.project_name)
        .await
        .map_err(|e| anyhow!("review_primary_db fresh use ns/db 失败: {}", e))?;

    Ok(db)
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
    if REVIEW_PRIMARY_DB_POOL.get().is_none() {
        init_review_primary_db(&aios_core::get_db_option()).await?;
    }

    let db_option = aios_core::get_db_option();
    let pool = REVIEW_PRIMARY_DB_POOL
        .get()
        .ok_or_else(|| anyhow!("review_primary_db pool 尚未初始化"))?;

    for (idx, db) in pool.iter().enumerate() {
        aios_core::use_ns_db_compat(db, &db_option.surreal_ns, &db_option.project_name)
            .await
            .map_err(|e| {
                anyhow!(
                    "review_primary_db pool[{}/{}] use ns/db 失败: {}",
                    idx + 1,
                    pool.len(),
                    e
                )
            })?;
    }

    Ok(())
}

pub fn review_primary_db() -> &'static Surreal<Client> {
    let pool = REVIEW_PRIMARY_DB_POOL
        .get()
        .expect("review_primary_db 尚未初始化");
    let idx = POOL_CURSOR.fetch_add(1, Ordering::Relaxed) % pool.len();
    &pool[idx]
}
