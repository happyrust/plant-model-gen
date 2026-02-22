//! 验证 --sync-to-db 后 SurrealDB 中 24381/145018 相关表的数据
//!
//! 基于 plant-surrealdb skill 的最佳实践：
//! - 使用 SELECT VALUE count() 获取标量值
//! - 使用 ID Range 查询 tubi_relate
//! - 使用正确的 pe key 格式 (pe:`ref0_ref1`)
//!
//! 用法: cargo run --example verify_sync_to_db_24381_145018

use aios_core::{RefnoEnum, SUL_DB, SurrealQueryExt, init_surreal};
use anyhow::Result;
use std::str::FromStr;

#[tokio::main]
async fn main() -> Result<()> {
    println!("\n========== 验证 sync-to-db 结果 (24381/145018) ==========\n");

    // 初始化数据库连接
    init_surreal().await?;

    // 准备 refno 列表 (24381_145018 .. 24381_145035)
    let refnos: Vec<RefnoEnum> = (145018..=145035)
        .map(|n| RefnoEnum::from_str(&format!("24381/{}", n)).unwrap())
        .collect();

    let pe_keys: Vec<String> = refnos.iter().map(|r| r.to_pe_key()).collect();
    let pe_list_str = pe_keys.join(", ");

    // 1. inst_relate: 调试范围内 refno 应有记录
    println!("1. inst_relate (in 为 24381_145018..145035):");
    let sql_inst_relate = format!(
        "SELECT VALUE count() FROM inst_relate WHERE in IN [{}];",
        pe_list_str
    );
    match SUL_DB.query_take::<Vec<u64>>(&sql_inst_relate, 0).await {
        Ok(results) => {
            let cnt = results.first().copied().unwrap_or(0);
            println!("   count = {} {}", cnt, if cnt > 0 { "✅" } else { "⚠️" });
        }
        Err(e) => println!("   Error: {} ❌", e),
    }

    // 2. geo_relate 总数
    println!("\n2. geo_relate 总条数:");
    let sql_geo_relate = "SELECT VALUE count() FROM geo_relate;";
    match SUL_DB.query_take::<Vec<u64>>(sql_geo_relate, 0).await {
        Ok(results) => {
            let cnt = results.first().copied().unwrap_or(0);
            println!("   count = {} {}", cnt, if cnt > 0 { "✅" } else { "⚠️" });
        }
        Err(e) => println!("   Error: {} ❌", e),
    }

    // 3. neg_relate 总条数
    println!("\n3. neg_relate 总条数:");
    let sql_neg_relate = "SELECT VALUE count() FROM neg_relate;";
    match SUL_DB.query_take::<Vec<u64>>(sql_neg_relate, 0).await {
        Ok(results) => {
            let cnt = results.first().copied().unwrap_or(0);
            println!("   count = {} {}", cnt, if cnt > 0 { "✅" } else { "⚠️" });
        }
        Err(e) => println!("   Error: {} ❌", e),
    }

    // 4. ngmr_relate 总条数
    println!("\n4. ngmr_relate 总条数:");
    let sql_ngmr_relate = "SELECT VALUE count() FROM ngmr_relate;";
    match SUL_DB.query_take::<Vec<u64>>(sql_ngmr_relate, 0).await {
        Ok(results) => {
            let cnt = results.first().copied().unwrap_or(0);
            println!("   count = {} {}", cnt, if cnt > 0 { "✅" } else { "⚠️" });
        }
        Err(e) => println!("   Error: {} ❌", e),
    }

    // 5. tubi_relate: BRAN 24381_145018 使用 ID Range 查询（推荐方式）
    println!("\n5. tubi_relate (BRAN pe:24381_145018, 使用 ID Range):");
    let bran_refno = RefnoEnum::from_str("24381/145018")?;
    let bran_pe_key = bran_refno.to_pe_key();
    let sql_tubi = format!(
        "SELECT VALUE count() FROM tubi_relate:[{}, 0]..[{}, ..];",
        bran_pe_key, bran_pe_key
    );
    match SUL_DB.query_take::<Vec<u64>>(&sql_tubi, 0).await {
        Ok(results) => {
            let cnt = results.first().copied().unwrap_or(0);
            let ok = cnt == 11;
            println!(
                "   count = {} (预期 11) {}",
                cnt,
                if ok { "✅" } else { "⚠️" }
            );
        }
        Err(e) => println!("   Error: {} ❌", e),
    }

    // 6. inst_relate_aabb: 上述 refno 中应有部分带 aabb
    println!("\n6. inst_relate_aabb (in 为 24381_145018..145035):");
    let sql_aabb = format!(
        "SELECT VALUE count() FROM inst_relate_aabb WHERE in IN [{}];",
        pe_list_str
    );
    match SUL_DB.query_take::<Vec<u64>>(&sql_aabb, 0).await {
        Ok(results) => {
            let cnt = results.first().copied().unwrap_or(0);
            println!("   count = {} {}", cnt, if cnt > 0 { "✅" } else { "⚠️" });
        }
        Err(e) => println!("   Error: {} ❌", e),
    }

    // 7. 验证实际写入的记录详情（用于调试）
    println!("\n7. 检查实际写入的 inst_relate 记录 (pe:`24381_145018`):");
    let sql_detail = format!(
        "SELECT in, out, owner FROM inst_relate WHERE in = {} LIMIT 5;",
        bran_refno.to_pe_key()
    );
    match SUL_DB
        .query_take::<Vec<serde_json::Value>>(&sql_detail, 0)
        .await
    {
        Ok(results) => {
            println!("   找到 {} 条记录", results.len());
            for (i, record) in results.iter().enumerate() {
                println!(
                    "   记录 {}: in={:?}, out={:?}, owner={:?}",
                    i + 1,
                    record.get("in"),
                    record.get("out"),
                    record.get("owner")
                );
            }
            if !results.is_empty() {
                println!("   ✅ 数据验证通过");
            } else {
                println!("   ⚠️ 未找到记录，可能数据未写入或 pe key 格式不正确");
            }
        }
        Err(e) => println!("   Error: {} ❌", e),
    }

    // 8. 验证 tubi_relate 详细信息
    println!("\n8. 检查 tubi_relate 详细信息 (使用 ID Range):");
    let sql_tubi_detail = format!(
        "SELECT id[0] as bran_refno, id[1] as index, in as leave, out as arrive FROM tubi_relate:[{}, 0]..[{}, ..] LIMIT 5;",
        bran_pe_key, bran_pe_key
    );
    match SUL_DB
        .query_take::<Vec<serde_json::Value>>(&sql_tubi_detail, 0)
        .await
    {
        Ok(results) => {
            println!("   找到 {} 条记录", results.len());
            for (i, record) in results.iter().enumerate() {
                println!(
                    "   记录 {}: bran={:?}, index={:?}, leave={:?}, arrive={:?}",
                    i + 1,
                    record.get("bran_refno"),
                    record.get("index"),
                    record.get("leave"),
                    record.get("arrive")
                );
            }
            if !results.is_empty() {
                println!("   ✅ tubi_relate 数据验证通过");
            }
        }
        Err(e) => println!("   Error: {} ❌", e),
    }

    // 9. 检查 pe 表是否存在该记录（验证 pe key 格式）
    println!("\n9. 验证 pe 表记录是否存在:");
    let sql_pe = format!("SELECT id, noun, name FROM {};", bran_refno.to_pe_key());
    match SUL_DB
        .query_take::<Vec<serde_json::Value>>(&sql_pe, 0)
        .await
    {
        Ok(results) => {
            if let Some(record) = results.first() {
                println!(
                    "   ✅ pe 记录存在: id={:?}, noun={:?}, name={:?}",
                    record.get("id"),
                    record.get("noun"),
                    record.get("name")
                );
            } else {
                println!("   ⚠️ pe 记录不存在，pe key 可能不正确");
            }
        }
        Err(e) => println!("   Error: {} ❌", e),
    }

    println!("\n========== 验证结束 ==========\n");
    Ok(())
}
