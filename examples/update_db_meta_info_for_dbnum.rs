//! 为 Full Noun / tree_query 生成补齐 output/scene_tree/db_meta_info.json 的 ref0->dbnum 映射
//!
//! 背景：
//! - Full Noun 新管线在做 descendants 查询时会走 tree_query 的 cache-only 路径；
//! - 若 db_meta_info.json 缺少 ref0->dbnum 映射，会报：
//!   "无法从缓存推导 refno 的 dbnum（cache-only 不回退 SurrealDB）"
//!
//! 用法：
//! - cargo run --example update_db_meta_info_for_dbnum --features sqlite-index -- --config DbOption-room-gen-7999 --dbnum 7999

use aios_core::{SUL_DB, SurrealQueryExt, init_surreal};
use anyhow::{Context, Result};
use serde_json::{Value, json};
use std::path::Path;

fn upsert_ref0_to_dbnum(meta: &mut Value, ref0: u32, dbnum: u32) -> Result<bool> {
    let obj = meta
        .get_mut("ref0_to_dbnum")
        .and_then(|v| v.as_object_mut())
        .context("db_meta_info.json 缺少 ref0_to_dbnum 对象")?;
    let key = ref0.to_string();
    let existed = obj.contains_key(&key);
    obj.insert(key, json!(dbnum));
    Ok(!existed)
}

fn parse_ref0_from_refno_str(s: &str) -> Option<u32> {
    // 兼容 24383_72444 / 24383/72444
    let s = s.trim();
    let head = s.split(|c| c == '_' || c == '/').next()?;
    head.parse::<u32>().ok()
}

#[tokio::main]
async fn main() -> Result<()> {
    let mut args = std::env::args().skip(1);

    let mut config = "db_options/DbOption".to_string();
    let mut target_dbnum: Option<u32> = None;
    let mut target_refno: Option<String> = None;

    while let Some(a) = args.next() {
        match a.as_str() {
            "--config" => {
                config = args
                    .next()
                    .context("--config 需要提供配置名（不带 .toml）")?;
            }
            "--dbnum" => {
                let v = args.next().context("--dbnum 需要提供数字")?;
                target_dbnum = Some(v.parse::<u32>().context("解析 --dbnum 失败")?);
            }
            "--refno" => {
                target_refno = Some(args.next().context("--refno 需要提供参考号")?);
            }
            _ => {}
        }
    }

    unsafe {
        std::env::set_var("DB_OPTION_FILE", &config);
    }

    init_surreal().await.context("初始化 SurrealDB 失败")?;

    let meta_path = Path::new("output/scene_tree/db_meta_info.json");
    let mut meta: Value = if meta_path.exists() {
        let content = std::fs::read_to_string(meta_path)
            .with_context(|| format!("读取失败: {}", meta_path.display()))?;
        serde_json::from_str(&content).context("解析 db_meta_info.json 失败")?
    } else {
        json!({
            "version": 1,
            "updated_at": chrono::Utc::now().to_rfc3339(),
            "ref0_to_dbnum": {},
            "db_files": {}
        })
    };

    let mut inserted = 0usize;
    let mut total = 0usize;

    if let Some(refno_str) = target_refno.as_deref() {
        let ref0 = parse_ref0_from_refno_str(refno_str)
            .with_context(|| format!("无法从 refno 解析 ref0: {}", refno_str))?;
        let key = refno_str.replace('/', "_");
        let sql = format!("SELECT VALUE refno.dbnum FROM pe:{};", key);
        let dbnums: Vec<u32> = SUL_DB.query_take(&sql, 0).await.with_context(|| {
            format!("查询 refno.dbnum 失败（refno={}），SQL={}", refno_str, sql)
        })?;
        let dbnum = dbnums.first().copied().context("refno.dbnum 查询为空")?;

        total = 1;
        inserted = if upsert_ref0_to_dbnum(&mut meta, ref0, dbnum)? {
            1
        } else {
            0
        };

        println!(
            "🔧 按 refno 补齐映射: refno={} => ref0={} -> dbnum={}",
            refno_str, ref0, dbnum
        );
    } else {
        let target_dbnum = target_dbnum.context("必须提供 --dbnum 或 --refno")?;

        // 只取 ref_0 标量，避免 record 类型导致的 SurrealValue 解码问题。
        let sql_ref0 = format!(
            "SELECT VALUE ref_0 FROM dbnum_info_table WHERE dbnum = {};",
            target_dbnum
        );

        let ref0s: Vec<u32> = match SUL_DB.query_take(&sql_ref0, 0).await {
            Ok(v) => v,
            Err(e) => {
                // 兼容：若表中没有 ref_0 字段，则退化为从 record id 提取（record::id(id)）
                let sql_id = format!(
                    "SELECT VALUE record::id(id) FROM dbnum_info_table WHERE dbnum = {};",
                    target_dbnum
                );
                let ids: Vec<u32> = SUL_DB
                    .query_take(&sql_id, 0)
                    .await
                    .with_context(|| format!("查询 dbnum_info_table 失败: {e}"))?;
                ids
            }
        };

        if ref0s.is_empty() {
            anyhow::bail!(
                "dbnum_info_table 未返回任何记录（dbnum={}），请确认 SurrealDB 中已存在该表/数据",
                target_dbnum
            );
        }

        for ref0 in &ref0s {
            total += 1;
            if upsert_ref0_to_dbnum(&mut meta, *ref0, target_dbnum)? {
                inserted += 1;
            }
        }
    }

    meta["updated_at"] = json!(chrono::Utc::now().to_rfc3339());

    std::fs::create_dir_all("output/scene_tree").context("创建 output/scene_tree 失败")?;
    std::fs::write(
        meta_path,
        serde_json::to_string_pretty(&meta).context("序列化 meta 失败")?,
    )
    .with_context(|| format!("写入失败: {}", meta_path.display()))?;

    println!(
        "✅ db_meta_info.json 已更新：total_rows={} newly_inserted={}",
        total, inserted
    );

    Ok(())
}
