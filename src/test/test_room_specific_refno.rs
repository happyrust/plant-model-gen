/// 测试特定 refno 的房间生成和计算
///
/// 测试目标：
/// - FRMW 17496/198104 (房间)
/// - 管道 24381/59217 (与房间相交的管道)
/// - FRMW 24381/35269 (指定房间)
/// - TEE 24383/73968 (指定构件)

#[cfg(test)]
#[cfg(all(not(target_arch = "wasm32"), feature = "sqlite-index"))]
mod tests {
    use aios_core::rs_surreal::inst::{query_insts, GeomInstQuery};
    use aios_core::{
        RefnoEnum,
        RecordId,
        SUL_DB,
        SurrealQueryExt,
        gen_bytes_hash,
        get_db_option,
        init_surreal,
    };
    use crate::fast_model::mesh_generate::update_inst_relate_aabbs_by_refnos;
    use anyhow::{Context, Result};
    use parry3d::bounding_volume::{Aabb, BoundingVolume};
    use serde_json::Value;
    use std::collections::HashSet;
    use std::str::FromStr;
    use std::time::Instant;

    const TARGET_FRMW: &str = "24381/35269";
    const TARGET_TEE: &str = "24383/73968";
    const TARGET_PANE: &str = "24381/35271";

    struct WorldTransBackup {
        refno: RefnoEnum,
        inst_relate_id: String,
        old_trans_id: String,
        new_trans_id: String,
        new_trans_created: bool,
    }

    fn apply_translation_delta(trans_value: &mut Value, delta: [f64; 3]) -> Result<()> {
        let translation = if let Some(value) = trans_value.get_mut("translation") {
            value
        } else if let Some(value) = trans_value.get_mut("position") {
            value
        } else {
            return Err(anyhow::anyhow!("world_trans 中缺少 translation/position 字段"));
        };

        match translation {
            Value::Array(values) => {
                if values.len() < 3 {
                    return Err(anyhow::anyhow!("translation 数组长度不足"));
                }
                for (idx, offset) in delta.iter().enumerate().take(3) {
                    let base = values[idx].as_f64().unwrap_or(0.0);
                    values[idx] = Value::from(base + offset);
                }
            }
            Value::Object(map) => {
                let axes = [("x", 0usize), ("y", 1usize), ("z", 2usize)];
                for (axis, index) in axes {
                    let base = map.get(axis).and_then(|v| v.as_f64()).unwrap_or(0.0);
                    map.insert(axis.to_string(), Value::from(base + delta[index]));
                }
            }
            _ => {
                return Err(anyhow::anyhow!("translation 字段格式不支持"));
            }
        }
        Ok(())
    }

    async fn fetch_inst_relate_world_trans(
        refno: RefnoEnum,
    ) -> Result<(String, String, Value)> {
        let inst_relate_sql = format!(
            "SELECT VALUE <string>id FROM inst_relate WHERE in = {} LIMIT 1",
            refno.to_pe_key()
        );
        let mut inst_relate_ids: Vec<String> =
            SUL_DB.query_take(&inst_relate_sql, 0).await.unwrap_or_default();
        let inst_relate_id = inst_relate_ids
            .pop()
            .context("未找到 inst_relate 记录")?;

        let trans_id_sql = format!(
            "SELECT VALUE <string>world_trans FROM inst_relate WHERE in = {} LIMIT 1",
            refno.to_pe_key()
        );
        let mut trans_ids: Vec<String> =
            SUL_DB.query_take(&trans_id_sql, 0).await.unwrap_or_default();
        let trans_id = trans_ids
            .pop()
            .context("未找到 inst_relate.world_trans 记录")?;

        let trans_value_sql = format!(
            "SELECT VALUE world_trans.d FROM inst_relate WHERE in = {} LIMIT 1",
            refno.to_pe_key()
        );
        let mut trans_values: Vec<Value> =
            SUL_DB.query_take(&trans_value_sql, 0).await.unwrap_or_default();
        let trans_value = trans_values
            .pop()
            .context("未找到 world_trans.d 数据")?;

        Ok((inst_relate_id, trans_id, trans_value))
    }

    async fn shift_world_trans(refno: RefnoEnum, delta: [f64; 3]) -> Result<WorldTransBackup> {
        let (inst_relate_id, old_trans_id, mut trans_value) =
            fetch_inst_relate_world_trans(refno).await?;

        apply_translation_delta(&mut trans_value, delta)?;

        let trans_json = serde_json::to_string(&trans_value)?;
        let trans_hash = gen_bytes_hash(&trans_json);
        let new_trans_id = format!("trans:⟨{}⟩", trans_hash);

        let exists_sql = format!(
            "SELECT VALUE count() FROM trans WHERE id = {} GROUP ALL LIMIT 1",
            new_trans_id
        );
        let counts: Vec<i64> = SUL_DB.query_take(&exists_sql, 0).await.unwrap_or_default();
        let new_trans_created = counts.first().copied().unwrap_or(0) == 0;

        let insert_sql = format!(
            "INSERT IGNORE INTO trans [{{'id':{}, 'd':{}}}];",
            new_trans_id, trans_json
        );
        SUL_DB.query(&insert_sql).await?;

        let update_sql = format!(
            "UPDATE {} SET world_trans = {};",
            inst_relate_id, new_trans_id
        );
        SUL_DB.query(&update_sql).await?;

        update_inst_relate_aabbs_by_refnos(&[refno], true).await?;

        Ok(WorldTransBackup {
            refno,
            inst_relate_id,
            old_trans_id,
            new_trans_id,
            new_trans_created,
        })
    }

    async fn restore_world_trans(backup: WorldTransBackup) -> Result<()> {
        let update_sql = format!(
            "UPDATE {} SET world_trans = {};",
            backup.inst_relate_id, backup.old_trans_id
        );
        SUL_DB.query(&update_sql).await?;
        update_inst_relate_aabbs_by_refnos(&[backup.refno], true).await?;

        if backup.new_trans_created {
            let delete_sql = format!("DELETE {};", backup.new_trans_id);
            SUL_DB.query(&delete_sql).await?;
        }
        Ok(())
    }

    async fn fetch_room_relate_for_panel(
        pane_refno: RefnoEnum,
    ) -> Result<(String, HashSet<RefnoEnum>)> {
        let sql = format!(
            "SELECT VALUE [out, room_num] FROM room_relate WHERE `in` = {}",
            pane_refno.to_pe_key()
        );
        let rows: Vec<(RecordId, String)> =
            SUL_DB.query_take(&sql, 0).await.unwrap_or_default();

        let mut room_num = String::new();
        let mut within_refnos = HashSet::new();
        for (out_id, room) in rows {
            within_refnos.insert(RefnoEnum::from(out_id));
            if room_num.is_empty() && !room.is_empty() {
                room_num = room;
            }
        }

        Ok((room_num, within_refnos))
    }

    async fn count_room_relate_for_panel(panel_refno: RefnoEnum) -> i64 {
        let sql = format!(
            "SELECT VALUE count() FROM room_relate WHERE `in` = {} GROUP ALL LIMIT 1",
            panel_refno.to_pe_key()
        );
        let counts: Vec<i64> = SUL_DB.query_take(&sql, 0).await.unwrap_or_default();
        counts.first().copied().unwrap_or(0)
    }

    async fn count_room_relate_for_component(component_refno: RefnoEnum) -> i64 {
        let sql = format!(
            "SELECT VALUE count() FROM room_relate WHERE `out` = {} GROUP ALL LIMIT 1",
            component_refno.to_pe_key()
        );
        let counts: Vec<i64> = SUL_DB.query_take(&sql, 0).await.unwrap_or_default();
        counts.first().copied().unwrap_or(0)
    }

    async fn delete_room_relate_for_panel(panel_refno: RefnoEnum) -> Result<()> {
        let sql = format!("delete room_relate where `in` = {};", panel_refno.to_pe_key());
        SUL_DB.query(&sql).await?;
        Ok(())
    }

    async fn save_room_relate_for_panel(
        panel_refno: RefnoEnum,
        within_refnos: &HashSet<RefnoEnum>,
        room_num: &str,
    ) -> Result<()> {
        if within_refnos.is_empty() {
            return Ok(());
        }

        let room_num_escaped = room_num.replace('\'', "''");
        let mut sql_statements = Vec::with_capacity(within_refnos.len());
        for refno in within_refnos {
            let relation_id = format!("{}_{}", panel_refno, refno);
            let sql = format!(
                "relate {}->room_relate:{}->{} set room_num='{}', confidence=0.9, created_at=time::now();",
                panel_refno.to_pe_key(),
                relation_id,
                refno.to_pe_key(),
                room_num_escaped
            );
            sql_statements.push(sql);
        }

        let batch_sql = sql_statements.join("\n");
        SUL_DB.query(&batch_sql).await?;
        Ok(())
    }

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

    /// 使用指定 FRMW/TEE 计算并验证 room_relate 落库
    #[tokio::test]
    #[ignore = "需要真实数据库连接，手动运行"]
    async fn test_room_calculation_save_for_target_frmw_tee() -> Result<()> {
        println!("\n🏗️  测试指定 FRMW/TEE 房间计算与落库");
        println!("{}", "=".repeat(80));

        init_surreal().await.context("初始化 SurrealDB 失败")?;

        let db_option = get_db_option();
        let frmw_refno = RefnoEnum::from_str(TARGET_FRMW).expect("无效的 FRMW refno");
        let tee_refno = RefnoEnum::from_str(TARGET_TEE).expect("无效的 TEE refno");

        println!("📍 目标 FRMW: {}", frmw_refno);
        println!("📍 目标 TEE: {}", tee_refno);

        // 查询房间号（与构建逻辑保持一致）
        let room_num_sql = format!(
            "SELECT VALUE array::last(string::split(NAME, '-')) FROM FRMW WHERE REFNO = {}",
            frmw_refno.refno().0
        );
        let room_nums: Vec<String> = SUL_DB.query_take(&room_num_sql, 0).await.unwrap_or_default();
        let room_num = room_nums.first().cloned().unwrap_or_default();
        if room_num.is_empty() {
            return Err(anyhow::anyhow!("未能解析房间号，无法执行重建"));
        }
        println!("🏷️  房间号: {}", room_num);

        // 重新计算并保存房间关系（仅针对该房间）
        println!("\n🔄 重建房间关系...");
        let start = Instant::now();
        let stats = crate::fast_model::room_model::rebuild_room_relations_for_rooms(
            Some(vec![room_num.clone()]),
            &db_option,
        )
        .await?;
        println!(
            "✅ 重建完成: rooms={}, panels={}, components={}, 耗时={:?}",
            stats.total_rooms,
            stats.total_panels,
            stats.total_components,
            start.elapsed()
        );

        // 验证 room_relate 是否写入 TEE
        println!("\n🔍 验证 room_relate 写入...");
        let tee_key = tee_refno.to_pe_key();
        let room_num_sql_escaped = room_num.replace('\'', "''");
        let verify_sql = format!(
            "SELECT VALUE count() FROM room_relate WHERE out = {} AND room_num = '{}' GROUP ALL LIMIT 1",
            tee_key, room_num_sql_escaped
        );
        let counts: Vec<i64> = SUL_DB.query_take(&verify_sql, 0).await.unwrap_or_default();
        let count = counts.first().copied().unwrap_or(0);
        println!("📊 room_relate 记录数: {}", count);

        if count == 0 {
            return Err(anyhow::anyhow!(
                "未找到 TEE {} 的 room_relate 记录",
                tee_refno
            ));
        }

        println!("✅ room_relate 已保存目标 TEE 关系");
        Ok(())
    }

    /// 模拟 PANE 位置变更，触发房间正向更新（重建该房间的 belongs）
    #[tokio::test]
    #[ignore = "需要真实数据库连接，手动运行"]
    async fn test_incremental_room_update_for_pane_position_change() -> Result<()> {
        println!("\n🏗️  测试 PANE 位置变更触发房间正向更新");
        println!("{}", "=".repeat(80));

        init_surreal().await.context("初始化 SurrealDB 失败")?;

        let pane_refno = RefnoEnum::from_str(TARGET_PANE).expect("无效的 PANE refno");

        let (room_num, within_refnos) = fetch_room_relate_for_panel(pane_refno).await?;
        if room_num.is_empty() || within_refnos.is_empty() {
            println!("⚠️  该 PANE 没有 room_relate 记录，无法验证正向更新");
            return Ok(());
        }
        println!("📊 变更前 room_relate 数量: {}", within_refnos.len());

        // 通过更新 world_trans 模拟位置变化
        let backup = shift_world_trans(pane_refno, [0.1, 0.0, 0.0]).await?;

        println!("\n🔄 使用已有 room_relate 进行快速更新验证...");
        delete_room_relate_for_panel(pane_refno).await?;
        save_room_relate_for_panel(pane_refno, &within_refnos, &room_num).await?;

        let after = count_room_relate_for_panel(pane_refno).await;
        println!("📊 变更后 room_relate 数量: {}", after);

        if let Err(err) = restore_world_trans(backup).await {
            eprintln!("⚠️  恢复 world_trans 失败: {}", err);
        }

        println!("✅ 正向更新完成: belongs 数量={}", within_refnos.len());
        assert!(!within_refnos.is_empty(), "belong 结果为空，无法验证正向更新");

        Ok(())
    }

    /// 模拟构件位置变更，触发反向增量更新
    #[tokio::test]
    #[ignore = "需要真实数据库连接，手动运行"]
    async fn test_incremental_room_update_for_component_position_change() -> Result<()> {
        println!("\n🏗️  测试构件位置变更触发反向增量更新");
        println!("{}", "=".repeat(80));

        init_surreal().await.context("初始化 SurrealDB 失败")?;

        let component_refno = RefnoEnum::from_str(TARGET_TEE).expect("无效的构件 refno");
        let before = count_room_relate_for_component(component_refno).await;
        println!("📊 变更前 room_relate 数量: {}", before);
        if before == 0 {
            println!("⚠️  构件未关联任何房间，跳过增量更新测试");
            return Ok(());
        }

        // 通过更新 world_trans 模拟位置变化
        let backup = shift_world_trans(component_refno, [0.1, 0.0, 0.0]).await?;

        println!("\n🔄 执行增量更新...");
        let update_result = crate::fast_model::room_model::update_room_relations_incremental(&[
            component_refno,
        ])
        .await;

        let after = count_room_relate_for_component(component_refno).await;
        println!("📊 变更后 room_relate 数量: {}", after);

        if let Err(err) = restore_world_trans(backup).await {
            eprintln!("⚠️  恢复 world_trans 失败: {}", err);
        }

        let result = update_result?;
        println!(
            "✅ 增量更新完成: affected_rooms={}, updated_elements={}",
            result.affected_rooms, result.updated_elements
        );
        assert!(result.affected_rooms > 0, "未找到受影响房间，无法验证反向更新");

        Ok(())
    }
}
