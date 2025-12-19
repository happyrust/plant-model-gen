/// 测试房间包含算法 - 验证三通是否在房间内
///
/// FRMW 参考号: 24381/35269
/// 待测试元件（三通）: 24383/73968
///
/// 测试流程:
/// 1. 初始化数据库连接
/// 2. 生成测试所需的模型数据 (FRMW 房间和 TEE 三通)
/// 3. 查询房间的面板数据
/// 4. 执行房间包含算法
/// 5. 验证三通是否在房间内

#[cfg(test)]
#[cfg(all(not(target_arch = "wasm32"), feature = "sqlite-index"))]
mod tests {
    use crate::fast_model::gen_model::orchestrator::gen_all_geos_data;
    use crate::options::DbOptionExt;
    use aios_core::rs_surreal::inst::query_insts;
    use aios_core::{get_db_option, init_surreal, RefnoEnum, SurrealQueryExt, SUL_DB};
    use anyhow::{Context, Result};
    use std::collections::HashSet;
    use std::str::FromStr;
    use std::time::Instant;

    #[tokio::test]
    #[ignore = "需要真实数据库连接"]
    async fn test_tee_in_room() -> Result<()> {
        println!("\n🏗️  房间包含算法测试 - 完整流程");
        println!("{}", "=".repeat(80));

        // ===== 步骤 1: 初始化数据库 =====
        println!("\n📡 步骤 1: 初始化 SurrealDB 连接");
        init_surreal().await.context("初始化 SurrealDB 失败")?;
        println!("✅ 数据库连接成功");

        let db_option = get_db_option();
        let mesh_dir = db_option.get_meshes_path();
        println!("   Mesh 目录: {}", mesh_dir.display());

        // ===== 步骤 2: 生成测试模型数据 =====
        println!("\n🔧 步骤 2: 生成测试模型数据");
        let frmw_refno = RefnoEnum::from_str("24381/35269").expect("invalid frmw refno");
        let tee_refno = RefnoEnum::from_str("24383/73968").expect("invalid tee refno");

        println!("   目标 FRMW (房间): {}", frmw_refno);
        println!("   目标 TEE (三通): {}", tee_refno);

        // 配置数据库选项
        let base_option = get_db_option();
        let mut db_option = DbOptionExt::from(base_option.clone());
        db_option.inner.gen_mesh = true;
        db_option.inner.apply_boolean_operation = true;
        db_option.full_noun_mode = false;

        // 使用 manual_refnos 指定要生成的模型
        let manual_refnos = vec![frmw_refno, tee_refno];

        println!("   配置:");
        println!("      gen_mesh: {}", db_option.inner.gen_mesh);
        println!("      apply_boolean_operation: {}", db_option.inner.apply_boolean_operation);
        println!("      full_noun_mode: {}", db_option.full_noun_mode);
        println!("      manual_refnos: {:?}", manual_refnos);

        println!("\n   正在生成模型 (这可能需要一些时间)...");
        let gen_start = Instant::now();
        match gen_all_geos_data(
            manual_refnos,
            &db_option,
            None, // 不使用增量更新
            None, // 不使用历史 sesno
        )
        .await
        {
            Ok(true) => {
                println!("   ✅ 模型生成成功 (耗时: {:?})", gen_start.elapsed());
            }
            Ok(false) => {
                println!("   ⚠️  模型生成返回 false");
                println!("   继续使用已存在的模型数据进行测试...");
            }
            Err(e) => {
                println!("   ⚠️  模型生成失败: {}", e);
                println!("   继续使用已存在的模型数据进行测试...");
            }
        }

        // ===== 步骤 3: 查询房间面板数据 =====
        println!("\n🔍 步骤 3: 查询房间面板数据");
        println!("   FRMW: {}, 三通: {}", frmw_refno, tee_refno);

        let sql = format!(
            "SELECT value REFNO FROM PANE WHERE OWNER.OWNER = {}",
            frmw_refno.refno().0
        );

        let panel_refnos: Vec<RefnoEnum> = SUL_DB.query_take(&sql, 0).await.unwrap_or_default();
        println!("   找到 {} 个面板", panel_refnos.len());

        if panel_refnos.is_empty() {
            println!("\n   ℹ️  直接查询未找到面板,尝试通过 SBFR 查询...");
            let sql2 = format!(
                "SELECT value REFNO FROM SBFR WHERE OWNER = {}",
                frmw_refno.refno().0
            );
            let sbfr_refnos: Vec<RefnoEnum> = SUL_DB.query_take(&sql2, 0).await.unwrap_or_default();
            println!("   SBFR 数量: {}", sbfr_refnos.len());

            if !sbfr_refnos.is_empty() {
                let sql3 = format!(
                    "SELECT value REFNO FROM PANE WHERE OWNER = {}",
                    sbfr_refnos[0].refno().0
                );
                let pane_refnos: Vec<RefnoEnum> = SUL_DB.query_take(&sql3, 0).await.unwrap_or_default();
                println!("   第一个 SBFR 下的 PANE 数量: {}", pane_refnos.len());
            }

            println!("\n❌ 没有找到面板数据");
            println!("   可能的原因:");
            println!("   1. 模型数据尚未生成到数据库");
            println!("   2. FRMW 参考号不正确");
            println!("   3. 数据库结构发生变化");
            return Ok(());
        }

        // ===== 步骤 4: 执行房间包含算法 =====
        println!("\n🧮 步骤 4: 执行房间包含算法");
        let exclude_refnos: HashSet<RefnoEnum> = panel_refnos.iter().cloned().collect();
        let mut found = false;

        for (i, panel_refno) in panel_refnos.iter().enumerate() {
            let panel_insts = query_insts(&[*panel_refno], true).await.unwrap_or_default();
            if panel_insts.is_empty() {
                println!("   ⚠️  面板 {} 没有实例数据,跳过", panel_refno);
                continue;
            }

            println!("\n   [{}/{}] 测试面板: {}", i + 1, panel_refnos.len(), panel_refno);
            println!("         面板实例数: {}", panel_insts.len());

            let start = Instant::now();
            let result = crate::fast_model::room_model::cal_room_refnos(
                &mesh_dir,
                *panel_refno,
                &exclude_refnos,
                0.1,
            )
            .await?;

            println!("         耗时: {:?}, 包含 {} 个构件", start.elapsed(), result.len());

            if result.contains(&tee_refno) {
                println!("         ✅ 三通在此面板内!");
                found = true;
                break;
            } else {
                println!("         ❌ 三通不在此面板内");
            }
        }

        // ===== 步骤 5: 测试结果 =====
        println!("\n{}", "=".repeat(80));
        println!("📊 测试结果:");
        if found {
            println!("   ✅ 三通 {} 在房间 {} 内", tee_refno, frmw_refno);
        } else {
            println!("   ❌ 三通 {} 不在房间 {} 内", tee_refno, frmw_refno);
            println!("\n   可能的原因:");
            println!("   1. 三通的 mesh 数据不存在或损坏");
            println!("   2. 三通实际上不在房间内 (几何位置问题)");
            println!("   3. 容差参数 (0.1) 不合适");
        }
        println!("{}", "=".repeat(80));

        Ok(())
    }

    /// 简化版测试 - 仅测试模型生成步骤
    #[tokio::test]
    #[ignore = "需要真实数据库连接"]
    async fn test_model_generation_only() -> Result<()> {
        println!("\n🔧 模型生成测试");
        println!("{}", "=".repeat(80));

        init_surreal().await.context("初始化 SurrealDB 失败")?;
        println!("✅ 数据库连接成功");

        let frmw_refno = RefnoEnum::from_str("24381/35269").expect("invalid frmw refno");
        let tee_refno = RefnoEnum::from_str("24383/73968").expect("invalid tee refno");

        // 配置数据库选项
        let base_option = get_db_option();
        let mut db_option = DbOptionExt::from(base_option.clone());
        db_option.inner.gen_mesh = true;
        db_option.inner.apply_boolean_operation = true;
        db_option.full_noun_mode = false;

        let manual_refnos = vec![frmw_refno, tee_refno];

        println!("\n正在生成模型...");
        println!("  FRMW: {}", frmw_refno);
        println!("  TEE: {}", tee_refno);

        let start = Instant::now();
        let success = gen_all_geos_data(manual_refnos, &db_option, None, None).await?;

        if success {
            println!("\n✅ 模型生成成功");
            println!("   耗时: {:?}", start.elapsed());
        } else {
            println!("\n⚠️  模型生成返回 false");
        }

        Ok(())
    }
}
