//! 测试房间计算中 PANEL 17496_198106 和管件 24381_59222 的包含关系

use std::collections::HashSet;
use aios_core::{init_surreal, RefnoEnum, SUL_DB, SurrealQueryExt};
use aios_core::pdms_types::RefU64;
use aios_core::room::algorithm::*;
use parry3d::bounding_volume::BoundingVolume;
use anyhow::Result;

#[tokio::main]
async fn main() -> Result<()> {
    // 初始化数据库连接
    println!("🔌 初始化数据库连接...");
    init_surreal().await?;
    println!("✅ 数据库连接成功");

    // 测试的参考号
    let panel_refno = RefnoEnum::Ref64(RefU64::from_two_nums(17496, 198106));
    let pipe_refno = RefnoEnum::Ref64(RefU64::from_two_nums(24381, 59222));

    println!("\n🎯 测试目标:");
    println!("  - PANEL: {}", panel_refno);
    println!("  - 管件: {}", pipe_refno);

    // 1. 查询 PANEL 的基本信息
    println!("\n📋 1. 查询 PANEL 基本信息...");
    query_panel_info(&panel_refno).await?;

    // 2. 查询管件的基本信息
    println!("\n📋 2. 查询管件基本信息...");
    query_pipe_info(&pipe_refno).await?;

    // 3. 查询 PANEL 的几何信息和 AABB
    println!("\n📦 3. 查询 PANEL 几何信息...");
    let panel_aabb = query_geometry_aabb(&panel_refno).await?;
    
    // 4. 查询管件的几何信息和 AABB
    println!("\n📦 4. 查询管件几何信息...");
    let pipe_aabb = query_geometry_aabb(&pipe_refno).await?;

    // 5. 检查 AABB 包含关系
    println!("\n🔍 5. 检查 AABB 包含关系...");
    check_aabb_containment(&panel_refno, &panel_aabb, &pipe_refno, &pipe_aabb);

    // 6. 执行精确的房间计算查询
    println!("\n🏠 6. 执行房间计算查询...");
    test_room_calculation(&panel_refno, &pipe_refno).await?;

    // 7. 查询管件所在的房间
    println!("\n🏠 7. 查询管件所在的房间...");
    query_pipe_room(&pipe_refno).await?;

    Ok(())
}

/// 查询 PANEL 的基本信息
async fn query_panel_info(refno: &RefnoEnum) -> Result<()> {
    let sql = format!(
        "SELECT refno, noun, name, dbno FROM {} WHERE id = {}",
        refno.table_name(),
        refno.to_pe_key()
    );
    
    if let Ok(result) = SUL_DB.query_take::<serde_json::Value>(&sql, 0).await {
        println!("   ✅ 找到 PANEL: {:?}", result);
    } else {
        println!("   ❌ 未找到 PANEL 记录");
    }
    
    Ok(())
}

/// 查询管件的基本信息
async fn query_pipe_info(refno: &RefnoEnum) -> Result<()> {
    let sql = format!(
        "SELECT refno, noun, name, dbno FROM {} WHERE id = {}",
        refno.table_name(),
        refno.to_pe_key()
    );
    
    if let Ok(result) = SUL_DB.query_take::<serde_json::Value>(&sql, 0).await {
        println!("   ✅ 找到管件: {:?}", result);
    } else {
        println!("   ❌ 未找到管件记录");
    }
    
    Ok(())
}

/// 查询几何体的 AABB
async fn query_geometry_aabb(refno: &RefnoEnum) -> Result<Option<parry3d::bounding_volume::Aabb>> {
    // 查询 inst_relate 获取 AABB
    let sql = format!(
        "SELECT aabb, world_trans FROM inst_relate WHERE in = {}",
        refno.to_pe_key()
    );
    
    if let Ok(results) = SUL_DB.query_take::<Vec<serde_json::Value>>(&sql, 0).await {
        if let Some(first) = results.first() {
            if let Some(aabb_record) = first.get("aabb") {
                if let Some(aabb_id) = aabb_record.get("id") {
                    // 查询 AABB 详情
                    let aabb_sql = format!("SELECT * FROM {}", aabb_id);
                    if let Ok(aabb_data) = SUL_DB.query_take::<Vec<serde_json::Value>>(&aabb_sql, 0).await {
                        if let Some(first) = aabb_data.first() {
                            println!("   📐 AABB: {:?}", first);
                            
                            // 尝试解析 AABB 值
                            if let Some(min) = first.get("min") {
                                if let Some(max) = first.get("max") {
                                    // 这里简化处理，实际应该根据具体数据结构解析
                                    println!("   📏 最小点: {:?}", min);
                                    println!("   📏 最大点: {:?}", max);
                                    return Ok(Some(parse_aabb_from_json(first)));
                                }
                            }
                        }
                    }
                }
            }
        }
    }
    
    println!("   ⚠️ 未找到 AABB 数据");
    Ok(None)
}

/// 从 JSON 解析 AABB（简化版本）
fn parse_aabb_from_json(_json: &serde_json::Value) -> parry3d::bounding_volume::Aabb {
    // 这里需要根据实际数据结构实现
    // 暂时返回一个默认 AABB
    parry3d::bounding_volume::Aabb::new(
        parry3d::math::Point::new(-10.0, -10.0, -10.0),
        parry3d::math::Point::new(10.0, 10.0, 10.0),
    )
}

/// 检查 AABB 包含关系
fn check_aabb_containment(
    panel_refno: &RefnoEnum,
    panel_aabb: &Option<parry3d::bounding_volume::Aabb>,
    pipe_refno: &RefnoEnum,
    pipe_aabb: &Option<parry3d::bounding_volume::Aabb>,
) {
    match (panel_aabb, pipe_aabb) {
        (Some(p_aabb), Some(pipe_aabb)) => {
            if p_aabb.contains(&pipe_aabb.center()) {
                println!("   ✅ PANEL AABB 包含管件中心点");
            } else {
                println!("   ❌ PANEL AABB 不包含管件中心点");
                println!("      PANEL 中心: {:?}", p_aabb.center());
                println!("      管件 中心: {:?}", pipe_aabb.center());
            }
            
            // 检查是否完全包含
            let pipe_corners = [
                pipe_aabb.mins,
                parry3d::math::Point::new(pipe_aabb.maxs.x, pipe_aabb.mins.y, pipe_aabb.mins.z),
                parry3d::math::Point::new(pipe_aabb.mins.x, pipe_aabb.maxs.y, pipe_aabb.mins.z),
                parry3d::math::Point::new(pipe_aabb.mins.x, pipe_aabb.mins.y, pipe_aabb.maxs.z),
                pipe_aabb.maxs,
                parry3d::math::Point::new(pipe_aabb.mins.x, pipe_aabb.maxs.y, pipe_aabb.maxs.z),
                parry3d::math::Point::new(pipe_aabb.maxs.x, pipe_aabb.mins.y, pipe_aabb.maxs.z),
                parry3d::math::Point::new(pipe_aabb.maxs.x, pipe_aabb.maxs.y, pipe_aabb.mins.z),
            ];
            
            let all_contained = pipe_corners.iter().all(|corner| p_aabb.contains(corner));
            if all_contained {
                println!("   ✅ PANEL AABB 完全包含管件 AABB");
            } else {
                println!("   ⚠️ PANEL AABB 部分包含管件 AABB（或完全不包含）");
            }
        }
        _ => {
            println!("   ❌ 无法检查包含关系（缺少 AABB 数据）");
        }
    }
}

/// 执行房间计算查询
async fn test_room_calculation(panel_refno: &RefnoEnum, pipe_refno: &RefnoEnum) -> Result<()> {
    // 查询 PANEL 所在的房间
    let panel_sql = format!(
        "SELECT room_number FROM panel_room WHERE panel_refno = {}",
        panel_refno.to_pe_key()
    );
    
    if let Ok(panel_rooms) = SUL_DB.query_take::<Vec<String>>(&panel_sql, 0).await {
        println!("   🏠 PANEL 所在房间: {:?}", panel_rooms);
        
        // 查询这些房间包含的所有构件
        for room_num in &panel_rooms {
            let room_components_sql = format!(
                "SELECT component_refno FROM room_component WHERE room_number = {}",
                room_num
            );
            
            if let Ok(components) = SUL_DB.query_take::<Vec<RefnoEnum>>(&room_components_sql, 0).await {
                println!("   📦 房间 {} 包含 {} 个构件", room_num, components.len());
                
                // 检查是否包含目标管件
                if components.contains(pipe_refno) {
                    println!("   ✅ 管件 {} 在房间 {} 中", pipe_refno, room_num);
                } else {
                    println!("   ❌ 管件 {} 不在房间 {} 中", pipe_refno, room_num);
                }
            }
        }
    } else {
        println!("   ⚠️ 未找到 PANEL 的房间信息");
    }
    
    Ok(())
}

/// 查询管件所在的房间
async fn query_pipe_room(pipe_refno: &RefnoEnum) -> Result<()> {
    // 方法1: 通过 room_component 表查询
    let room_sql = format!(
        "SELECT room_number FROM room_component WHERE component_refno = {}",
        pipe_refno.to_pe_key()
    );
    
    if let Ok(rooms) = SUL_DB.query_take::<Vec<String>>(&room_sql, 0).await {
        println!("   🏠 管件所在房间（通过 room_component）: {:?}", rooms);
    } else {
        println!("   ⚠️ 通过 room_component 未找到房间信息");
    }
    
    // 方法2: 通过空间查询（如果支持）
    println!("   🔍 尝试空间查询...");
    
    // 获取管件的世界坐标
    let trans_sql = format!(
        "SELECT world_trans FROM inst_relate WHERE in = {}",
        pipe_refno.to_pe_key()
    );
    
    if let Ok(trans_results) = SUL_DB.query_take::<Vec<serde_json::Value>>(&trans_sql, 0).await {
        if let Some(first) = trans_results.first() {
            if let Some(world_trans) = first.get("world_trans") {
                println!("   📍 管件世界变换: {:?}", world_trans);
                
                // 这里可以调用房间计算的空间查询函数
                // 但需要根据实际的房间计算 API 来实现
                println!("   📝 TODO: 根据世界坐标查询所在房间");
            }
        }
    }
    
    Ok(())
}
