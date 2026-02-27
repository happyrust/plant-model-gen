use aios_core::*;
use anyhow::Result;

/// 诊断 WALL 17496/106224 的 FIXING 布尔运算状态
///
/// 用法: cargo run --example diagnose_fixing_boolean
#[tokio::main]
async fn main() -> Result<()> {
    init_surreal().await?;
    println!("═══════════════════════════════════════════════════");
    println!("  WALL 17496/106224 FIXING 布尔诊断");
    println!("═══════════════════════════════════════════════════");

    let wall_refno = RefnoEnum::from("17496_106224");

    // 1. 查询 WALL 的子孙 FIXING（通过 contains 关系遍历）
    let sql_fixings = r#"
        SELECT value out FROM pe:⟨17496_106224⟩->contains->?->contains->?->contains
        WHERE out.noun = 'FIXING'
    "#;
    let fixings: Vec<serde_json::Value> = SUL_DB.query_take(sql_fixings, 0).await.unwrap_or_default();
    println!("\n📋 [方法1] WALL 子孙 FIXING (via contains): {} 个", fixings.len());
    for f in &fixings {
        println!("   {}", f);
    }

    // 1b. 直接查询已知 FIXING 的 owner chain
    let fixing_refnos = [
        "17496_125257", "17496_142089", "17496_142092", "17496_142459",
        "17496_152124", "17496_152127", "17496_152153",
    ];
    println!("\n📋 [方法2] 各 FIXING 的 owner 链:");
    for r in &fixing_refnos {
        let sql = format!(
            "SELECT noun, OWNR FROM pe:⟨{}⟩", r
        );
        let result: Vec<serde_json::Value> = SUL_DB.query_take(&sql, 0).await.unwrap_or_default();
        println!("   {} → {:?}", r, result.first());
    }

    // 2. 查询 WALL 的 ngmr_relate（交叉负实体关系）
    let sql_ngmr = r#"
        SELECT 
            in AS geo_relate_id,
            in.geo_type AS geo_type,
            in.trans != NONE AS has_trans,
            pe AS carrier_pe
        FROM pe:⟨17496_106224⟩<-ngmr_relate
    "#;
    let ngmr_results: Vec<serde_json::Value> = SUL_DB.query_take(sql_ngmr, 0).await.unwrap_or_default();
    println!("\n📋 WALL 的 ngmr_relate 记录: {} 条", ngmr_results.len());
    for (i, r) in ngmr_results.iter().enumerate() {
        println!("   [{}] {}", i, r);
    }

    // 2b. 也检查 pe:'17496_106224' 格式
    let sql_ngmr2 = r#"
        SELECT count() AS cnt FROM ngmr_relate WHERE out = pe:⟨17496_106224⟩
    "#;
    let ngmr_count: Vec<serde_json::Value> = SUL_DB.query_take(sql_ngmr2, 0).await.unwrap_or_default();
    println!("   ngmr_relate WHERE out=wall: {:?}", ngmr_count.first());

    // 3. 查询 WALL 的 neg_relate（直接负实体关系）
    let sql_neg = r#"
        SELECT count() AS cnt FROM neg_relate WHERE out = pe:⟨17496_106224⟩
    "#;
    let neg_results: Vec<serde_json::Value> = SUL_DB.query_take(sql_neg, 0).await.unwrap_or_default();
    println!("\n📋 WALL 的 neg_relate: {:?}", neg_results.first());

    // 4. 查询各 FIXING 的 geo_relate 中 CateNeg 类型
    println!("\n📋 各 FIXING 的 CateNeg geo_relate:");
    for r in &fixing_refnos {
        let sql = format!(
            r#"SELECT geo_type, trans != NONE AS has_trans, out AS geo FROM pe:⟨{}⟩->geo_relate WHERE geo_type IN ['CateNeg', 'CataCrossNeg', 'Neg', 'Compound']"#,
            r
        );
        let result: Vec<serde_json::Value> = SUL_DB.query_take(&sql, 0).await.unwrap_or_default();
        if !result.is_empty() {
            println!("   {} → {} 条: {:?}", r, result.len(), result);
        } else {
            println!("   {} → 0 条 (无负实体几何)", r);
        }
    }

    // 5. 查询 WALL 的 inst_relate 状态
    let sql_inst = r#"
        SELECT 
            in AS pe_id,
            in.noun AS noun,
            bool_status,
            has_cata_neg
        FROM inst_relate:⟨17496_106224⟩
    "#;
    let inst_results: Vec<serde_json::Value> = SUL_DB.query_take(sql_inst, 0).await.unwrap_or_default();
    println!("\n📋 WALL inst_relate 状态: {:?}", inst_results);

    // 5b. 也检查 select *
    let sql_inst2 = r#"SELECT * FROM inst_relate:⟨17496_106224⟩"#;
    let inst_results2: Vec<serde_json::Value> = SUL_DB.query_take(sql_inst2, 0).await.unwrap_or_default();
    println!("   inst_relate raw: {} 条", inst_results2.len());
    for r in &inst_results2 {
        println!("   {}", r);
    }

    // 6. 各 FIXING 的 pe_transform
    println!("\n📋 各 FIXING 的 pe_transform world_trans:");
    for r in &fixing_refnos {
        let sql = format!(
            "SELECT world_trans.d != NONE AS has_wt FROM pe_transform:⟨{}⟩",
            r
        );
        let result: Vec<serde_json::Value> = SUL_DB.query_take(&sql, 0).await.unwrap_or_default();
        println!("   {} → {:?}", r, result.first());
    }

    // 7. 使用布尔查询相同的 SQL 查 ngmr_relate
    println!("\n📋 [布尔查询复现] ngmr_relate (同 boolean_query_optimized.rs):");
    let sql_bool = r#"
        SELECT 
            in.out AS id,
            in.geo_type AS geo_type,
            in.para_type ?? "" AS para_type,
            in.trans.d != NONE AS has_trans,
            pe AS carrier_pe,
            type::record("pe_transform", record::id(pe)).world_trans.d != NONE AS has_carrier_wt
        FROM pe:⟨17496_106224⟩<-ngmr_relate
        WHERE in.trans.d != NONE
    "#;
    let bool_results: Vec<serde_json::Value> = SUL_DB.query_take(sql_bool, 0).await.unwrap_or_default();
    println!("   有效记录 (WHERE in.trans.d != NONE): {} 条", bool_results.len());
    for (i, r) in bool_results.iter().enumerate() {
        println!("   [{}] {}", i, r);
    }

    // 不加 WHERE 的版本
    let sql_bool_all = r#"
        SELECT 
            id,
            in AS geo_relate_in,
            out AS target_out,
            pe AS carrier_pe,
            in.trans != NONE AS geo_has_trans
        FROM pe:⟨17496_106224⟩<-ngmr_relate
    "#;
    let all_ngmr: Vec<serde_json::Value> = SUL_DB.query_take(sql_bool_all, 0).await.unwrap_or_default();
    println!("   全部记录 (无 WHERE): {} 条", all_ngmr.len());
    for (i, r) in all_ngmr.iter().enumerate() {
        println!("   [{}] {}", i, r);
    }

    // 8. 直接扫 ngmr_relate 全表
    println!("\n📋 ngmr_relate 全表扫描:");
    let sql_all_ngmr = r#"SELECT id, in, out, pe FROM ngmr_relate"#;
    let all_records: Vec<serde_json::Value> = SUL_DB.query_take(sql_all_ngmr, 0).await.unwrap_or_default();
    println!("   总记录数: {}", all_records.len());
    for (i, r) in all_records.iter().enumerate() {
        println!("   [{}] {}", i, r);
    }

    // 9. 检查 inst_relate 全表
    println!("\n📋 inst_relate 全表扫描 (LIMIT 20):");
    let sql_all_inst = r#"SELECT id, in, in.noun AS noun, has_cata_neg, bool_status FROM inst_relate LIMIT 20"#;
    let all_inst: Vec<serde_json::Value> = SUL_DB.query_take(sql_all_inst, 0).await.unwrap_or_default();
    println!("   记录数: {}", all_inst.len());
    for (i, r) in all_inst.iter().enumerate() {
        println!("   [{}] {}", i, r);
    }

    // 10. 检查 geo_relate 全表 CataCrossNeg
    println!("\n📋 geo_relate 中 CataCrossNeg 类型:");
    let sql_ccn = r#"SELECT id, in AS pe, out AS geo, geo_type, trans != NONE AS has_trans FROM geo_relate WHERE geo_type = 'CataCrossNeg' LIMIT 20"#;
    let ccn_records: Vec<serde_json::Value> = SUL_DB.query_take(sql_ccn, 0).await.unwrap_or_default();
    println!("   记录数: {}", ccn_records.len());
    for (i, r) in ccn_records.iter().enumerate() {
        println!("   [{}] {}", i, r);
    }

    println!("\n═══════════════════════════════════════════════════");
    Ok(())
}
