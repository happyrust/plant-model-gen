//! 验证 inst_relate 中 owner_refno 和 owner_type 修复

use aios_core::{SUL_DB, SurrealQueryExt, init_surreal};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // 初始化数据库连接
    init_surreal().await?;
    
    // 查询 BRAN 17496_171606 下管件 17496_171626 的 inst_relate 记录
    let sql = r#"
        SELECT id, generic, owner_refno, owner_type, in as input 
        FROM inst_relate 
        WHERE in = pe:17496_171626
    "#;
    
    println!("查询 SQL: {}", sql);
    
    let result: Vec<serde_json::Value> = SUL_DB.query_take(sql, 0).await?;
    
    if result.is_empty() {
        println!("❌ 未找到 pe:17496_171626 的 inst_relate 记录");
    } else {
        println!("✅ 找到 {} 条记录:", result.len());
        for record in &result {
            println!("{}", serde_json::to_string_pretty(record)?);
        }
        
        // 验证 owner_refno 和 owner_type
        if let Some(first) = result.first() {
            let owner_refno = first.get("owner_refno").and_then(|v| v.as_str());
            let owner_type = first.get("owner_type").and_then(|v| v.as_str());
            
            println!("\n=== 验证结果 ===");
            if owner_refno == Some("pe:17496_171606") || owner_refno == Some("pe:⟨17496_171606⟩") {
                println!("✅ owner_refno 正确: {:?}", owner_refno);
            } else {
                println!("❌ owner_refno 不正确: {:?} (期望: pe:17496_171606)", owner_refno);
            }
            
            if owner_type == Some("BRAN") {
                println!("✅ owner_type 正确: {:?}", owner_type);
            } else {
                println!("❌ owner_type 不正确: {:?} (期望: BRAN)", owner_type);
            }
        }
    }
    
    Ok(())
}
