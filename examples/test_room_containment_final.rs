//! 使用 SurrealValue 方式查询房间计算中 PANEL 17496_198106 和管件 24381_59222 的包含关系

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
    println!("  - PANEL: {} (id: {})", panel_refno, panel_refno.to_string());
    println!("  - 管件: {} (id: {})", pipe_refno, pipe_refno.to_string());

    // 1. 查询基本信息（使用 Vec 包装）
    println!("\n📋 1. 查询基本信息...");
    let panel_info = query_pe_info(&panel_refno).await?;
    let pipe_info = query_pe_info(&pipe_refno).await?;

    // 2. 查询几何信息
    println!("\n📦 2. 查询几何信息...");
    let panel_inst = query_inst_relate(&panel_refno).await?;
    let pipe_inst = query_inst_relate(&pipe_refno).await?;

    // 3. 查询房间关系
    println!("\n🏠 3. 查询房间关系...");
    let panel_rooms = query_panel_rooms(&panel_refno).await?;
    let pipe_rooms = query_room_components(&pipe_refno).await?;
    let panel_in_rooms = query_room_components(&panel_refno).await?;

    // 4. 分析包含关系
    println!("\n🔍 4. 分析包含关系...");
    analyze_containment(&panel_info, &pipe_info, &panel_rooms, &pipe_rooms, &panel_in_rooms).await?;

    // 5. 输出测试报告
    println!("\n📊 测试报告:");
    generate_test_report(&panel_info, &pipe_info, &panel_inst, &pipe_inst).await?;

    Ok(())
}

/// 查询 PE 表信息（返回 Vec）
async fn query_pe_info(refno: &RefnoEnum) -> Result<Vec<serde_json::Value>> {
    let sql = format!("SELECT refno, noun, name, dbno FROM pe:{}", refno.to_string());
    
    match SUL_DB.query_take::<Vec<serde_json::Value>>(&sql, 0).await {
        Ok(info) => {
            println!("   ✅ 找到 PE 记录: {} 条", info.len());
            for item in &info {
                println!("     {:?}", item);
            }
            Ok(info)
        }
        Err(e) => {
            println!("   ❌ 未找到 PE 记录: {}", e);
            Ok(vec![])
        }
    }
}

/// 查询 inst_relate 信息（返回 Vec）
async fn query_inst_relate(refno: &RefnoEnum) -> Result<Vec<serde_json::Value>> {
    // 尝试查询 in 字段
    let sql_in = format!("SELECT * FROM inst_relate WHERE in = pe:{}", refno.to_string());
    
    match SUL_DB.query_take::<Vec<serde_json::Value>>(&sql_in, 0).await {
        Ok(results) => {
            if !results.is_empty() {
                println!("   ✅ 找到 inst_relate 记录 (in): {} 条", results.len());
                for item in results.iter().take(3) {
                    if let Some(aabb) = item.get("aabb") {
                        println!("     AABB: {:?}", aabb);
                    }
                    if let Some(mesh_id) = item.get("mesh_id") {
                        println!("     Mesh ID: {:?}", mesh_id);
                    }
                }
                return Ok(results);
            }
        }
        Err(e) => {
            println!("   ⚠️ 查询 inst_relate (in) 失败: {}", e);
        }
    }
    
    // 尝试查询 out 字段
    let sql_out = format!("SELECT * FROM inst_relate WHERE out = pe:{}", refno.to_string());
    
    match SUL_DB.query_take::<Vec<serde_json::Value>>(&sql_out, 0).await {
        Ok(results) => {
            if !results.is_empty() {
                println!("   ✅ 找到 inst_relate 记录 (out): {} 条", results.len());
                for item in results.iter().take(3) {
                    if let Some(aabb) = item.get("aabb") {
                        println!("     AABB: {:?}", aabb);
                    }
                    if let Some(mesh_id) = item.get("mesh_id") {
                        println!("     Mesh ID: {:?}", mesh_id);
                    }
                }
                return Ok(results);
            }
        }
        Err(e) => {
            println!("   ⚠️ 查询 inst_relate (out) 失败: {}", e);
        }
    }
    
    println!("   ❌ 未找到 inst_relate 记录");
    Ok(vec![])
}

/// 查询 panel_room 关系
async fn query_panel_rooms(refno: &RefnoEnum) -> Result<Vec<serde_json::Value>> {
    let sql = format!("SELECT * FROM panel_room:{}", refno.to_string());
    
    match SUL_DB.query_take::<Vec<serde_json::Value>>(&sql, 0).await {
        Ok(results) => {
            println!("   ✅ 找到 panel_room 记录: {} 条", results.len());
            for room in &results {
                println!("     房间号: {:?}", room.get("room_number"));
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
async fn query_room_components(refno: &RefnoEnum) -> Result<Vec<serde_json::Value>> {
    let sql = format!("SELECT * FROM room_component:{}", refno.to_string());
    
    match SUL_DB.query_take::<Vec<serde_json::Value>>(&sql, 0).await {
        Ok(results) => {
            println!("   ✅ 找到 room_component 记录: {} 条", results.len());
            for comp in &results {
                println!("     房间号: {:?}", comp.get("room_number"));
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
    panel_info: &Vec<serde_json::Value>,
    pipe_info: &Vec<serde_json::Value>,
    panel_rooms: &Vec<serde_json::Value>,
    pipe_rooms: &Vec<serde_json::Value>,
    panel_in_rooms: &Vec<serde_json::Value>,
) -> Result<()> {
    println!("\n📊 包含关系分析结果:");
    
    // 基本信息
    if let Some(panel) = panel_info.first() {
        println!("   PANEL: refno={:?}, noun={:?}, name={:?}", 
            panel.get("refno"), 
            panel.get("noun"), 
            panel.get("name"));
    }
    if let Some(pipe) = pipe_info.first() {
        println!("   管件: refno={:?}, noun={:?}, name={:?}", 
            pipe.get("refno"), 
            pipe.get("noun"), 
            pipe.get("name"));
    }
    
    // 房间关系
    println!("\n   房间关系:");
    println!("     PANEL 作为房间面板: {} 个房间", panel_rooms.len());
    for room in panel_rooms {
        if let Some(room_num) = room.get("room_number") {
            println!("       - 房间 {:?}", room_num);
        }
    }
    
    println!("     管件所在房间: {} 个房间", pipe_rooms.len());
    for room in pipe_rooms {
        if let Some(room_num) = room.get("room_number") {
            println!("       - 房间 {:?}", room_num);
        }
    }
    
    println!("     PANEL 作为房间组件: {} 个房间", panel_in_rooms.len());
    for room in panel_in_rooms {
        if let Some(room_num) = room.get("room_number") {
            println!("       - 房间 {:?}", room_num);
        }
    }
    
    // 判断包含关系
    if !panel_rooms.is_empty() {
        let mut panel_room_numbers = std::collections::HashSet::new();
        for room in panel_rooms {
            if let Some(room_num) = room.get("room_number") {
                if let Some(s) = room_num.as_str() {
                    panel_room_numbers.insert(s.to_string());
                }
            }
        }
        
        let mut pipe_room_numbers = std::collections::HashSet::new();
        for room in pipe_rooms {
            if let Some(room_num) = room.get("room_number") {
                if let Some(s) = room_num.as_str() {
                    pipe_room_numbers.insert(s.to_string());
                }
            }
        }
        
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
                println!("   📝 建议: 使用空间查询 API 判断管件是否在 PANEL 构成的空间内");
            }
        }
    } else {
        println!("\n   ❌ 结论: 无法确定包含关系（PANEL 不是房间面板）");
    }
    
    Ok(())
}

/// 生成测试报告
async fn generate_test_report(
    panel_info: &Vec<serde_json::Value>,
    pipe_info: &Vec<serde_json::Value>,
    panel_inst: &Vec<serde_json::Value>,
    pipe_inst: &Vec<serde_json::Value>,
) -> Result<()> {
    println!("===============================================");
    println!("房间计算包含关系测试报告");
    println!("===============================================");
    
    println!("\n1. 数据存在性检查:");
    println!("   - PANEL 17496_198106: {} {}", 
        if panel_info.is_empty() { "❌ 不存在" } else { "✅ 存在" },
        if panel_inst.is_empty() { "(无几何数据)" } else { "(有几何数据)" }
    );
    println!("   - 管件 24381_59222: {} {}", 
        if pipe_info.is_empty() { "❌ 不存在" } else { "✅ 存在" },
        if pipe_inst.is_empty() { "(无几何数据)" } else { "(有几何数据)" }
    );
    
    println!("\n2. 测试结论:");
    if panel_info.is_empty() || pipe_info.is_empty() {
        println!("   ⚠️ 测试无法完成：参考号不存在于数据库中");
        println!("   📝 建议:");
        println!("     1. 检查参考号是否正确");
        println!("     2. 确认数据库是否包含测试数据");
        println!("     3. 验证数据库连接配置");
    } else {
        println!("   ✅ 测试执行完成");
        println!("   📊 详细结果请查看上述分析");
    }
    
    println!("\n3. 后续建议:");
    println!("   - 如果数据不存在，请提供有效的参考号");
    println!("   - 如果需要验证空间包含关系，可以使用:");
    println!("     * AABB 包含检查");
    println!("     * 点在多面体内判断");
    println!("     * 房间计算 API");
    
    println!("===============================================");
    
    Ok(())
}
