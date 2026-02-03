//! 检查指定 refno 的关键属性/外键（用于排查 tubi ARRIVE/LEAVE 端点缺失）。
//!
//! 用法（PowerShell）：
//!   $env:REFNOS="24381/103386,24381/103387,24381/103388,24381/103385"
//!   cargo run --example inspect_refno_catr_arri_leav
//!
//! 输出：
//! - TYPE/NAME/OWNER
//! - 是否含 ARRI/LEAV/CATR/SPRE
//! - ARRI/LEAV 值（若有）
//! - CATR/SPRE foreign_refno（若能解析）
//! - aios_core::get_cat_refno(refno) 的结果

use anyhow::Result;
use aios_core::{init_surreal, RefnoEnum, SUL_DB, SurrealQueryExt};
use std::env;
use std::str::FromStr;

fn parse_refno_list(s: &str) -> Vec<RefnoEnum> {
    s.split(',')
        .map(|x| x.trim())
        .filter(|x| !x.is_empty())
        .filter_map(|x| RefnoEnum::from_str(x).ok())
        .collect()
}

#[tokio::main]
async fn main() -> Result<()> {
    init_surreal().await?;

    let refnos_str = env::var("REFNOS")
        .unwrap_or_else(|_| "24381/103386,24381/103387,24381/103388,24381/103385".to_string());
    let refnos = parse_refno_list(&refnos_str);
    anyhow::ensure!(!refnos.is_empty(), "REFNOS 为空：{}", refnos_str);

    for r in refnos {
        println!("\n==============================");
        println!("refno={}", r);

        // 先用最底层 SurrealQL 验证 pe 记录是否存在（避免 get_named_attmap 返回 default 造成误判）
        let pe_sql = format!(
            "SELECT noun, name, record::id(owner) as owner, record::id(refno) as refno_id FROM pe:{};",
            r
        );
        match SUL_DB.query_take::<Vec<serde_json::Value>>(&pe_sql, 0).await {
            Ok(pe_rows) => {
                println!("pe_rows(len={})={}", pe_rows.len(), pe_sql);
                if let Some(row) = pe_rows.first() {
                    println!(
                        "pe_row={}",
                        serde_json::to_string_pretty(row).unwrap_or_default()
                    );
                } else {
                    println!("⚠️ pe 表中未找到该 refno（SurrealQL 0 行）");
                }
            }
            Err(e) => {
                println!("❌ SurrealQL 查询 pe 失败: {e}");
                println!("   sql={pe_sql}");
            }
        }

        // 再直接从 pe.refno（属性记录）取关键字段，绕过 get_named_attmap 的封装，验证“属性记录是否存在/是否有值”
        let att_sql = format!(
            r#"SELECT
                refno.ARRI as ARRI,
                refno.LEAV as LEAV,
                record::id(refno.SPRE) as SPRE,
                record::id(refno.CATR) as CATR
            FROM pe:{} FETCH refno;"#,
            r
        );
        match SUL_DB.query_take::<Vec<serde_json::Value>>(&att_sql, 0).await {
            Ok(rows) => {
                println!("att_rows(len={})={}", rows.len(), att_sql.replace('\n', " "));
                if let Some(row) = rows.first() {
                    println!("att_row={}", serde_json::to_string_pretty(row).unwrap_or_default());
                }
            }
            Err(e) => {
                println!("❌ SurrealQL 查询 pe.refno.* 失败: {e}");
                println!("   sql={att_sql}");
            }
        }

        let att = match aios_core::get_named_attmap(r).await {
            Ok(v) => v,
            Err(e) => {
                println!("❌ get_named_attmap failed: {e}");
                continue;
            }
        };

        println!(
            "TYPE={} NAME={} OWNER={}",
            att.get_type_str(),
            att.get_name_or_default(),
            att.get_owner()
        );

        let has_arri = att.contains_key("ARRI");
        let has_leav = att.contains_key("LEAV");
        let has_catr = att.contains_key("CATR") || att.contains_key(":CATR");
        let has_spre = att.contains_key("SPRE") || att.contains_key(":SPRE");
        println!(
            "keys: ARRI={} LEAV={} CATR={} SPRE={}",
            has_arri, has_leav, has_catr, has_spre
        );

        if has_arri {
            println!("ARRI={:?}", att.get_i32("ARRI"));
        }
        if has_leav {
            println!("LEAV={:?}", att.get_i32("LEAV"));
        }

        let catr_ref = att.get_foreign_refno("CATR");
        let spre_ref = att.get_foreign_refno("SPRE");
        println!("foreign: CATR={:?} SPRE={:?}", catr_ref, spre_ref);

        let cat_refno = aios_core::get_cat_refno(r).await;
        println!("aios_core::get_cat_refno => {:?}", cat_refno);
    }

    Ok(())
}
