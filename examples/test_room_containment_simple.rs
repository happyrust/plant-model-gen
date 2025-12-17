//! 测试房间计算中 PANEL 17496_198106 和管件 24381_59222 的包含关系

use aios_core::{init_surreal, RefnoEnum, SUL_DB, SurrealQueryExt};
use aios_core::pdms_types::RefU64;
use anyhow::Result;

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
    println!("  - PANEL: {}", panel_refno);
    println!("  - 管件: {}", pipe_refno);

    // 1. 查询 PANEL 的基本信息
    println!("\n📋 1. 查询 PANEL 基本信息...");
    query_entity_info(&panel_refno, "PANEL").await?;

    // 2. 查询管件的基本信息
    println!("\n📋 2. 查询管件基本信息...");
    query_entity_info(&pipe_refno, "管件").await?;

    // 3. 查询 PANEL 的几何信息
    println!("\n📦 3. 查询 PANEL 几何信息...");
    query_geometry_info(&panel_refno).await?;
    
    // 4. 查询管件的几何信息
    println!("\n📦 4. 查询管件几何信息...");
    query_geometry_info(&pipe_refno).await?;

    // 5. 查询房间关系表
    println!("\n🏠 5. 查询房间关系表...");
    query_room_relations(&panel_refno, &pipe_refno).await?;

    Ok(())
}

/// 查询实体基本信息
async fn query_entity_info(refno: &RefnoEnum, entity_type: &str) -> Result<()> {
    let table_name = match refno {
        RefnoEnum::Refno(_) => "pe",
        RefnoEnum::SesRef(_) => "sesno",
    };
    
    // 使用 table:id 方式查询
    let sql = format!(
        "SELECT refno, noun, name, dbno FROM {}:{}",
        table_name,
        refno.to_string()
    );
    
    if let Ok(result) = SUL_DB.query_take::<Vec<serde_json::Value>>(&sql, 0).await {
        if let Some(first) = result.first() {
            println!("   ✅ 找到 {}: {:?}", entity_type, first);
        } else {
            println!("   ❌ 未找到 {} 记录", entity_type);
        }
    } else {
        println!("   ❌ 查询 {} 失败", entity_type);
    }
    
    Ok(())
}

/// 查询几何信息
async fn query_geometry_info(refno: &RefnoEnum) -> Result<()> {
    // 查询 inst_relate - 使用 table:id 方式
    let sql = format!(
        "SELECT * FROM inst_relate:{}",
        refno.to_string()
    );
    
    if let Ok(results) = SUL_DB.query_take::<Vec<serde_json::Value>>(&sql, 0).await {
        println!("   📐 inst_relate 记录数: {}", results.len());
        if let Some(first) = results.first() {
            // 打印关键信息
            if let Some(aabb) = first.get("aabb") {
                println!("   📦 AABB: {:?}", aabb);
            }
            if let Some(world_trans) = first.get("world_trans") {
                println!("   🌍 世界变换: {:?}", world_trans);
            }
            if let Some(mesh_id) = first.get("mesh_id") {
                println!("   🔷 Mesh ID: {:?}", mesh_id);
            }
        }
    } else {
        println!("   ❌ 未找到 inst_relate 记录");
    }
    
    // 查询相关的 geo_relate - 使用 table:id 方式
    let geo_sql = format!(
        "SELECT * FROM geo_relate:{}",
        refno.to_string()
    );
    
    if let Ok(geo_results) = SUL_DB.query_take::<Vec<serde_json::Value>>(&geo_sql, 0).await {
        println!("   🔗 geo_relate 记录数: {}", geo_results.len());
    }
    
    Ok(())
}

/// 查询房间关系
async fn query_room_relations(panel_refno: &RefnoEnum, pipe_refno: &RefnoEnum) -> Result<()> {
    // 1. 查询 panel_room 表 - 使用 table:id 方式
    let panel_room_sql = format!(
        "SELECT * FROM panel_room:{}",
        panel_refno.to_string()
    );
    
    println!("\n   📋 Panel-Room 关系:");
    if let Ok(results) = SUL_DB.query_take::<Vec<serde_json::Value>>(&panel_room_sql, 0).await {
        if !results.is_empty() {
            for result in &results {
                println!("     {:?}", result);
            }
        } else {
            println!("     ⚠️ PANEL 未关联到房间");
        }
    }
    
    // 2. 查询 room_component 表（管件在哪个房间） - 使用 table:id 方式
    let pipe_room_sql = format!(
        "SELECT * FROM room_component:{}",
        pipe_refno.to_string()
    );
    
    println!("\n   📋 Pipe-Room 关系:");
    if let Ok(results) = SUL_DB.query_take::<Vec<serde_json::Value>>(&pipe_room_sql, 0).await {
        if !results.is_empty() {
            for result in &results {
                println!("     {:?}", result);
            }
        } else {
            println!("     ⚠️ 管件未关联到房间");
        }
    }
    
    // 3. 查询 room_component 表（PANEL 在哪个房间） - 使用 table:id 方式
    let panel_component_sql = format!(
        "SELECT * FROM room_component:{}",
        panel_refno.to_string()
    );
    
    println!("\n   📋 Panel 作为房间组件:");
    if let Ok(results) = SUL_DB.query_take::<Vec<serde_json::Value>>(&panel_component_sql, 0).await {
        if !results.is_empty() {
            for result in &results {
                println!("     {:?}", result);
            }
        } else {
            println!("     ⚠️ PANEL 未作为房间组件");
        }
    }
    
    // 4. 检查两者是否在同一个房间
    println!("\n   🔍 包含关系分析:");
    let panel_rooms = find_rooms_for_entity(panel_refno).await?;
    let pipe_rooms = find_rooms_for_entity(pipe_refno).await?;
    
    println!("     PANEL 所在房间: {:?}", panel_rooms);
    println!("     管件所在房间: {:?}", pipe_rooms);
    
    // 检查是否有共同的房间
    let common_rooms: Vec<String> = panel_rooms
        .intersection(&pipe_rooms)
        .cloned()
        .collect();
    
    if !common_rooms.is_empty() {
        println!("     ✅ 两者在同一个房间: {:?}", common_rooms);
    } else {
        println!("     ❌ 两者不在同一个房间");
        
        // 如果 PANEL 是房间面板，检查管件是否在其中
        if !panel_rooms.is_empty() {
            println!("     🔍 PANEL 是房间面板，检查管件是否在房间内...");
            for room in &panel_rooms {
                check_pipe_in_room(room, pipe_refno).await?;
            }
        }
    }
    
    Ok(())
}

/// 查找实体所在的房间
async fn find_rooms_for_entity(refno: &RefnoEnum) -> Result<std::collections::HashSet<String>> {
    let mut rooms = std::collections::HashSet::new();
    
    // 通过 room_component 查询 - 使用 table:id 方式
    let sql = format!(
        "SELECT room_number FROM room_component:{}",
        refno.to_string()
    );
    
    if let Ok(results) = SUL_DB.query_take::<Vec<serde_json::Value>>(&sql, 0).await {
        for result in results {
            if let Some(room_num) = result.get("room_number") {
                if let Some(room_str) = room_num.as_str() {
                    rooms.insert(room_str.to_string());
                }
            }
        }
    }
    
    Ok(rooms)
}

/// 检查管件是否在房间内
async fn check_pipe_in_room(room_number: &str, pipe_refno: &RefnoEnum) -> Result<()> {
    // 查询房间的所有组件
    let sql = format!(
        "SELECT component_refno FROM room_component WHERE room_number = '{}'",
        room_number
    );
    
    if let Ok(results) = SUL_DB.query_take::<Vec<serde_json::Value>>(&sql, 0).await {
        let mut found = false;
        for result in results {
            if let Some(comp_refno) = result.get("component_refno") {
                if let Some(refno_str) = comp_refno.as_str() {
                    if refno_str.contains(&format!("{}", pipe_refno)) {
                        found = true;
                        break;
                    }
                }
            }
        }
        
        if found {
            println!("       ✅ 管件 {} 在房间 {} 内", pipe_refno, room_number);
        } else {
            println!("       ❌ 管件 {} 不在房间 {} 内", pipe_refno, room_number);
        }
    }
    
    Ok(())
}
