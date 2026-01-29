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
use std::collections::HashSet;
use std::env;

fn env_or(key: &str, default: &str) -> String {
    env::var(key).unwrap_or_else(|_| default.to_string())
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

    // 3) 初始化 SurrealDB
    init_surreal().await.context("初始化 SurrealDB 失败")?;

    // 4) panel -> SBFR(OWNER) -> FRMW(OWNER) -> room_num(FRMW.NAME 最后一段)
    let panel_u64 = panel_refno.refno().0;

    let sbfr_sql = format!(
        "SELECT VALUE OWNER FROM PANE WHERE REFNO = {} LIMIT 1",
        panel_u64
    );
    let sbfr_nums: Vec<i64> = SUL_DB.query_take(&sbfr_sql, 0).await.unwrap_or_default();
    let sbfr_num = sbfr_nums.first().copied().unwrap_or(0);
    anyhow::ensure!(sbfr_num > 0, "未找到 PANE.OWNER(SBFR) : {}", panel_refno);

    let frmw_sql = format!(
        "SELECT VALUE OWNER FROM SBFR WHERE REFNO = {} LIMIT 1",
        sbfr_num
    );
    let frmw_nums: Vec<i64> = SUL_DB.query_take(&frmw_sql, 0).await.unwrap_or_default();
    let frmw_num = frmw_nums.first().copied().unwrap_or(0);
    anyhow::ensure!(frmw_num > 0, "未找到 SBFR.OWNER(FRMW) : SBFR={}", sbfr_num);

    let room_num_sql = format!(
        "SELECT VALUE array::last(string::split(NAME, '-')) FROM FRMW WHERE REFNO = {} LIMIT 1",
        frmw_num
    );
    let room_nums: Vec<String> = SUL_DB.query_take(&room_num_sql, 0).await.unwrap_or_default();
    let room_num = room_nums.first().cloned().unwrap_or_default();
    anyhow::ensure!(
        !room_num.is_empty(),
        "未能从 FRMW.NAME 解析房间号: FRMW={}",
        frmw_num
    );

    println!("🎯 panel: {}", panel_refno);
    println!("   - SBFR: {}", sbfr_num);
    println!("   - FRMW: {}", frmw_num);
    println!("   - room_num: {}", room_num);

    // 5) 仅重建该房间的 room_relate
    let stats = aios_database::fast_model::rebuild_room_relations_for_rooms(
        Some(vec![room_num.clone()]),
        &db_option_ext.inner,
    )
    .await
    .context("重建房间关系失败")?;

    println!(
        "✅ 重建完成: rooms={}, panels={}, components={}, build_time_ms={}",
        stats.total_rooms, stats.total_panels, stats.total_components, stats.build_time_ms
    );

    // 6) 查询该 panel 的 room_relate 结果（panel -> component，附 room_num）
    let relate_sql = format!(
        "SELECT VALUE [out, room_num] FROM room_relate WHERE `in` = {}",
        panel_refno.to_pe_key()
    );
    let rows: Vec<(RecordId, String)> = SUL_DB.query_take(&relate_sql, 0).await.unwrap_or_default();
    let mut within = HashSet::<RefnoEnum>::new();
    let mut got_room_num = String::new();
    for (out_id, rn) in rows {
        within.insert(RefnoEnum::from(out_id));
        if got_room_num.is_empty() && !rn.is_empty() {
            got_room_num = rn;
        }
    }

    println!(
        "📌 room_relate(panel -> components): count={}, room_num={}",
        within.len(),
        got_room_num
    );
    for (idx, r) in within.iter().take(30).enumerate() {
        println!("   - [{}] {}", idx + 1, r);
    }
    if within.len() > 30 {
        println!("   ... 还有 {} 条未打印", within.len() - 30);
    }

    Ok(())
}

