/// 测试特定 refno 的房间生成和计算
///
/// 测试目标：
/// - FRMW 17496/198104 (房间)
/// - 管道 24381/59217 (与房间相交的管道)

#[cfg(test)]
#[cfg(all(not(target_arch = "wasm32"), feature = "sqlite-index"))]
mod tests {
    use aios_core::options::DbOption;
    use aios_core::rs_surreal::inst::{query_insts, GeomInstQuery};
    use aios_core::{RefnoEnum, SUL_DB, SurrealQueryExt, get_db_option, init_surreal};
    use anyhow::{Context, Result};
    use parry3d::bounding_volume::{Aabb, BoundingVolume};
    use std::collections::HashSet;
    use std::str::FromStr;
    use std::time::Instant;

    /// 测试 FRMW 和管道的几何信息查询
    #[tokio::test]
    #[ignore = "需要真实数据库连接，手动运行"]
    async fn test_query_frmw_and_pipe_geometry() -> Result<()> {
        println!("\n🏗️  测试 FRMW 和管道几何信息查询");
        println!("{}", "=".repeat(80));

        init_surreal().await.context("初始化 SurrealDB 失败")?;

        // 目标 refno
        let frmw_refno = RefnoEnum::from_str("17496/198104").expect("无效的 FRMW refno");
        let pipe_refno = RefnoEnum::from_str("24381/59217").expect("无效的管道 refno");

        println!("📍 FRMW refno: {}", frmw_refno);
        println!("📍 管道 refno: {}", pipe_refno);

        // 查询 FRMW 信息
        println!("\n🔍 查询 FRMW 信息...");
        let frmw_sql = format!(
            "SELECT REFNO, OWNER, noun, NAME FROM FRMW WHERE REFNO = {}",
            frmw_refno.refno().0
        );
        let frmw_info: Vec<serde_json::Value> = SUL_DB.query_take(&frmw_sql, 0).await?;
        if let Some(info) = frmw_info.first() {
            println!("   FRMW 信息: {}", serde_json::to_string_pretty(info)?);
        }

        // 查询管道信息
        println!("\n🔍 查询管道信息...");
        let pipe_sql = format!(
            "SELECT REFNO, OWNER, noun, NAME FROM pe WHERE REFNO = {}",
            pipe_refno.refno().0
        );
        let pipe_info: Vec<serde_json::Value> = SUL_DB.query_take(&pipe_sql, 0).await?;
        if let Some(info) = pipe_info.first() {
            println!("   管道信息: {}", serde_json::to_string_pretty(info)?);
        }

        // 查询 FRMW 的子节点（SBFR -> PANE）
        println!("\n🔍 查询 FRMW 的子节点（房间面板）...");
        let panels_sql = format!(
            r#"
            SELECT value array::flatten((
                SELECT value (SELECT value REFNO FROM PANE WHERE OWNER = $parent.REFNO) 
                FROM SBFR WHERE OWNER = $parent.REFNO
            )) FROM pe WHERE id = {}
            "#,
            frmw_refno.to_pe_key()
        );
        let panels: Vec<Vec<RefnoEnum>> = SUL_DB.query_take(&panels_sql, 0).await.unwrap_or_default();
        if let Some(panel_list) = panels.first() {
            println!("   找到 {} 个面板", panel_list.len());
            for (i, panel) in panel_list.iter().take(5).enumerate() {
                println!("   - 面板 {}: {}", i + 1, panel);
            }
            if panel_list.len() > 5 {
                println!("   - ... 还有 {} 个面板", panel_list.len() - 5);
            }
        }

        Ok(())
    }

    /// 测试 FRMW 和管道的 AABB 相交
    #[tokio::test]
    #[ignore = "需要真实数据库连接，手动运行"]
    async fn test_frmw_pipe_aabb_intersection() -> Result<()> {
        println!("\n🏗️  测试 FRMW 和管道 AABB 相交");
        println!("{}", "=".repeat(80));

        init_surreal().await.context("初始化 SurrealDB 失败")?;

        // 目标 refno
        let frmw_refno = RefnoEnum::from_str("17496/198104").expect("无效的 FRMW refno");
        let pipe_refno = RefnoEnum::from_str("24381/59217").expect("无效的管道 refno");

        // 查询 FRMW 的 inst_relate（获取 AABB）
        println!("\n🔍 查询 FRMW inst_relate...");
        let frmw_inst_sql = format!(
            r#"
            SELECT aabb.d as world_aabb, world_trans.d as world_trans 
            FROM inst_relate WHERE in = {}
            "#,
            frmw_refno.to_pe_key()
        );
        let frmw_insts: Vec<serde_json::Value> = SUL_DB.query_take(&frmw_inst_sql, 0).await.unwrap_or_default();
        println!("   FRMW inst_relate 数量: {}", frmw_insts.len());
        if let Some(inst) = frmw_insts.first() {
            println!("   FRMW AABB: {:?}", inst.get("world_aabb"));
        }

        // 查询管道的 inst_relate（获取 AABB）
        println!("\n🔍 查询管道 inst_relate...");
        let pipe_inst_sql = format!(
            r#"
            SELECT aabb.d as world_aabb, world_trans.d as world_trans 
            FROM inst_relate WHERE in = {}
            "#,
            pipe_refno.to_pe_key()
        );
        let pipe_insts: Vec<serde_json::Value> = SUL_DB.query_take(&pipe_inst_sql, 0).await.unwrap_or_default();
        println!("   管道 inst_relate 数量: {}", pipe_insts.len());
        if let Some(inst) = pipe_insts.first() {
            println!("   管道 AABB: {:?}", inst.get("world_aabb"));
        }

        // 使用 query_insts 查询几何实例
        println!("\n🔍 使用 query_insts 查询几何实例...");
        
        let frmw_geom_insts: Vec<GeomInstQuery> = query_insts(&[frmw_refno], true).await.unwrap_or_default();
        println!("   FRMW 几何实例数量: {}", frmw_geom_insts.len());
        
        let pipe_geom_insts: Vec<GeomInstQuery> = query_insts(&[pipe_refno], true).await.unwrap_or_default();
        println!("   管道几何实例数量: {}", pipe_geom_insts.len());

        // 检查 AABB 相交
        if !frmw_geom_insts.is_empty() && !pipe_geom_insts.is_empty() {
            let frmw_aabb: Aabb = frmw_geom_insts[0].world_aabb.into();
            let pipe_aabb: Aabb = pipe_geom_insts[0].world_aabb.into();

            println!("\n📊 AABB 信息:");
            println!("   FRMW AABB: mins={:?}, maxs={:?}", frmw_aabb.mins, frmw_aabb.maxs);
            println!("   管道 AABB: mins={:?}, maxs={:?}", pipe_aabb.mins, pipe_aabb.maxs);

            let intersects = frmw_aabb.intersects(&pipe_aabb);
            println!("\n✅ AABB 相交测试: {}", if intersects { "相交" } else { "不相交" });
        } else {
            println!("⚠️  无法获取几何实例，跳过 AABB 相交测试");
        }

        Ok(())
    }

    /// 测试单个房间的计算（使用特定的 FRMW）
    #[tokio::test]
    #[ignore = "需要真实数据库连接，手动运行"]
    async fn test_single_room_calculation() -> Result<()> {
        println!("\n🏗️  测试单个房间计算");
        println!("{}", "=".repeat(80));

        init_surreal().await.context("初始化 SurrealDB 失败")?;

        let db_option = get_db_option();
        let mesh_dir = db_option.get_meshes_path();

        // 目标 FRMW
        let frmw_refno = RefnoEnum::from_str("17496/198104").expect("无效的 FRMW refno");
        let pipe_refno = RefnoEnum::from_str("24381/59217").expect("无效的管道 refno");

        println!("📍 目标 FRMW: {}", frmw_refno);
        println!("📍 测试管道: {}", pipe_refno);
        println!("📁 Mesh 目录: {}", mesh_dir.display());

        // 查询 FRMW 的面板
        let panels_sql = format!(
            r#"
            SELECT value array::flatten((
                SELECT value (SELECT value REFNO FROM PANE WHERE OWNER = $parent.REFNO) 
                FROM SBFR WHERE OWNER = $parent.REFNO
            )) FROM pe WHERE id = {}
            "#,
            frmw_refno.to_pe_key()
        );
        let panels: Vec<Vec<RefnoEnum>> = SUL_DB.query_take(&panels_sql, 0).await.unwrap_or_default();
        
        let panel_refnos: Vec<RefnoEnum> = panels.into_iter().flatten().collect();
        println!("📋 找到 {} 个面板", panel_refnos.len());

        if panel_refnos.is_empty() {
            println!("⚠️  未找到面板，无法进行房间计算");
            return Ok(());
        }

        // 使用第一个面板进行测试
        let test_panel = panel_refnos[0];
        println!("\n🔧 使用面板 {} 进行房间计算测试", test_panel);

        // 查询面板几何
        let panel_insts: Vec<GeomInstQuery> = query_insts(&[test_panel], true).await.unwrap_or_default();
        if panel_insts.is_empty() {
            println!("⚠️  面板 {} 没有几何实例", test_panel);
            return Ok(());
        }

        let panel_aabb: Aabb = panel_insts[0].world_aabb.into();
        println!("   面板 AABB: mins={:?}, maxs={:?}", panel_aabb.mins, panel_aabb.maxs);

        // 查询管道几何
        let pipe_insts: Vec<GeomInstQuery> = query_insts(&[pipe_refno], true).await.unwrap_or_default();
        if !pipe_insts.is_empty() {
            let pipe_aabb: Aabb = pipe_insts[0].world_aabb.into();
            println!("   管道 AABB: mins={:?}, maxs={:?}", pipe_aabb.mins, pipe_aabb.maxs);

            // 检查相交
            let intersects = panel_aabb.intersects(&pipe_aabb);
            println!("\n✅ 面板-管道 AABB 相交: {}", if intersects { "是" } else { "否" });
        }

        // 调用房间计算函数
        println!("\n🔄 调用 cal_room_refnos 进行房间计算...");
        let exclude_refnos: HashSet<RefnoEnum> = panel_refnos.iter().cloned().collect();
        
        let start = Instant::now();
        let result = crate::fast_model::room_model::cal_room_refnos(
            &mesh_dir,
            test_panel,
            &exclude_refnos,
            0.1,
        ).await;

        match result {
            Ok(within_refnos) => {
                println!("✅ 房间计算完成，耗时: {:?}", start.elapsed());
                println!("   找到 {} 个房间内构件", within_refnos.len());

                // 检查管道是否在结果中
                if within_refnos.contains(&pipe_refno) {
                    println!("   🎯 管道 {} 在房间内!", pipe_refno);
                } else {
                    println!("   ❌ 管道 {} 不在房间内", pipe_refno);
                }

                // 显示前 10 个结果
                for (i, refno) in within_refnos.iter().take(10).enumerate() {
                    println!("   - 构件 {}: {}", i + 1, refno);
                }
                if within_refnos.len() > 10 {
                    println!("   - ... 还有 {} 个构件", within_refnos.len() - 10);
                }
            }
            Err(e) => {
                println!("❌ 房间计算失败: {}", e);
            }
        }

        Ok(())
    }

}
