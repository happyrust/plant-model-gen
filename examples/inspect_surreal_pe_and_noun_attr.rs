//! 直接检查 SurrealDB 中 pe 记录与“按 noun 分表”的属性记录是否存在。
//!
//! 用法：
//!   $env:REFNO="24381/103385"
//!   cargo run --example inspect_surreal_pe_and_noun_attr

use aios_core::{RefnoEnum, SUL_DB, SurrealQueryExt, init_surreal};
use anyhow::Result;
use serde_json::Value;
use std::env;

fn pretty(v: &Value) -> String {
    serde_json::to_string_pretty(v).unwrap_or_else(|_| format!("{v:?}"))
}

#[tokio::main]
async fn main() -> Result<()> {
    init_surreal().await?;

    let refno_str = env::var("REFNO").unwrap_or_else(|_| "24381/103385".to_string());
    let r = RefnoEnum::from(refno_str.as_str());
    anyhow::ensure!(r.is_valid(), "无效 REFNO: {}", refno_str);

    let sql_pe = format!("SELECT * FROM pe:{};", r);
    let pe_rows: Vec<Value> = SUL_DB.query_take(&sql_pe, 0).await?;
    println!("SQL:\n{sql_pe}\nrows={}", pe_rows.len());
    for row in &pe_rows {
        println!("pe_row:\n{}", pretty(row));
    }

    let noun = pe_rows
        .first()
        .and_then(|v| v.get("noun"))
        .and_then(|v| v.as_str())
        .unwrap_or("");
    if noun.is_empty() {
        println!("⚠️ pe 记录无 noun 字段，无法继续检查 noun 属性表");
        return Ok(());
    }

    // 假设属性表按 noun 分表（如 BRAN:refno / BEND:refno）
    let sql_att = format!("SELECT * FROM {}:{};", noun, r);
    let att_rows: Vec<Value> = SUL_DB.query_take(&sql_att, 0).await.unwrap_or_default();
    println!("\nSQL:\n{sql_att}\nrows={}", att_rows.len());
    for row in &att_rows {
        println!("att_row:\n{}", pretty(row));
    }

    Ok(())
}
