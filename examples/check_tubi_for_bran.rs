//! 检查指定 BRAN 的 tubi_relate 数据是否存在
//!
//! 用法: cargo run --example check_tubi_for_bran

use aios_core::{RefnoEnum, SUL_DB, SurrealQueryExt, init_surreal};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let target = std::env::var("BRAN_REFNO").unwrap_or_else(|_| "24381/145018".to_string());
    let refno = RefnoEnum::from(target.as_str());
    let pe_key = refno.to_pe_key();

    println!("=== 检查 BRAN {} 的 tubi_relate 数据 ===", refno);
    println!("pe_key = {}\n", pe_key);

    init_surreal().await?;
    println!("✓ SurrealDB 连接成功\n");

    // 1. 查询 tubi_relate 记录数
    let sql = format!(
        "SELECT count() as cnt FROM tubi_relate:[{pe_key}, 0]..[{pe_key}, ..];"
    );
    let rows: Vec<serde_json::Value> = SUL_DB.query_take(&sql, 0).await.unwrap_or_default();
    println!("tubi_relate 查询结果: {:?}", rows);

    // 2. 查询详细段数据
    let detail_sql = format!(
        r#"
        SELECT
            id[0] as owner_refno,
            in as leave_refno,
            out as arrive_refno,
            start_pt.d as start_pt,
            end_pt.d as end_pt,
            id[1] as index
        FROM tubi_relate:[{pe_key}, 0]..[{pe_key}, ..];
        "#
    );
    let detail_rows: Vec<serde_json::Value> = SUL_DB.query_take(&detail_sql, 0).await.unwrap_or_default();
    println!("\ntubi_relate 段数: {}", detail_rows.len());

    if detail_rows.is_empty() {
        println!("\n⚠️  该 BRAN 无 tubi_relate 数据，需要先执行模型生成：");
        println!("   cargo run --bin aios-database -- --debug-model {} --regen-model", target);
    } else {
        println!("\n✅ 该 BRAN 已有 tubi_relate 数据，各段详情：");
        for (i, row) in detail_rows.iter().enumerate() {
            println!("  [{}] leave={}, arrive={}, index={}, start_pt={}, end_pt={}",
                i,
                row.get("leave_refno").unwrap_or(&serde_json::Value::Null),
                row.get("arrive_refno").unwrap_or(&serde_json::Value::Null),
                row.get("index").unwrap_or(&serde_json::Value::Null),
                row.get("start_pt").unwrap_or(&serde_json::Value::Null),
                row.get("end_pt").unwrap_or(&serde_json::Value::Null),
            );
        }
    }

    // 3. 检查 pe 表中该 refno 的 noun
    let pe_sql = format!(
        "SELECT id, noun FROM {} LIMIT 1;",
        pe_key
    );
    let pe_rows: Vec<serde_json::Value> = SUL_DB.query_take(&pe_sql, 0).await.unwrap_or_default();
    if let Some(row) = pe_rows.first() {
        println!("\npe 表记录: noun={}", row.get("noun").unwrap_or(&serde_json::Value::Null));
    } else {
        println!("\n⚠️  pe 表中未找到 {} 的记录", pe_key);
    }

    println!("\n=== 检查完成 ===");
    Ok(())
}
