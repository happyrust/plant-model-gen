//! 通过 “指定 PANE(panel) refno” 找到其所属房间号，并仅重建该房间的 room_relate。
//!
//! 用途：
//! - 验证 `output/spatial_index.sqlite` 是否可用于房间计算粗筛
//! - 便于针对单个房间快速回归（避免全库 room compute）
//!
//! 运行示例：
//!   set PANEL_REFNO=17496/199296
//!   set DBOPTION_PATH=DbOption-room-pane17496-cache
//!   cargo run --example room_calc_by_panel_demo --features "sqlite-index" -- --nocapture
//!
//! 注意：
//! - 该示例会连接 SurrealDB（用于查询 PANE->SBFR->FRMW->ROOM_NUM，并写入 room_relate）
//! - 该示例不会自动生成模型；请确保相关 mesh/索引已就绪

use anyhow::{Context, Result};
use aios_core::{RecordId, RefnoEnum, SUL_DB, SurrealQueryExt, init_surreal};
use serde_json::json;
use std::collections::HashSet;
use std::env;

fn env_or(key: &str, default: &str) -> String {
    env::var(key).unwrap_or_else(|_| default.to_string())
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
    // 1) 解析输入 panel refno
    let panel_str = env_or("PANEL_REFNO", "17496/199296");
    let panel_refno = RefnoEnum::from(panel_str.as_str());
    anyhow::ensure!(panel_refno.is_valid(), "无效的 PANEL_REFNO: {}", panel_str);

    // 2) 加载配置（仅用于 mesh_dir/room_keyword 等；示例不触发模型生成）
    let dbopt_path = env_or("DBOPTION_PATH", "DbOption");
    let mut db_option_ext = aios_database::options::get_db_option_ext_from_path(&dbopt_path)
        .with_context(|| format!("加载配置失败: {}", dbopt_path))?;
    db_option_ext.inner.gen_model = false;
    db_option_ext.inner.gen_mesh = false;

    // aios_core 的 init_surreal 使用 DB_OPTION_FILE 选择配置文件；
    // 示例里 DBOPTION_PATH 仅用于加载 DbOptionExt，因此这里同步设置以避免“看似没用到配置”的困惑。
    unsafe {
        std::env::set_var("DB_OPTION_FILE", &dbopt_path);
    }

    // 诊断用：输出房间计算内部 info 日志（默认只写文件；如需同时输出到控制台请设置 AIOS_LOG_TO_CONSOLE=1）。
    aios_database::init_logging(true);

    // 强制房间计算走 cache 查询 inst 信息（避免依赖 SurrealDB 的 inst_relate/inst_relate_aabb）。
    // 注意：room_model.rs 里通过环境变量开关，避免默认行为改变。
    unsafe {
        std::env::set_var("AIOS_ROOM_USE_CACHE", "1");
        std::env::set_var(
            "FOYER_CACHE_DIR",
            db_option_ext.get_foyer_cache_dir().to_string_lossy().to_string(),
        );
    }

    // 3) 初始化 SurrealDB
    init_surreal().await.context("初始化 SurrealDB 失败")?;

    // 4) panel -> SBFR(OWNER) -> FRMW(OWNER) -> room_num(FRMW.NAME 最后一段)
    // 注意：该项目的 PANE/SBFR/FRMW 表字段 `REFNO/OWNER` 都是 RecordId（如 pe:`17496_199296`），
    // 不是纯数字 refno。因此这里用 pe-key 做关联查询。
    let panel_pe = pe_key(panel_refno);

    let sbfr_sql = format!("SELECT VALUE OWNER FROM PANE WHERE REFNO = {} LIMIT 1", panel_pe);
    let sbfr_ids: Vec<RecordId> = SUL_DB.query_take(&sbfr_sql, 0).await.unwrap_or_default();
    let sbfr_pe = sbfr_ids
        .first()
        .map(record_id_to_surreal_literal)
        .unwrap_or_default();
    anyhow::ensure!(!sbfr_pe.is_empty(), "未找到 PANE.OWNER(SBFR) : {}", panel_refno);

    let frmw_sql = format!("SELECT VALUE OWNER FROM SBFR WHERE REFNO = {} LIMIT 1", sbfr_pe);
    let frmw_ids: Vec<RecordId> = SUL_DB.query_take(&frmw_sql, 0).await.unwrap_or_default();
    let frmw_pe = frmw_ids
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
    let room_nums: Vec<String> = SUL_DB.query_take(&room_num_sql, 0).await.unwrap_or_default();
    let room_num = room_nums.first().cloned().unwrap_or_default();
    anyhow::ensure!(
        !room_num.is_empty(),
        "未能从 FRMW.NAME 解析房间号: FRMW={}",
        frmw_pe
    );

    println!("🎯 panel: {}", panel_refno);
    println!("   - SBFR(pe): {}", sbfr_pe);
    println!("   - FRMW(pe): {}", frmw_pe);
    println!("   - room_num: {}", room_num);

    // 5) 只计算“该 panel”的 room_relate（避免重建整房间导致耗时过长）
    let mesh_dir = db_option_ext.inner.get_meshes_path();
    let exclude = HashSet::<RefnoEnum>::new();
    let within = aios_database::fast_model::room_model::cal_room_refnos(
        &mesh_dir,
        panel_refno,
        &exclude,
        0.1,
    )
    .await
    .context("房间计算失败")?;

    println!("✅ 房间计算完成: panel={}, components={}", panel_refno, within.len());

    // 覆盖写入：先删旧的，再批量写入新的（与 room_model.rs 的 save_room_relate 保持一致）。
    let delete_sql = format!("DELETE room_relate WHERE `in` = {};", panel_refno.to_pe_key());
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
