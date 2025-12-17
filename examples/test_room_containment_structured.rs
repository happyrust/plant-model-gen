//! 使用 SurrealValue 方式查询房间计算中 PANEL 17496_198106 和管件 24381_59222 的包含关系

use aios_core::{init_surreal, RefnoEnum, SUL_DB, SurrealQueryExt};
use aios_core::pdms_types::RefU64;
use surrealdb::opt::RecordId;
use surrealdb::types::SurrealValue;
use serde::{Deserialize, Serialize};
use anyhow::Result;

#[derive(Debug, Serialize, Deserialize, SurrealValue)]
struct PeInfo {
    refno: u64,
    noun: String,
    name: Option<String>,
    dbno: Option<u32>,
}

#[derive(Debug, Serialize, Deserialize, SurrealValue)]
struct InstRelateInfo {
    id: RecordId,
    #[serde(rename = "in")]
    in_ref: Option<String>,
    out: Option<String>,
    aabb: Option<AabbInfo>,
    world_trans: Option<serde_json::Value>,
    mesh_id: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, SurrealValue)]
struct AabbInfo {
    id: String,
    min: [f64; 3],
    max: [f64; 3],
}

#[derive(Debug, Serialize, Deserialize, SurrealValue)]
struct PanelRoomInfo {
    id: RecordId,
    panel_refno: String,
    room_number: String,
}

#[derive(Debug, Serialize, Deserialize, SurrealValue)]
struct RoomComponentInfo {
    id: RecordId,
    room_number: String,
    component_refno: String,
}

#[tokio::main]
async fn main() -> Result<()> {
    // 初始化数据库连接
    println!("🔌 初始化数据库连接...");
    init_surreal().await?;
    println!("✅ 数据库连接成功");

    // 测试的参考号
    let panel_refno = RefnoEnum::Refno(RefU64::from_two_nums(17496, 198106));
    let pipe_refno = RefnoEnum::Refno(RefU64::from_two_nums(24381, 59222));

    println!("\n🎯 测试目标:");
    println!("  - PANEL: {} (id: {})", panel_refno, panel_refno.to_string());
    println!("  - 管件: {} (id: {})", pipe_refno, pipe_refno.to_string());

    // 1. 查询基本信息
    println!("\n📋 1. 查询基本信息...");
    let panel_info = query_pe_info(&panel_refno).await?;
    let pipe_info = query_pe_info(&pipe_refno).await?;

    // 2. 查询几何信息
    println!("\n📦 2. 查询几何信息...");
    let _panel_inst = query_inst_relate(&panel_refno).await?;
    let _pipe_inst = query_inst_relate(&pipe_refno).await?;

    // 3. 查询房间关系
    println!("\n🏠 3. 查询房间关系...");
    let panel_rooms = query_panel_rooms(&panel_refno).await?;
    let pipe_rooms = query_room_components(&pipe_refno).await?;
    let panel_in_rooms = query_room_components(&panel_refno).await?;

    // 4. 分析包含关系
    println!("\n🔍 4. 分析包含关系...");
    analyze_containment(&panel_info, &pipe_info, &panel_rooms, &pipe_rooms, &panel_in_rooms).await?;

    Ok(())
}

/// 查询 PE 表信息
async fn query_pe_info(refno: &RefnoEnum) -> Result<Option<PeInfo>> {
    let sql = format!("SELECT refno, noun, name, dbno FROM pe:{}", refno.to_string());
    
    match SUL_DB.query_take::<PeInfo>(&sql, 0).await {
        Ok(info) => {
            println!("   ✅ 找到 PE 记录: {:?}", info);
            Ok(Some(info))
        }
        Err(e) => {
            println!("   ❌ 未找到 PE 记录: {}", e);
            Ok(None)
        }
    }
}

/// 查询 inst_relate 信息
async fn query_inst_relate(refno: &RefnoEnum) -> Result<Option<InstRelateInfo>> {
    // 尝试查询 in 字段
    let sql_in = format!("SELECT * FROM inst_relate WHERE in = pe:{}", refno.to_string());
    
    match SUL_DB.query_take::<Vec<InstRelateInfo>>(&sql_in, 0).await {
        Ok(results) => {
            if !results.is_empty() {
                println!("   ✅ 找到 inst_relate 记录 (in): {} 条", results.len());
                if let Some(first) = results.first() {
                    println!("     AABB: {:?}", first.aabb);
                    println!("     Mesh ID: {:?}", first.mesh_id);
                }
                return Ok(Some(results[0].clone()));
            }
        }
        Err(e) => {
            println!("   ⚠️ 查询 inst_relate (in) 失败: {}", e);
        }
    }
    
    // 尝试查询 out 字段
    let sql_out = format!("SELECT * FROM inst_relate WHERE out = pe:{}", refno.to_string());
    
    match SUL_DB.query_take::<Vec<InstRelateInfo>>(&sql_out, 0).await {
        Ok(results) => {
            if !results.is_empty() {
                println!("   ✅ 找到 inst_relate 记录 (out): {} 条", results.len());
                if let Some(first) = results.first() {
                    println!("     AABB: {:?}", first.aabb);
                    println!("     Mesh ID: {:?}", first.mesh_id);
                }
                return Ok(Some(results[0].clone()));
            }
        }
        Err(e) => {
            println!("   ⚠️ 查询 inst_relate (out) 失败: {}", e);
        }
    }
    
    println!("   ❌ 未找到 inst_relate 记录");
    Ok(None)
}

/// 查询 panel_room 关系
async fn query_panel_rooms(refno: &RefnoEnum) -> Result<Vec<PanelRoomInfo>> {
    let sql = format!("SELECT * FROM panel_room:{}", refno.to_string());
    
    match SUL_DB.query_take::<Vec<PanelRoomInfo>>(&sql, 0).await {
        Ok(results) => {
            println!("   ✅ 找到 panel_room 记录: {} 条", results.len());
            for room in &results {
                println!("     房间号: {}", room.room_number);
            }
            Ok(results)
        }
        Err(e) => {
            println!("   ❌ 查询 panel_room 失败: {}", e);
            Ok(vec![])
        }
    }
}

/// 查询 room_component 关系
async fn query_room_components(refno: &RefnoEnum) -> Result<Vec<RoomComponentInfo>> {
    let sql = format!("SELECT * FROM room_component:{}", refno.to_string());
    
    match SUL_DB.query_take::<Vec<RoomComponentInfo>>(&sql, 0).await {
        Ok(results) => {
            println!("   ✅ 找到 room_component 记录: {} 条", results.len());
            for comp in &results {
                println!("     房间号: {}", comp.room_number);
            }
            Ok(results)
        }
        Err(e) => {
            println!("   ❌ 查询 room_component 失败: {}", e);
            Ok(vec![])
        }
    }
}

/// 分析包含关系
async fn analyze_containment(
    panel_info: &Option<PeInfo>,
    pipe_info: &Option<PeInfo>,
    panel_rooms: &Vec<PanelRoomInfo>,
    pipe_rooms: &Vec<RoomComponentInfo>,
    panel_in_rooms: &Vec<RoomComponentInfo>,
) -> Result<()> {
    println!("\n📊 包含关系分析结果:");
    
    // 基本信息
    if let Some(panel) = panel_info {
        println!("   PANEL: {} (noun: {}, name: {:?})", panel.refno, panel.noun, panel.name);
    }
    if let Some(pipe) = pipe_info {
        println!("   管件: {} (noun: {}, name: {:?})", pipe.refno, pipe.noun, pipe.name);
    }
    
    // 房间关系
    println!("\n   房间关系:");
    println!("     PANEL 作为房间面板: {} 个房间", panel_rooms.len());
    for room in panel_rooms {
        println!("       - 房间 {}", room.room_number);
    }
    
    println!("     管件所在房间: {} 个房间", pipe_rooms.len());
    for room in pipe_rooms {
        println!("       - 房间 {}", room.room_number);
    }
    
    println!("     PANEL 作为房间组件: {} 个房间", panel_in_rooms.len());
    for room in panel_in_rooms {
        println!("       - 房间 {}", room.room_number);
    }
    
    // 判断包含关系
    if !panel_rooms.is_empty() {
        let panel_room_numbers: std::collections::HashSet<String> = 
            panel_rooms.iter().map(|r| r.room_number.clone()).collect();
        let pipe_room_numbers: std::collections::HashSet<String> = 
            pipe_rooms.iter().map(|r| r.room_number.clone()).collect();
        
        let common_rooms: Vec<String> = panel_room_numbers
            .intersection(&pipe_room_numbers)
            .cloned()
            .collect();
        
        if !common_rooms.is_empty() {
            println!("\n   ✅ 结论: PANEL 和管件在同一个房间内");
            for room in common_rooms {
                println!("     共同房间: {}", room);
            }
        } else {
            println!("\n   ❌ 结论: PANEL 和管件不在同一个房间");
            
            // 如果 PANEL 是房间面板，检查管件是否在其中
            if !panel_rooms.is_empty() {
                println!("   🔍 进一步检查: PANEL 是房间面板，需要检查空间包含关系");
                println!("   📝 TODO: 需要使用空间查询判断管件是否在 PANEL 构成的空间内");
            }
        }
    } else {
        println!("\n   ❌ 结论: 无法确定包含关系（PANEL 不是房间面板）");
    }
    
    Ok(())
}
