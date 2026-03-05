//! 通过 “指定 PANE(panel) refno” 找到其所属房间号，并仅重建该房间的 room_relate。
//!
//! 用途：
//! - 验证 `output/spatial_index.sqlite` 是否可用于房间计算粗筛
//! - 便于针对单个房间快速回归（避免全库 room compute）
//!
//! 运行示例：
//!   cargo run --example room_calc_by_panel_demo --features "gen_model,sqlite-index" -- `
//!     --panel-refno 17496/199296 `
//!     --dboption-path DbOption-room-pane17496-cache `
//!     --auto-gen-model true
//!
//! 回归测试（panel 24381/35798 应包含弯头 24381/145019）：
//!   cargo run --example room_calc_by_panel_demo -- \
//!     --panel-refno 24381/35798 \
//!     --expect-refnos 24381/145019
//!
//! 可选环境变量（仅影响日志输出）：
//! - AIOS_LOG_TO_CONSOLE=1：将日志同时输出到控制台（默认只写文件）
//!
//! 注意：
//! - 仅当启用 `--write-db` 时才会连接 SurrealDB（用于查询 PANE->SBFR->FRMW->ROOM_NUM，并写入 room_relate）
//! - 若启用 `--auto-gen-model true`，会先通过 Foyer Cache 自动生成模型/mesh/空间索引，再做房间计算

use aios_core::{RecordId, RefnoEnum, SUL_DB, SurrealQueryExt, init_surreal};
use anyhow::{Context, Result};
use clap::Parser;
use serde_json::{Value, json};
use std::collections::HashSet;
use std::fs;
use std::path::PathBuf;

#[derive(Debug, Parser)]
#[command(
    author,
    version,
    about = "房间计算回归：按 panel 计算房间内构件并可选落库"
)]
struct Args {
    /// 待测 panel refno（格式：24381/35798）
    #[arg(long, default_value = "17496/199296")]
    panel_refno: String,

    /// DbOption 配置名/路径（不带 .toml 后缀亦可）
    #[arg(long, default_value = "db_options/DbOption")]
    dboption_path: String,

    /// 是否自动生成模型/mesh/空间索引（Foyer Cache）后再做房间计算
    ///
    /// - 建议默认关（false），避免一次跑触发全量 gen_model，耗时极长。
    /// - 需要时再显式开启：`--auto-gen-model true`
    #[arg(
        long,
        default_value_t = false,
        value_parser = clap::builder::BoolishValueParser::new(),
        action = clap::ArgAction::Set
    )]
    auto_gen_model: bool,

    /// 指定 dbnum（注意：ref0 ≠ dbnum）。
    ///
    /// - 不传时：会尝试从 `output/<project>/scene_tree/db_meta_info.json` 的 `ref0_to_dbnum` 映射推导；
    /// - 若映射缺失：将直接报错（不会回退使用 ref0），请显式传 `--dbnum`。
    #[arg(long)]
    dbnum: Option<u32>,

    /// 房间包含容差（默认 0.1）
    #[arg(long, default_value_t = 0.1)]
    inside_tol: f32,

    /// 逗号/分号/空白分隔的 refno 列表；若有任一不在结果中则退出码非 0
    #[arg(long, default_value = "")]
    expect_refnos: String,

    /// 直接指定房间号字符串；提供后将跳过 PANE->SBFR->FRMW 查询
    #[arg(long)]
    room_num: Option<String>,

    /// 强制设置 FOYER_CACHE_DIR（不提供则使用配置推导的默认目录）
    #[arg(long)]
    foyer_cache_dir: Option<String>,

    /// 是否使用缓存查询 inst 信息（默认 true；传 false 走 SurrealDB 直查）
    ///
    /// 示例：`--room-use-cache false`
    #[arg(
        long,
        default_value_t = true,
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

fn try_map_dbnum_from_project_meta(
    db_option_ext: &aios_database::options::DbOptionExt,
    ref0: u32,
) -> Option<u32> {
    let meta_path = db_option_ext
        .get_project_output_dir()
        .join("scene_tree")
        .join("db_meta_info.json");
    let raw = fs::read_to_string(meta_path).ok()?;
    let v: Value = serde_json::from_str(&raw).ok()?;
    let map = v.get("ref0_to_dbnum")?.as_object()?;
    let dbnum = map.get(&ref0.to_string())?.as_u64()?;
    u32::try_from(dbnum).ok()
}

async fn ensure_tree_index_by_parse(
    db_option_ext: &aios_database::options::DbOptionExt,
    dbnum: u32,
) -> anyhow::Result<PathBuf> {
    use aios_database::versioned_db::database::sync_pdms;

    let tree_path = db_option_ext
        .get_project_output_dir()
        .join("scene_tree")
        .join(format!("{dbnum}.tree"));

    if tree_path.exists() {
        return Ok(tree_path);
    }

    println!("📂 检测到 TreeIndex 缺失: {}", tree_path.display());
    println!("🔄 正在通过 PDMS 解析生成 TreeIndex (gen_tree_only 模式)...");
    println!("📡 初始化 SurrealDB（sync_pdms 内部会执行索引/事件优化逻辑）...");
    aios_core::init_surreal()
        .await
        .context("初始化 SurrealDB 失败（sync_pdms 依赖）")?;

    let mut parse_option = db_option_ext.inner.clone();
    parse_option.gen_tree_only = true;
    parse_option.total_sync = true;
    parse_option.manual_db_nums = Some(vec![dbnum]);
    parse_option.save_db = Some(false); // 不写入 SurrealDB

    sync_pdms(&parse_option)
        .await
        .with_context(|| format!("TreeIndex 生成失败: dbnum={dbnum}"))?;

    anyhow::ensure!(
        tree_path.exists(),
        "TreeIndex 生成完成但未找到文件：{}",
        tree_path.display()
    );

    println!("✅ TreeIndex 生成完成: {}", tree_path.display());
    Ok(tree_path)
}

fn get_ref0_from_refno_enum(r: RefnoEnum) -> u32 {
    match r {
        RefnoEnum::Refno(x) => x.get_0(),
        RefnoEnum::SesRef(x) => x.refno.get_0(),
    }
}

// SurrealDB 里 pe 的 id 是字符串（形如 `17496_199296`），需要用反引号包裹，避免被解析为带下划线的数字字面量。
fn pe_key(refno: RefnoEnum) -> String {
    format!("pe:`{}`", refno.to_string())
}

fn record_id_to_surreal_literal(id: &RecordId) -> String {
    // aios_core::RecordId 的 serde 表示通常形如：
    // {"table":"pe","key":{"String":"17496_199295"}}
    // 这里将其转换为 Surreal 可直接识别的字面量：pe:`17496_199295`
    let v = serde_json::to_value(id).unwrap_or_else(|_| json!({}));
    let table = v.get("table").and_then(|x| x.as_str()).unwrap_or_default();
    let key = v.get("key");
    if let Some(key) = key {
        if let Some(s) = key.get("String").and_then(|x| x.as_str()) {
            return format!("{table}:`{s}`");
        }
        // 兼容数字 key（如果有）
        if let Some(n) = key.get("Number").and_then(|x| x.as_i64()) {
            return format!("{table}:{n}");
        }
        if let Some(n) = key.get("Int").and_then(|x| x.as_i64()) {
            return format!("{table}:{n}");
        }
    }

    // 兜底：尽量返回一个能用于日志的字符串
    format!("{v}")
}

#[tokio::main]
async fn main() -> Result<()> {
    let args = Args::parse();

    // 1) 解析输入 panel refno
    let panel_refno = RefnoEnum::from(args.panel_refno.as_str());
    anyhow::ensure!(
        panel_refno.is_valid(),
        "无效的 --panel-refno: {}",
        args.panel_refno
    );

    // 额外参数
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
    let mut db_option_ext = aios_database::options::get_db_option_ext_from_path(&dbopt_path)
        .with_context(|| format!("加载配置失败: {}", dbopt_path))?;

    // aios_core 的 init_surreal 使用 DB_OPTION_FILE 选择配置文件；
    // 示例里 DBOPTION_PATH 仅用于加载 DbOptionExt，因此这里同步设置以避免“看似没用到配置”的困惑。
    unsafe {
        std::env::set_var("DB_OPTION_FILE", &dbopt_path);
    }

    // 3) 推导 dbnum：优先用用户输入；否则从项目输出目录的 db_meta_info.json 做 ref0->dbnum 映射。
    // 说明：形如 `24381/35798` 的左半边通常是 ref0（高 32 位），并非最终 dbnum。
    // 约束：映射缺失时直接报错（不允许回退用 ref0 当 dbnum）。
    let dbnum = if let Some(dbnum) = args.dbnum {
        dbnum
    } else {
        let ref0 = get_ref0_from_refno_enum(panel_refno);
        if let Some(dbnum) = try_map_dbnum_from_project_meta(&db_option_ext, ref0) {
            dbnum
        } else {
            // 兜底：尽力用 DbMetaManager（若其加载路径与当前项目不一致，可能仍会失败）
            let meta = aios_database::data_interface::db_meta();
            meta.ensure_loaded().ok();
            if let Some(dbnum) = meta.get_dbnum_by_ref0(ref0) {
                dbnum
            } else {
                anyhow::bail!(
                    "未能由 ref0={} 推导 dbnum（db_meta_info.json / DbMetaManager 均缺失）。请显式传 `--dbnum <真实dbnum>`。",
                    ref0
                );
            }
        }
    };

    // 诊断用：输出房间计算内部 info 日志（默认只写文件；如需同时输出到控制台请设置 AIOS_LOG_TO_CONSOLE=1）。
    aios_database::init_logging(true);

    // cache 目录（生成/房间计算共用）
    if let Some(ref cache_dir) = args.foyer_cache_dir {
        db_option_ext.model_cache_dir = Some(cache_dir.clone());
    }
    let foyer_cache_dir = db_option_ext
        .get_foyer_cache_dir()
        .to_string_lossy()
        .to_string();

    println!("🎯 panel: {}", panel_refno);
    println!("   - dbnum: {}", dbnum);
    println!("   - auto_gen_model: {}", args.auto_gen_model);
    println!("   - write_db: {}", args.write_db);
    println!("   - inside_tol: {}", inside_tol);
    println!("   - room_use_cache: {}", args.room_use_cache);
    println!("   - foyer_cache_dir: {}", foyer_cache_dir);

    // 3.5) 初始化 SurrealDB —— ensure_spatial_index_ready / SurrealDB 直查 / write_db / auto_gen 均需要。
    init_surreal().await.context("初始化 SurrealDB 失败")?;

    // 4) 自动生成模型/mesh/空间索引（Foyer Cache）
    if args.auto_gen_model {
        println!("🔄 自动生成模型（Foyer Cache）: dbnum={}", dbnum);

        // TreeIndex 是 Full Noun / cache-only 链路的前置；缺失时先解析生成。
        ensure_tree_index_by_parse(&db_option_ext, dbnum).await?;

        db_option_ext.export_instances = true;

        db_option_ext.inner.manual_db_nums = Some(vec![dbnum]);
        db_option_ext.inner.gen_model = true;
        db_option_ext.inner.gen_mesh = true;
        db_option_ext.inner.enable_sqlite_rtree = true;
        db_option_ext.inner.replace_mesh = Some(false);
        db_option_ext.inner.save_db = Some(false);

        aios_database::fast_model::gen_all_geos_data(
            vec![],
            &db_option_ext,
            None,
            db_option_ext.target_sesno,
        )
        .await
        .context("自动生成模型失败")?;

        let idx_path = aios_database::spatial_index::SqliteSpatialIndex::default_path();
        anyhow::ensure!(
            idx_path.exists(),
            "未生成 SQLite 空间索引文件：{:?}（请确认 enable_sqlite_rtree=true 且启用 features: gen_model + sqlite-index）",
            idx_path
        );
    }

    // 5) 运行期：设置房间计算数据源开关
    unsafe {
        std::env::set_var(
            "AIOS_ROOM_USE_CACHE",
            if args.room_use_cache { "1" } else { "0" },
        );
        std::env::set_var("FOYER_CACHE_DIR", &foyer_cache_dir);
    }

    // 6) 获取 room_num（仅写库需要）
    let mut sbfr_pe = String::new();
    let mut frmw_pe = String::new();

    if args.write_db {

        if room_num.is_empty() {
            // panel -> SBFR(OWNER) -> FRMW(OWNER) -> room_num(FRMW.NAME 最后一段)
            // 注意：该项目的 PANE/SBFR/FRMW 表字段 `REFNO/OWNER` 都是 RecordId（如 pe:`17496_199296`），
            // 不是纯数字 refno。因此这里用 pe-key 做关联查询。
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
        }

        if !sbfr_pe.is_empty() {
            println!("   - SBFR(pe): {}", sbfr_pe);
        }
        if !frmw_pe.is_empty() {
            println!("   - FRMW(pe): {}", frmw_pe);
        }
        println!("   - room_num: {}", room_num);
    } else if !room_num.is_empty() {
        println!("   - room_num(ARG): {}", room_num);
    }

    // 7) 计算“该 panel”的房间构件集合
    // 6.5) Diagnostic: check panel data availability in SurrealDB
    {
        let pk = panel_refno.to_string().replace("/", "_");
        let tables = vec![
            "pe", "inst_relate", "inst_relate_aabb", "geo_relate",
            "pe_transform", "inst_geo", "inst_relate_bool",
        ];
        for t in &tables {
            let sql = format!("SELECT count() as cnt FROM {t} GROUP ALL");
            let r: Vec<serde_json::Value> = SUL_DB.query_take(&sql, 0).await.unwrap_or_default();
            let cnt = r.first().and_then(|v| v.get("cnt")).and_then(|v| v.as_i64()).unwrap_or(0);
            println!("[diag] {t}: {cnt} rows");
        }

        let sql_ir = format!("SELECT * FROM inst_relate WHERE record::id(in) = '{pk}' LIMIT 3");
        let r_ir: Vec<serde_json::Value> = SUL_DB.query_take(&sql_ir, 0).await.unwrap_or_default();
        println!("[diag] inst_relate(in has {pk}): {} rows, sample={}", r_ir.len(), serde_json::to_string(&r_ir.first()).unwrap_or_default());
    }

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
                        .map(|a| format!("({:.1},{:.1},{:.1})..({:.1},{:.1},{:.1})", a.mins.x, a.mins.y, a.mins.z, a.maxs.x, a.maxs.y, a.maxs.z))
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
                let all_ok = diag.expect_refno_in_rtree.iter().all(|(_, in_rtree)| *in_rtree);
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

    // 断言：期望 refnos 必须包含在结果中（用于回归）
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

    // 默认不落库，只打印结果摘要
    if !args.write_db {
        for (idx, r) in within.iter().take(30).enumerate() {
            println!("   - [{}] {}", idx + 1, r);
        }
        if within.len() > 30 {
            println!("   ... 还有 {} 条未打印", within.len() - 30);
        }
        return Ok(());
    }

    // 8) 覆盖写入：先删旧的，再批量写入新的（与 room_model.rs 的 save_room_relate 保持一致）。
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

    // 6) 查询该 panel 的 room_relate 结果（panel -> component，附 room_num）
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
