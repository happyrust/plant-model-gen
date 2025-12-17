//! 验证参考号是否存在并测试不同的查询方式

use aios_core::{init_surreal, RefnoEnum, SUL_DB, SurrealQueryExt};
use aios_core::pdms_types::RefU64;
use anyhow::Result;

#[tokio::main]
async fn main() -> Result<()> {
    // 初始化数据库连接
    println!("🔌 初始化数据库连接...");
    init_surreal().await?;
    println!("✅ 数据库连接成功");

    // 测试的参考号
    let panel_refno = RefnoEnum::Refno(RefU64::from_two_nums(17496, 198106));
    let pipe_refno = RefnoEnum::Refno(RefU64::from_two_nums(24381, 59222));

    println!("\n🎯 验证参考号:");
    println!("  - PANEL: {} (to_string: {})", panel_refno, panel_refno.to_string());
    println!("  - 管件: {} (to_string: {})", pipe_refno, pipe_refno.to_string());
    println!("  - PANEL: {} (to_pe_key: {})", panel_refno, panel_refno.to_pe_key());
    println!("  - 管件: {} (to_pe_key: {})", pipe_refno, pipe_refno.to_pe_key());

    // 1. 查询 pe 表中是否有这些参考号（使用 WHERE）
    println!("\n📋 1. 使用 WHERE 查询 pe 表...");
    query_pe_with_where(&panel_refno, "PANEL").await?;
    query_pe_with_where(&pipe_refno, "管件").await?;

    // 2. 查询 pe 表中是否有这些参考号（使用 table:id）
    println!("\n📋 2. 使用 table:id 查询 pe 表...");
    query_pe_with_table_id(&panel_refno, "PANEL").await?;
    query_pe_with_table_id(&pipe_refno, "管件").await?;

    // 3. 查询 inst_relate 表
    println!("\n📋 3. 查询 inst_relate 表...");
    query_inst_relate(&panel_refno, "PANEL").await?;
    query_inst_relate(&pipe_refno, "管件").await?;

    // 4. 查询相似的参考号（看看是否存在相近的）
    println!("\n📋 4. 查询相似的参考号...");
    query_similar_refnos(17496, "PANEL").await?;
    query_similar_refnos(24381, "管件").await?;

    // 5. 查询 PANEL 类型的实体
    println!("\n📋 5. 查询所有 PANEL 类型的实体（前10个）...");
    query_panels().await?;

    Ok(())
}

/// 使用 WHERE 查询 pe 表
async fn query_pe_with_where(refno: &RefnoEnum, entity_type: &str) -> Result<()> {
    let sql = format!(
        "SELECT refno, noun, name FROM pe WHERE refno = {}",
        refno.to_string()
    );
    
    println!("   SQL: {}", sql);
    
    if let Ok(results) = SUL_DB.query_take::<Vec<serde_json::Value>>(&sql, 0).await {
        println!("   ✅ 找到 {} 个 {} 记录", results.len(), entity_type);
        for result in &results {
            println!("     {:?}", result);
        }
    } else {
        println!("   ❌ 查询失败");
    }
    
    Ok(())
}

/// 使用 table:id 查询 pe 表
async fn query_pe_with_table_id(refno: &RefnoEnum, entity_type: &str) -> Result<()> {
    let sql = format!(
        "SELECT refno, noun, name FROM pe:{}",
        refno.to_string()
    );
    
    println!("   SQL: {}", sql);
    
    if let Ok(results) = SUL_DB.query_take::<Vec<serde_json::Value>>(&sql, 0).await {
        println!("   ✅ 找到 {} 个 {} 记录", results.len(), entity_type);
        for result in &results {
            println!("     {:?}", result);
        }
    } else {
        println!("   ❌ 查询失败");
    }
    
    Ok(())
}

/// 查询 inst_relate 表
async fn query_inst_relate(refno: &RefnoEnum, entity_type: &str) -> Result<()> {
    // 尝试两种方式：in 和 out
    let sql_in = format!(
        "SELECT * FROM inst_relate WHERE in = {}",
        refno.to_pe_key()
    );
    
    let sql_out = format!(
        "SELECT * FROM inst_relate WHERE out = {}",
        refno.to_pe_key()
    );
    
    println!("   查询 inst_relate WHERE in...");
    if let Ok(results) = SUL_DB.query_take::<Vec<serde_json::Value>>(&sql_in, 0).await {
        println!("   ✅ 找到 {} 个 {} 记录 (in)", results.len(), entity_type);
        if !results.is_empty() && results.len() <= 3 {
            for result in &results {
                println!("     {:?}", result);
            }
        }
    } else {
        println!("   ❌ 查询失败 (in)");
    }
    
    println!("   查询 inst_relate WHERE out...");
    if let Ok(results) = SUL_DB.query_take::<Vec<serde_json::Value>>(&sql_out, 0).await {
        println!("   ✅ 找到 {} 个 {} 记录 (out)", results.len(), entity_type);
        if !results.is_empty() && results.len() <= 3 {
            for result in &results {
                println!("     {:?}", result);
            }
        }
    } else {
        println!("   ❌ 查询失败 (out)");
    }
    
    Ok(())
}

/// 查询相似的参考号
async fn query_similar_refnos(prefix: u32, entity_type: &str) -> Result<()> {
    let sql = format!(
        "SELECT refno, noun, name FROM pe WHERE refno > {} AND refno < {} LIMIT 10",
        prefix * 100000,
        (prefix + 1) * 100000
    );
    
    println!("   查询 {} 开头的参考号...", prefix);
    
    if let Ok(results) = SUL_DB.query_take::<Vec<serde_json::Value>>(&sql, 0).await {
        println!("   ✅ 找到 {} 个 {} 开头的记录", results.len(), prefix);
        for result in &results {
            if let Some(refno) = result.get("refno") {
                println!("     refno: {:?}, noun: {:?}", refno, result.get("noun"));
            }
        }
    } else {
        println!("   ❌ 查询失败");
    }
    
    Ok(())
}

/// 查询 PANEL 类型的实体
async fn query_panels() -> Result<()> {
    let sql = "SELECT refno, noun, name FROM pe WHERE noun = 'PANEL' LIMIT 10";
    
    if let Ok(results) = SUL_DB.query_take::<Vec<serde_json::Value>>(&sql, 0).await {
        println!("   ✅ 找到 {} 个 PANEL 记录", results.len());
        for result in &results {
            if let Some(refno) = result.get("refno") {
                println!("     refno: {:?}, name: {:?}", refno, result.get("name"));
            }
        }
    } else {
        println!("   ❌ 查询失败");
    }
    
    Ok(())
}
