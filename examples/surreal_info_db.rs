//! 打印 SurrealDB 当前 database 的表/函数/定义信息，用于定位“属性表”。
//!
//! 用法：
//!   cargo run --example surreal_info_db

use aios_core::{SUL_DB, SurrealQueryExt, init_surreal};
use anyhow::Result;

#[tokio::main]
async fn main() -> Result<()> {
    init_surreal().await?;

    let sql = "INFO FOR DB;";
    let rows: Vec<serde_json::Value> = SUL_DB.query_take(sql, 0).await?;
    println!("SQL: {sql}");
    println!("rows={}", rows.len());
    for (i, row) in rows.iter().enumerate() {
        println!("\n#{}:\n{}", i, serde_json::to_string_pretty(row)?);
    }

    Ok(())
}
