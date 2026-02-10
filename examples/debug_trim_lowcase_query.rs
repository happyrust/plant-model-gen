//! 使用 SurrealDB 查询元件库 15194/4553 的几何体子节点
//!
//! 运行方式：
//! ```bash
//! cargo run --example debug_trim_lowcase_query
//! ```

use aios_core::{SUL_DB, SurrealQueryExt, init_surreal};
use anyhow::Context;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    println!("=== 查询元件库 15194/4553 的几何体定义 ===\n");
    
    // 初始化数据库连接
    let _db_option_ext = aios_database::options::get_db_option_ext_from_path("db_options/DbOption")
        .context("加载 DbOption 失败")?;
    init_surreal().await.context("初始化 SurrealDB 失败")?;
    
    // 1. 检查数据库中有哪些表
    println!("=== 1. 数据库中的表 ===");
    let sql = "INFO FOR DB";
    match SUL_DB.query_take::<Vec<serde_json::Value>>(sql, 0).await {
        Ok(result) => {
            for r in &result {
                if let Some(tables) = r.get("tables") {
                    println!("表: {:?}", tables);
                }
            }
        }
        Err(e) => println!("查询失败: {}", e),
    }
    
    // 2. 查询 pe 表中的 SCOM 元素
    println!("\n=== 2. 查询 SCOM 元素 ===");
    let sql = "SELECT id, noun, name FROM pe WHERE noun = 'SCOM' LIMIT 10";
    match SUL_DB.query_take::<Vec<serde_json::Value>>(sql, 0).await {
        Ok(result) => {
            println!("找到 {} 个 SCOM", result.len());
            for r in &result {
                println!("  {:?}", r);
            }
        }
        Err(e) => println!("查询失败: {}", e),
    }
    
    // 3. 全局搜索包含 "TRIM" 字符串的 pe 记录
    println!("\n=== 3. 全局搜索 pe 表中包含 TRIM 的记录 ===");
    let sql = r#"SELECT id, noun, name FROM pe WHERE name CONTAINS "TRIM" LIMIT 20"#;
    match SUL_DB.query_take::<Vec<serde_json::Value>>(sql, 0).await {
        Ok(result) => {
            println!("找到 {} 条记录", result.len());
            for r in &result {
                println!("  {:?}", r);
            }
        }
        Err(e) => println!("查询失败: {}", e),
    }
    
    // 4. 查询 cata_att 表结构
    println!("\n=== 4. 查询 cata_att 表样本 ===");
    let sql = "SELECT * FROM cata_att LIMIT 5";
    match SUL_DB.query_take::<Vec<serde_json::Value>>(sql, 0).await {
        Ok(result) => {
            println!("cata_att 样本数量: {}", result.len());
            for r in &result {
                let r_str = serde_json::to_string(r)?;
                if r_str.to_uppercase().contains("TRIM") || r_str.to_uppercase().contains("LOWCASE") {
                    println!("\n⚠️ 发现目标!");
                    println!("{}", serde_json::to_string_pretty(r)?);
                }
            }
        }
        Err(e) => println!("查询失败: {}", e),
    }
    
    // 5. 搜索 cata_att 表中属性值包含 TRIM 的记录
    println!("\n=== 5. 搜索 cata_att 中包含 TRIM 的属性值 ===");
    // 尝试不同的字段名
    for field in ["pxxx", "pyyy", "pzzz", "pdia", "phei", "pwid", "pang", "prad"] {
        let sql = format!(r#"SELECT * FROM cata_att WHERE {} CONTAINS "TRIM" LIMIT 5"#, field);
        match SUL_DB.query_take::<Vec<serde_json::Value>>(&sql, 0).await {
            Ok(result) if !result.is_empty() => {
                println!("\n在字段 {} 中找到 {} 条记录:", field, result.len());
                for r in &result {
                    println!("{}", serde_json::to_string_pretty(r)?);
                }
            }
            _ => {}
        }
    }
    
    // 6. 直接查询 24381_57603 元素
    println!("\n=== 6. 查询 24381_57603 元素 ===");
    let sql = "SELECT * FROM pe:⟨24381_57603⟩";
    match SUL_DB.query_take::<Vec<serde_json::Value>>(sql, 0).await {
        Ok(result) => {
            for r in &result {
                println!("{}", serde_json::to_string_pretty(r)?);
            }
        }
        Err(e) => println!("查询失败: {}", e),
    }
    
    println!("\n=== 查询完成 ===");
    Ok(())
}
