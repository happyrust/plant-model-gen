//! Scene Tree 初始化测试
//!
//! 测试 Scene Tree 初始化功能，包括：
//! - Schema 初始化
//! - 按 dbno 初始化
//! - geo_type 分布查询
//!
//! 运行方式：
//! ```bash
//! cargo run --bin test_scene_tree_init --features gen_model
//! ```

use aios_core::{SUL_DB, SurrealQueryExt};
use anyhow::Result;

const TEST_DBNO: u32 = 1112;

#[tokio::main]
async fn main() -> Result<()> {
    // 初始化日志
    env_logger::Builder::from_env(
        env_logger::Env::default().default_filter_or("info")
    ).init();

    println!("=== Scene Tree 初始化测试 ===\n");

    // 1. 初始化数据库连接
    println!("1. 初始化数据库连接...");
    if let Err(e) = aios_core::init_surreal().await {
        let msg = e.to_string();
        if !msg.contains("Already connected") {
            println!("   ✗ 数据库连接失败: {}", e);
            return Err(e);
        }
        println!("   ⚠ 数据库已连接");
    } else {
        println!("   ✓ 数据库连接成功");
    }

    // 2. 测试 Schema 初始化
    println!("\n2. 测试 Schema 初始化...");
    match aios_database::scene_tree::init_schema().await {
        Ok(_) => println!("   ✓ Schema 初始化成功"),
        Err(e) => println!("   ✗ Schema 初始化失败: {}", e),
    }

    // 3. 测试初始化检查
    println!("\n3. 测试初始化检查...");
    match aios_database::scene_tree::is_initialized().await {
        Ok(initialized) => {
            println!("   ✓ 检查成功，已初始化: {}", initialized);
        }
        Err(e) => println!("   ✗ 检查失败: {}", e),
    }

    // 4. 按 dbno 初始化
    println!("\n4. 按 dbno={} 初始化 Scene Tree...", TEST_DBNO);
    match aios_database::scene_tree::init_scene_tree_by_dbno(TEST_DBNO, true).await {
        Ok(result) => {
            println!("   ✓ 初始化成功:");
            println!("     - 节点数: {}", result.node_count);
            println!("     - 关系数: {}", result.relation_count);
            println!("     - 耗时: {} ms", result.duration_ms);
        }
        Err(e) => {
            println!("   ✗ 初始化失败: {}", e);
            return Err(e);
        }
    }

    // 5. 查询 geo_type 分布
    println!("\n5. 查询 geo_type 分布...");
    let sql = format!(
        "SELECT geo_type, count() as cnt FROM scene_node \
         WHERE dbno = {} AND has_geo = true GROUP BY geo_type",
        TEST_DBNO
    );
    match SUL_DB.query(&sql).await {
        Ok(mut resp) => {
            let results: Vec<serde_json::Value> = resp.take(0).unwrap_or_default();
            println!("   ✓ 查询成功，geo_type 分布:");
            for row in results {
                let geo_type = row.get("geo_type")
                    .and_then(|v| v.as_str())
                    .unwrap_or("None");
                let cnt = row.get("cnt")
                    .and_then(|v| v.as_i64())
                    .unwrap_or(0);
                println!("     - {}: {}", geo_type, cnt);
            }
        }
        Err(e) => println!("   ✗ 查询失败: {}", e),
    }

    // 6. 查询正负实体数量
    println!("\n6. 查询正负实体数量...");
    let sql_pos = format!(
        "SELECT VALUE count() FROM scene_node WHERE dbno = {} AND geo_type = 'Pos' GROUP ALL",
        TEST_DBNO
    );
    let sql_neg = format!(
        "SELECT VALUE count() FROM scene_node WHERE dbno = {} AND geo_type IN ['Neg', 'CataNeg', 'CataCrossNeg'] GROUP ALL",
        TEST_DBNO
    );

    let pos_count: i64 = SUL_DB.query_take(&sql_pos, 0).await
        .map(|v: Vec<i64>| v.first().copied().unwrap_or(0))
        .unwrap_or(0);
    let neg_count: i64 = SUL_DB.query_take(&sql_neg, 0).await
        .map(|v: Vec<i64>| v.first().copied().unwrap_or(0))
        .unwrap_or(0);

    println!("   ✓ 查询结果:");
    println!("     - 正实体 (Pos): {}", pos_count);
    println!("     - 负实体 (Neg/CataNeg/CataCrossNeg): {}", neg_count);

    // 7. 查询总节点数
    println!("\n7. 查询总节点数...");
    let sql_total = format!(
        "SELECT VALUE count() FROM scene_node WHERE dbno = {} GROUP ALL",
        TEST_DBNO
    );
    let total: i64 = SUL_DB.query_take(&sql_total, 0).await
        .map(|v: Vec<i64>| v.first().copied().unwrap_or(0))
        .unwrap_or(0);
    println!("   ✓ 总节点数: {}", total);

    println!("\n=== 测试完成 ===");
    Ok(())
}
