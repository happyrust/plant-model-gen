/// 检查 FRMW 24381/35269 的数据库结构
///
/// 这个测试用于诊断为什么找不到房间面板数据

#[cfg(test)]
#[cfg(all(not(target_arch = "wasm32"), feature = "sqlite-index"))]
mod tests {
    use aios_core::{init_surreal, RefnoEnum, SUL_DB, SurrealQueryExt};
    use anyhow::Result;
    use std::str::FromStr;

    #[tokio::test]
    #[ignore = "需要真实数据库连接"]
    async fn test_check_frmw_structure() -> Result<()> {
        println!("\n🔍 检查 FRMW 24381/35269 的数据库结构");
        println!("{}", "=".repeat(80));

        init_surreal().await?;
        println!("✅ 数据库连接成功\n");

        let frmw_refno = RefnoEnum::from_str("24381/35269").expect("invalid frmw refno");
        let tee_refno = RefnoEnum::from_str("24383/73968").expect("invalid tee refno");

        println!("目标 FRMW: {}", frmw_refno);
        println!("目标 TEE: {}", tee_refno);

        // 1. 检查 FRMW 节点类型
        println!("\n📋 步骤 1: 检查 FRMW 节点类型");
        let sql1 = format!(
            "SELECT type, NAME FROM type::record('inst', {}) LIMIT 1",
            frmw_refno.refno().0
        );
        let result1: Vec<serde_json::Value> =
            SUL_DB.query_take(&sql1, 0).await.unwrap_or_default();
        println!("   FRMW 节点信息: {:?}", result1);

        // 2. 查询直接子节点
        println!("\n📋 步骤 2: 查询 FRMW 的直接子节点");
        let sql2 = format!(
            "SELECT value REFNO, type FROM inst WHERE OWNER = {} LIMIT 20",
            frmw_refno.refno().0
        );
        let result2: Vec<serde_json::Value> =
            SUL_DB.query_take(&sql2, 0).await.unwrap_or_default();
        println!("   找到 {} 个直接子节点:", result2.len());
        for (i, item) in result2.iter().enumerate() {
            println!("      [{}] {:?}", i + 1, item);
        }

        // 3. 尝试不同的 PANE 查询方式
        println!("\n📋 步骤 3: 尝试不同方式查询 PANE");

        // 3.1 OWNER.OWNER = FRMW
        let sql3_1 = format!(
            "SELECT value REFNO FROM PANE WHERE OWNER.OWNER = {}",
            frmw_refno.refno().0
        );
        let result3_1: Vec<RefnoEnum> = SUL_DB.query_take(&sql3_1, 0).await.unwrap_or_default();
        println!("   3.1 PANE WHERE OWNER.OWNER = FRMW: {} 个", result3_1.len());

        // 3.2 OWNER = FRMW
        let sql3_2 = format!(
            "SELECT value REFNO FROM PANE WHERE OWNER = {}",
            frmw_refno.refno().0
        );
        let result3_2: Vec<RefnoEnum> = SUL_DB.query_take(&sql3_2, 0).await.unwrap_or_default();
        println!("   3.2 PANE WHERE OWNER = FRMW: {} 个", result3_2.len());

        // 3.3 查询所有 PANE 并检查层级
        let sql3_3 = "SELECT value REFNO, OWNER, OWNER.OWNER FROM PANE LIMIT 10".to_string();
        let result3_3: Vec<serde_json::Value> =
            SUL_DB.query_take(&sql3_3, 0).await.unwrap_or_default();
        println!("   3.3 示例 PANE 结构 (前10个):");
        for (i, item) in result3_3.iter().enumerate() {
            println!("      [{}] {:?}", i + 1, item);
        }

        // 4. 查询 SBFR
        println!("\n📋 步骤 4: 查询 SBFR");
        let sql4 = format!(
            "SELECT value REFNO FROM SBFR WHERE OWNER = {}",
            frmw_refno.refno().0
        );
        let result4: Vec<RefnoEnum> = SUL_DB.query_take(&sql4, 0).await.unwrap_or_default();
        println!("   SBFR WHERE OWNER = FRMW: {} 个", result4.len());

        // 5. 检查三通的位置和所属关系
        println!("\n📋 步骤 5: 检查三通的位置");
        let sql5 = format!(
            "SELECT type, NAME, OWNER, OWNER.type, OWNER.NAME FROM type::record('inst', {}) LIMIT 1",
            tee_refno.refno().0
        );
        let result5: Vec<serde_json::Value> =
            SUL_DB.query_take(&sql5, 0).await.unwrap_or_default();
        println!("   三通节点信息: {:?}", result5);

        // 6. 查询三通所在的管道或分支
        println!("\n📋 步骤 6: 查询三通所在的层级结构");
        let sql6 = format!(
            "SELECT OWNER, OWNER.OWNER, OWNER.OWNER.OWNER, OWNER.OWNER.OWNER.OWNER FROM type::record('inst', {}) LIMIT 1",
            tee_refno.refno().0
        );
        let result6: Vec<serde_json::Value> =
            SUL_DB.query_take(&sql6, 0).await.unwrap_or_default();
        println!("   三通的层级结构: {:?}", result6);

        println!("\n{}", "=".repeat(80));
        println!("✅ 检查完成");

        Ok(())
    }
}
