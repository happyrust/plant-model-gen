//! 纯房间计算：指定 PANE(panel) refno，计算属于该房间面板的构件集合。
//!
//! 本 example 只做计算，不触发任何模型生成。
//! 前置条件：模型数据（inst_relate、geo_relate、inst_relate_aabb 等）已在 SurrealDB 中就绪。
//!
//! 运行示例：
//!   cargo run --example room_calc_by_panel_demo --features "gen_model,sqlite-index" -- \
//!     --panel-refno 24381/35798 \
//!     --expect-refnos 24381/145019 \
//!     --dboption-path db_options/DbOption
//!
//! 回归测试（panel 24381/35798 应包含弯头 24381/145019）：
//!   cargo run --example room_calc_by_panel_demo -- \
//!     --panel-refno 24381/35798 \
//!     --expect-refnos 24381/145019
//!
//! 可选环境变量（仅影响日志输出）：
//! - AIOS_LOG_TO_CONSOLE=1：将日志同时输出到控制台（默认只写文件）
//! - AIOS_ROOM_DEBUG=1：输出房间计算中间步骤

use aios_core::{RecordId, RefnoEnum, SUL_DB, SurrealQueryExt, init_surreal};
use anyhow::{Context, Result};
use clap::Parser;
use serde_json::json;
use std::collections::HashSet;

#[derive(Debug, Parser)]
#[command(
    author,
    version,
    about = "纯房间计算：按 panel 计算房间内构件（不触发模型生成）"
)]
struct Args {
    /// 待测 panel refno（格式：24381/35798）
    #[arg(long, default_value = "17496/199296")]
    panel_refno: String,

    /// DbOption 配置名/路径（不带 .toml 后缀亦可）
    #[arg(long, default_value = "db_options/DbOption")]
    dboption_path: String,

    /// 房间包含容差（默认 0.1）
    #[arg(long, default_value_t = 0.1)]
    inside_tol: f32,

    /// 逗号/分号/空白分隔的 refno 列表；若有任一不在结果中则退出码非 0
    #[arg(long, default_value = "")]
    expect_refnos: String,

    /// 直接指定房间号字符串；提供后将跳过 PANE->SBFR->FRMW 查询
    #[arg(long)]
    room_num: Option<String>,

    /// 是否使用缓存查询 inst 信息（默认 false，走 SurrealDB 直查）
    #[arg(
        long,
        default_value_t = false,
        value_parser = clap::builder::BoolishValueParser::new(),
        action = clap::ArgAction::Set
    )]
    room_use_cache: bool,

    /// 是否写入 SurrealDB 的 room_relate（含 DELETE + RELATE）
    #[arg(long, default_value_t = false)]
    write_db: bool,
}

fn parse_refno_list(raw: &str) -> anyhow::Result<Vec<RefnoEnum>> {
    let mut out = Vec::new();
    for s in raw.split(|c: char| c == ',' || c == ';' || c.is_whitespace()) {
        let s = s.trim();
        if s.is_empty() {
            continue;
        }
        let r = RefnoEnum::from(s);
        anyhow::ensure!(r.is_valid(), "无效的 refno：{}", s);
        out.push(r);
    }
    Ok(out)
}

fn pe_key(refno: RefnoEnum) -> String {
    format!("pe:`{}`", refno.to_string())
}

fn record_id_to_surreal_literal(id: &RecordId) -> String {
    let v = serde_json::to_value(id).unwrap_or_else(|_| json!({}));
    let table = v.get("table").and_then(|x| x.as_str()).unwrap_or_default();
    let key = v.get("key");
    if let Some(key) = key {
        if let Some(s) = key.get("String").and_then(|x| x.as_str()) {
            return format!("{table}:`{s}`");
        }
        if let Some(n) = key.get("Number").and_then(|x| x.as_i64()) {
            return format!("{table}:{n}");
        }
        if let Some(n) = key.get("Int").and_then(|x| x.as_i64()) {
            return format!("{table}:{n}");
        }
    }
    format!("{v}")
}

#[tokio::main]
async fn main() -> Result<()> {
    let args = Args::parse();

    // 1) 解析输入
    let panel_refno = RefnoEnum::from(args.panel_refno.as_str());
    anyhow::ensure!(
        panel_refno.is_valid(),
        "无效的 --panel-refno: {}",
        args.panel_refno
    );
    let inside_tol = args.inside_tol;
    let mut room_num = args.room_num.unwrap_or_default();
    if room_num.trim().is_empty() {
        room_num.clear();
    }
    let expect_raw = args.expect_refnos;
    let expect_refnos = if expect_raw.trim().is_empty() {
        Vec::new()
    } else {
        parse_refno_list(&expect_raw)?
    };

    // 2) 加载配置
    let dbopt_path = args.dboption_path;
    let db_option_ext = aios_database::options::get_db_option_ext_from_path(&dbopt_path)
        .with_context(|| format!("加载配置失败: {}", dbopt_path))?;
    unsafe {
        std::env::set_var("DB_OPTION_FILE", &dbopt_path);
    }

    aios_database::init_logging(true);

    let foyer_cache_dir = db_option_ext
        .get_foyer_cache_dir()
        .to_string_lossy()
        .to_string();

    println!("🎯 panel: {}", panel_refno);
    println!("   - inside_tol: {}", inside_tol);
    println!("   - room_use_cache: {}", args.room_use_cache);
    println!("   - write_db: {}", args.write_db);

    // 3) 设置房间计算数据源
    unsafe {
        std::env::set_var(
            "AIOS_ROOM_USE_CACHE",
            if args.room_use_cache { "1" } else { "0" },
        );
        std::env::set_var("FOYER_CACHE_DIR", &foyer_cache_dir);
    }

    // 4) 初始化 SurrealDB（房间计算查询 + ensure_spatial_index_ready 均需要）
    init_surreal().await.context("初始化 SurrealDB 失败")?;

    // 诊断：各关键表行数
    {
        let tables = [
            "pe",
            "inst_relate",
            "inst_relate_aabb",
            "geo_relate",
            "inst_geo",
        ];
        for t in &tables {
            let sql = format!("SELECT count() as cnt FROM {t} GROUP ALL");
            let r: Vec<serde_json::Value> = SUL_DB.query_take(&sql, 0).await.unwrap_or_default();
            let cnt = r
                .first()
                .and_then(|v| v.get("cnt"))
                .and_then(|v| v.as_i64())
                .unwrap_or(0);
            println!("[diag] {t}: {cnt} rows");
        }
    }

    // 5) 获取 room_num（仅写库需要）
    let mut sbfr_pe = String::new();
    let mut frmw_pe = String::new();
    if args.write_db && room_num.is_empty() {
        let panel_pe = pe_key(panel_refno);
        let sbfr_sql = format!(
            "SELECT VALUE OWNER FROM PANE WHERE REFNO = {} LIMIT 1",
            panel_pe
        );
        let sbfr_ids: Vec<RecordId> = SUL_DB.query_take(&sbfr_sql, 0).await.unwrap_or_default();
        sbfr_pe = sbfr_ids
            .first()
            .map(record_id_to_surreal_literal)
            .unwrap_or_default();
        anyhow::ensure!(
            !sbfr_pe.is_empty(),
            "未找到 PANE.OWNER(SBFR) : {}",
            panel_refno
        );

        let frmw_sql = format!(
            "SELECT VALUE OWNER FROM SBFR WHERE REFNO = {} LIMIT 1",
            sbfr_pe
        );
        let frmw_ids: Vec<RecordId> = SUL_DB.query_take(&frmw_sql, 0).await.unwrap_or_default();
        frmw_pe = frmw_ids
            .first()
            .map(record_id_to_surreal_literal)
            .unwrap_or_default();
        anyhow::ensure!(
            !frmw_pe.is_empty(),
            "未找到 SBFR.OWNER(FRMW) : SBFR={}",
            sbfr_pe
        );

        let room_num_sql = format!(
            "SELECT VALUE array::last(string::split(NAME, '-')) FROM FRMW WHERE REFNO = {} LIMIT 1",
            frmw_pe
        );
        let room_nums: Vec<String> = SUL_DB
            .query_take(&room_num_sql, 0)
            .await
            .unwrap_or_default();
        room_num = room_nums.first().cloned().unwrap_or_default();
        anyhow::ensure!(
            !room_num.is_empty(),
            "未能从 FRMW.NAME 解析房间号: FRMW={}",
            frmw_pe
        );

        println!("   - SBFR(pe): {}", sbfr_pe);
        println!("   - FRMW(pe): {}", frmw_pe);
        println!("   - room_num: {}", room_num);
    }

    // 6) 纯房间计算
    let mesh_dir = db_option_ext.inner.get_meshes_path();
    let exclude = HashSet::<RefnoEnum>::new();

    // 粗算诊断：在期望 refnos 时先验证 AABB 相交查询是否正确
    if !expect_refnos.is_empty() {
        println!("\n📐 粗算（AABB 相交）诊断:");
        match aios_database::fast_model::diagnose_coarse_aabb_intersection(
            panel_refno,
            &expect_refnos,
        )
        .await
        {
            Ok(diag) => {
                if let Some(pa) = &diag.panel_aabb {
                    println!(
                        "   panel_aabb: ({:.2},{:.2},{:.2})..({:.2},{:.2},{:.2})",
                        pa.mins.x, pa.mins.y, pa.mins.z, pa.maxs.x, pa.maxs.y, pa.maxs.z
                    );
                } else {
                    println!("   panel_aabb: (缺失)");
                }
                if let Some(qa) = &diag.query_aabb {
                    println!(
                        "   query_aabb: ({:.2},{:.2},{:.2})..({:.2},{:.2},{:.2})",
                        qa.mins.x, qa.mins.y, qa.mins.z, qa.maxs.x, qa.maxs.y, qa.maxs.z
                    );
                }
                for (r, aabb_opt, intersects) in &diag.expect_refno_aabb_intersects {
                    let aabb_str = aabb_opt
                        .as_ref()
                        .map(|a| {
                            format!(
                                "({:.1},{:.1},{:.1})..({:.1},{:.1},{:.1})",
                                a.mins.x, a.mins.y, a.mins.z, a.maxs.x, a.maxs.y, a.maxs.z
                            )
                        })
                        .unwrap_or_else(|| "缺失".to_string());
                    println!(
                        "   expect {}: aabb={} 与query_aabb相交={}",
                        r, aabb_str, intersects
                    );
                }
                for (r, in_rtree) in &diag.expect_refno_in_rtree {
                    println!("   expect {} 在RTree候选列表: {}", r, in_rtree);
                }
                println!("   RTree 候选总数: {}", diag.rtree_candidates.len());
                let all_ok = diag
                    .expect_refno_in_rtree
                    .iter()
                    .all(|(_, in_rtree)| *in_rtree);
                if all_ok {
                    println!("   ✅ 粗算通过：所有 expect_refno 均在候选列表中");
                } else {
                    println!("   ❌ 粗算异常：部分 expect_refno 未进入候选（将导致细算漏判）");
                }
            }
            Err(e) => println!("   粗算诊断失败: {}", e),
        }
    }

    let within = aios_database::fast_model::room_model::cal_room_refnos(
        &mesh_dir,
        panel_refno,
        &exclude,
        inside_tol,
    )
    .await
    .context("房间计算失败")?;

    println!(
        "✅ 房间计算完成: panel={}, components={}",
        panel_refno,
        within.len()
    );

    // 7) 断言
    if !expect_refnos.is_empty() {
        let mut missing = Vec::new();
        for r in &expect_refnos {
            if !within.contains(r) {
                missing.push(r.clone());
            }
        }
        if missing.is_empty() {
            println!("✅ EXPECT_REFNOS 断言通过: {}", expect_raw.trim());
        } else {
            eprintln!("❌ EXPECT_REFNOS 断言失败");
            eprintln!("   - panel: {}", panel_refno);
            eprintln!("   - inside_tol: {}", inside_tol);
            eprintln!("   - within_count: {}", within.len());
            eprintln!("   - missing_refnos(count={}):", missing.len());
            for r in &missing {
                eprintln!("     - {}", r);
            }
            anyhow::bail!("EXPECT_REFNOS 缺失 {} 项", missing.len());
        }
    }

    // 不落库时打印结果摘要
    if !args.write_db {
        for (idx, r) in within.iter().take(30).enumerate() {
            println!("   - [{}] {}", idx + 1, r);
        }
        if within.len() > 30 {
            println!("   ... 还有 {} 条未打印", within.len() - 30);
        }
        return Ok(());
    }

    // 8) 写入 room_relate
    anyhow::ensure!(
        !room_num.is_empty(),
        "未提供 ROOM_NUM 且无法获取房间号（写库需要 room_num）"
    );
    let delete_sql = format!(
        "LET $ids = SELECT VALUE id FROM [{}]->room_relate;\nDELETE $ids;",
        panel_refno.to_pe_key()
    );
    SUL_DB.query(&delete_sql).await?;

    if !within.is_empty() {
        let room_num_escaped = room_num.replace('\'', "''");
        let mut sql_statements = Vec::with_capacity(within.len());
        for refno in &within {
            let relation_id = format!("{}_{}", panel_refno, refno);
            sql_statements.push(format!(
                "relate {}->room_relate:{}->{} set room_num='{}', confidence=0.9, created_at=time::now();",
                panel_refno.to_pe_key(),
                relation_id,
                refno.to_pe_key(),
                room_num_escaped
            ));
        }
        SUL_DB.query(sql_statements.join("\n")).await?;
    }

    let relate_sql = format!(
        "SELECT VALUE [out, room_num] FROM room_relate WHERE `in` = {}",
        panel_refno.to_pe_key()
    );
    let rows: Vec<(RecordId, String)> = SUL_DB.query_take(&relate_sql, 0).await.unwrap_or_default();
    let mut within_from_db = HashSet::<RefnoEnum>::new();
    let mut got_room_num = String::new();
    for (out_id, rn) in rows {
        within_from_db.insert(RefnoEnum::from(out_id));
        if got_room_num.is_empty() && !rn.is_empty() {
            got_room_num = rn;
        }
    }

    println!(
        "📌 room_relate(panel -> components): count={}, room_num={}",
        within_from_db.len(),
        got_room_num
    );
    for (idx, r) in within_from_db.iter().take(30).enumerate() {
        println!("   - [{}] {}", idx + 1, r);
    }
    if within_from_db.len() > 30 {
        println!("   ... 还有 {} 条未打印", within_from_db.len() - 30);
    }

    Ok(())
}
