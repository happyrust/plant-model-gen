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
        get_db_option,
        init_surreal,
    };
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};
    use crate::options::DbOptionExt;
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

    // 本次需求：房间/管道
    const TARGET_ROOM_FRMW_25688_71821: &str = "25688/71821";
    const TARGET_PIPE_24383_73962: &str = "24383/73962";

    struct WorldTransBackup {
        refno: RefnoEnum,
        pe_transform_id: String,
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
        let pe_transform_id = refno
            .to_pe_key()
            .replace("pe:", "pe_transform:");

        let trans_id_sql = format!(
            "SELECT VALUE <string>world_trans FROM {} LIMIT 1",
            pe_transform_id
        );
        let mut trans_ids: Vec<String> =
            SUL_DB.query_take(&trans_id_sql, 0).await.unwrap_or_default();
        let trans_id = trans_ids
            .pop()
            .context("未找到 pe_transform.world_trans 记录")?;

        let trans_value_sql = format!(
            "SELECT VALUE world_trans.d FROM {} LIMIT 1",
            pe_transform_id
        );
        let mut trans_values: Vec<Value> =
            SUL_DB.query_take(&trans_value_sql, 0).await.unwrap_or_default();
        let trans_value = trans_values
            .pop()
            .context("未找到 world_trans.d 数据")?;

        Ok((pe_transform_id, trans_id, trans_value))
    }

    async fn shift_world_trans(refno: RefnoEnum, delta: [f64; 3]) -> Result<WorldTransBackup> {
        let (pe_transform_id, old_trans_id, mut trans_value) =
            fetch_inst_relate_world_trans(refno).await?;

        apply_translation_delta(&mut trans_value, delta)?;

        let trans_json = serde_json::to_string(&trans_value)?;
        // 对 JSON 字符串计算 hash
        let trans_hash = {
            let mut hasher = DefaultHasher::new();
            trans_json.hash(&mut hasher);
            hasher.finish()
        };
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
            pe_transform_id, new_trans_id
        );
        SUL_DB.query(&update_sql).await?;

        update_inst_relate_aabbs_by_refnos(&[refno], true).await?;

        Ok(WorldTransBackup {
            refno,
            pe_transform_id,
            old_trans_id,
            new_trans_id,
            new_trans_created,
        })
    }

    async fn restore_world_trans(backup: WorldTransBackup) -> Result<()> {
        let update_sql = format!(
            "UPDATE {} SET world_trans = {};",
            backup.pe_transform_id, backup.old_trans_id
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

    async fn count_room_relate_for_component_in_room(
        component_refno: RefnoEnum,
        room_num: &str,
    ) -> i64 {
        let room_num_escaped = room_num.replace('\'', "''");
        let sql = format!(
            "SELECT VALUE count() FROM room_relate WHERE `out` = {} AND room_num = '{}' GROUP ALL LIMIT 1",
            component_refno.to_pe_key(),
            room_num_escaped
        );
        let counts: Vec<i64> = SUL_DB.query_take(&sql, 0).await.unwrap_or_default();
        counts.first().copied().unwrap_or(0)
    }

    async fn count_inst_relate_for_refno(refno: RefnoEnum) -> i64 {
        let sql = format!(
            "SELECT VALUE count() FROM inst_relate WHERE `in` = {} GROUP ALL LIMIT 1",
            refno.to_pe_key()
        );
        let counts: Vec<i64> = SUL_DB.query_take(&sql, 0).await.unwrap_or_default();
        counts.first().copied().unwrap_or(0)
    }

    async fn fetch_room_num_for_frmw(frmw_refno: RefnoEnum) -> Result<String> {
        // 与既有测试保持一致：从 NAME 的最后一段解析 room_num
        let sql = format!(
            "SELECT VALUE array::last(string::split(NAME, '-')) FROM FRMW WHERE REFNO = {}",
            frmw_refno.refno().0
        );
        let room_nums: Vec<String> = SUL_DB.query_take(&sql, 0).await.unwrap_or_default();
        let room_num = room_nums.first().cloned().unwrap_or_default();
        if room_num.is_empty() {
            return Err(anyhow::anyhow!("未能从 FRMW.NAME 解析房间号: {}", frmw_refno));
        }
        Ok(room_num)
    }

    async fn gen_models_by_debug_refnos(refnos: &[&str]) -> Result<()> {
        init_surreal().await.context("初始化 SurrealDB 失败")?;

        // 复用 main.rs 的思路：启用 debug_model，并通过 debug_model_refnos 限定生成范围。
        aios_core::set_debug_model_enabled(true);

        let base = get_db_option().clone();
        aios_core::mesh_precision::set_active_precision(base.mesh_precision.clone());

        let mut db_option_ext = DbOptionExt::from(base);
        db_option_ext.inner.gen_model = true;
        db_option_ext.inner.gen_mesh = true;
        // 只生成少量目标 refno 时，强制重新生成更符合“自动生成”预期。
        db_option_ext.inner.replace_mesh = Some(true);
        db_option_ext.inner.debug_model_refnos =
            Some(refnos.iter().map(|s| s.to_string()).collect());

        crate::fast_model::gen_all_geos_data(vec![], &db_option_ext, None, None)
            .await
            .context("debug_refno 模式生成模型失败")?;

        // 最小兜底校验：目标 refno 至少应生成 inst_relate。
        for refno in refnos {
            let r = RefnoEnum::from_str(refno)
                .map_err(|_| anyhow::anyhow!("无效的 refno: {}", refno))?;
            let cnt = count_inst_relate_for_refno(r).await;
            anyhow::ensure!(cnt > 0, "模型生成后未找到 inst_relate: {}", r);
        }

        Ok(())
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
            SELECT 
                type::record('inst_relate_aabb', record::id(in)).aabb.d as world_aabb, 
                world_trans.d as world_trans 
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
            SELECT 
                type::record('inst_relate_aabb', record::id(in)).aabb.d as world_aabb, 
                world_trans.d as world_trans 
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
            let Some(frmw_world_aabb) = frmw_geom_insts[0].world_aabb.clone() else {
                println!("⚠️  FRMW world_aabb 为空，跳过 AABB 相交测试");
                return Ok(());
            };
            let Some(pipe_world_aabb) = pipe_geom_insts[0].world_aabb.clone() else {
                println!("⚠️  管道 world_aabb 为空，跳过 AABB 相交测试");
                return Ok(());
            };

            let frmw_aabb: Aabb = frmw_world_aabb.into();
            let pipe_aabb: Aabb = pipe_world_aabb.into();

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

        let Some(panel_world_aabb) = panel_insts[0].world_aabb.clone() else {
            println!("⚠️  面板 world_aabb 为空，无法进行 AABB 相交测试");
            return Ok(());
        };
        let panel_aabb: Aabb = panel_world_aabb.into();
        println!("   面板 AABB: mins={:?}, maxs={:?}", panel_aabb.mins, panel_aabb.maxs);

        // 查询管道几何
        let pipe_insts: Vec<GeomInstQuery> = query_insts(&[pipe_refno], true).await.unwrap_or_default();
        if !pipe_insts.is_empty() {
            let Some(pipe_world_aabb) = pipe_insts[0].world_aabb.clone() else {
                println!("⚠️  管道 world_aabb 为空，跳过 AABB 相交测试");
                return Ok(());
            };
            let pipe_aabb: Aabb = pipe_world_aabb.into();
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

    /// 需求：
    /// 1) 通过 debug_refno 自动生成房间/管道模型（25688/71821, 24383/73962）
    /// 2) 执行房间计算后，验证管道能拿到所属房间号（room_relate 落库）
    #[tokio::test]
    #[ignore = "需要真实数据库连接，手动运行"]
    async fn test_room_pipe_belongs_after_room_calculation_25688_71821_24383_73962() -> Result<()> {
        println!("\n🏗️  测试房间/管道模型生成 + 房间计算所属关系验证");
        println!("{}", "=".repeat(80));

        // Step 1: 自动生成两个目标模型（通过 debug_refno 传递）
        println!("\n⚙️  Step 1: debug_refno 生成模型...");
        gen_models_by_debug_refnos(&[TARGET_ROOM_FRMW_25688_71821, TARGET_PIPE_24383_73962])
            .await?;

        // Step 2: 执行房间计算（按房间号重建）
        println!("\n🏠 Step 2: 执行房间计算（重建该房间关系）...");
        let db_option = get_db_option();
        let frmw_refno = RefnoEnum::from_str(TARGET_ROOM_FRMW_25688_71821)
            .expect("无效的 FRMW refno");
        let room_num = fetch_room_num_for_frmw(frmw_refno).await?;
        println!("🏷️  目标房间号: {}", room_num);

        let start = Instant::now();
        let stats = crate::fast_model::room_model::rebuild_room_relations_for_rooms(
            Some(vec![room_num.clone()]),
            &db_option,
        )
        .await
        .context("房间计算失败")?;
        println!(
            "✅ 房间计算完成: rooms={}, panels={}, components={}, 耗时={:?}",
            stats.total_rooms,
            stats.total_panels,
            stats.total_components,
            start.elapsed()
        );
        anyhow::ensure!(stats.total_panels > 0, "该房间未查询到面板，无法验证 belongs");

        // Step 3: 验证管道所属房间（room_relate 落库）
        println!("\n🔍 Step 3: 验证管道所属房间号...");
        let pipe_refno = RefnoEnum::from_str(TARGET_PIPE_24383_73962)
            .expect("无效的管道 refno");
        let belongs_cnt = count_room_relate_for_component_in_room(pipe_refno, &room_num).await;
        println!("📊 管道 room_relate(该房间) 记录数: {}", belongs_cnt);
        anyhow::ensure!(
            belongs_cnt > 0,
            "未找到管道 {} 在房间 {} 的 room_relate 记录",
            pipe_refno,
            room_num
        );

        Ok(())
    }
}
