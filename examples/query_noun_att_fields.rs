//! 直接从 noun 属性表查询 ARRI/LEAV/SPRE/CATR（用于证明属性表是否缺失）。
//!
//! 用法：
//!   $env:REFNO="24381/103386"
//!   cargo run --example query_noun_att_fields

use aios_core::{RefnoEnum, SUL_DB, SurrealQueryExt, init_surreal};
use anyhow::Result;
use serde_json::Value;
use std::env;

#[tokio::main]
async fn main() -> Result<()> {
    init_surreal().await?;

    let refno_str = env::var("REFNO").unwrap_or_else(|_| "24381/103386".to_string());
    let r = RefnoEnum::from(refno_str.as_str());
    anyhow::ensure!(r.is_valid(), "无效 REFNO: {}", refno_str);

    // 1) 先取 noun
    let sql_noun = format!("SELECT noun FROM pe:{};", r);
    let rows: Vec<Value> = SUL_DB.query_take(&sql_noun, 0).await?;
    let noun = rows
        .first()
        .and_then(|v| v.get("noun"))
        .and_then(|v| v.as_str())
        .unwrap_or("");
    println!("refno={} noun={}", r, noun);
    anyhow::ensure!(!noun.is_empty(), "pe 中未取到 noun: {}", r);

    // 2) 直接查 noun 表
    let sql = format!(
        "SELECT ARRI, LEAV, record::id(SPRE) as SPRE, record::id(CATR) as CATR FROM ONLY {}:{};",
        noun, r
    );
    println!("SQL:\n{sql}");
    let att_rows: Vec<Value> = SUL_DB.query_take(&sql, 0).await.unwrap_or_default();
    println!("rows={}", att_rows.len());
    for (i, row) in att_rows.iter().enumerate() {
        println!("\n#{}:\n{}", i, serde_json::to_string_pretty(row)?);
    }

    Ok(())
}
