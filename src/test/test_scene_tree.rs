/// 测试 Scene Tree 模块
///
/// 测试目标：
/// - 使用 dbnum 1112
/// - 测试场景树初始化
/// - 测试查询未生成的几何叶子节点
/// - 测试 AABB 更新和生成状态标记
///
/// 运行方式：
/// ```bash
/// cargo test test_scene_tree --features gen_model -- --nocapture
/// ```

#[cfg(test)]
#[cfg(all(feature = "gen_model", not(target_arch = "wasm32")))]
mod tests {
    use crate::scene_tree;
    use aios_core::{RefU64, RefnoEnum, SurrealQueryExt, project_primary_db};

    const TEST_DBNO: i32 = 1112;

    /// 测试场景树 Schema 初始化
    #[tokio::test]
    async fn test_scene_tree_schema() {
        println!("=== 测试场景树 Schema 初始化 ===");

        match scene_tree::init_schema().await {
            Ok(_) => {
                println!("✓ Schema 初始化成功");
            }
            Err(e) => {
                println!("✗ Schema 初始化失败: {}", e);
            }
        }
    }

    /// 测试生成状态查询（单元测试）
    #[tokio::test]
    async fn test_generation_status_unit() {
        println!("=== 测试生成状态查询 ===");

        // 创建测试 refnos
        let test_refnos = vec![
            RefnoEnum::from(RefU64(104679055498)), // 24383_73962
            RefnoEnum::from(RefU64(104679055499)), // 24383_73963
        ];

        println!("测试 refnos:");
        for refno in &test_refnos {
            println!("  - {}", refno);
        }

        // 查询生成状态
        match scene_tree::query_generation_status(&test_refnos).await {
            Ok(statuses) => {
                println!("✓ 查询成功，返回 {} 条记录", statuses.len());
                for status in statuses {
                    let refno = RefnoEnum::from(RefU64(status.id as u64));
                    println!(
                        "  - {}: has_geo={}, generated={}",
                        refno, status.has_geo, status.generated
                    );
                }
            }
            Err(e) => {
                println!("✗ 查询失败: {}", e);
            }
        }
    }

    /// 测试标记为已生成
    #[tokio::test]
    async fn test_mark_generated_unit() {
        println!("=== 测试标记为已生成 ===");

        // 创建测试节点 ID
        let test_ids = vec![104679055498i64, 104679055499i64];

        println!("测试节点 ID:");
        for id in &test_ids {
            println!("  - {}", id);
        }

        // 标记为已生成
        match scene_tree::mark_as_generated(&test_ids).await {
            Ok(_) => {
                println!("✓ 标记成功");
            }
            Err(e) => {
                println!("✗ 标记失败: {}", e);
            }
        }
    }

    /// 测试过滤未生成的几何节点
    #[tokio::test]
    async fn test_filter_ungenerated_unit() {
        println!("=== 测试过滤未生成的几何节点 ===");

        // 创建测试 refnos
        let test_refnos = vec![
            RefnoEnum::from(RefU64(104679055498)), // 24383_73962
            RefnoEnum::from(RefU64(104679055499)), // 24383_73963
        ];

        println!("测试 refnos:");
        for refno in &test_refnos {
            println!("  - {}", refno);
        }

        // 过滤未生成的几何节点
        match scene_tree::filter_ungenerated_geo_nodes(&test_refnos).await {
            Ok(ungenerated) => {
                println!("✓ 过滤成功，未生成的几何节点数: {}", ungenerated.len());
                for id in &ungenerated {
                    let refno = RefnoEnum::from(RefU64(*id as u64));
                    println!("  - {}", refno);
                }
            }
            Err(e) => {
                println!("✗ 过滤失败: {}", e);
            }
        }
    }

    /// 测试初始化检查函数
    #[tokio::test]
    async fn test_is_initialized() {
        println!("=== 测试初始化检查函数 ===");

        match scene_tree::is_initialized().await {
            Ok(initialized) => {
                println!("✓ 检查成功，已初始化: {}", initialized);
            }
            Err(e) => {
                println!("✗ 检查失败: {}", e);
            }
        }
    }

    /// 测试按 dbnum 初始化 Scene Tree (dbnum=1112)
    #[tokio::test]
    async fn test_init_scene_tree_by_dbno_1112() {
        println!("=== 测试按 dbnum 初始化 Scene Tree (dbnum=1112) ===");

        // 先初始化数据库连接
        if let Err(e) = aios_core::init_surreal().await {
            let msg = e.to_string();
            if !msg.contains("Already connected") {
                println!("✗ 数据库连接失败: {}", e);
                return;
            }
        }

        // 按 dbnum 初始化（force_rebuild=true 强制重建）
        match scene_tree::init_scene_tree_by_dbno(TEST_DBNO as u32, true).await {
            Ok(result) => {
                println!("✓ 初始化成功:");
                println!("  - 节点数: {}", result.node_count);
                println!("  - 关系数: {}", result.relation_count);
                println!("  - 耗时: {} ms", result.duration_ms);
            }
            Err(e) => {
                println!("✗ 初始化失败: {}", e);
            }
        }
    }

    /// 测试查询 geo_type 分布 (dbnum=1112)
    #[tokio::test]
    async fn test_query_geo_type_distribution() {
        println!("=== 测试查询 geo_type 分布 (dbnum=1112) ===");

        // 先初始化数据库连接
        if let Err(e) = aios_core::init_surreal().await {
            let msg = e.to_string();
            if !msg.contains("Already connected") {
                println!("✗ 数据库连接失败: {}", e);
                return;
            }
        }

        // 查询 geo_type 分布
        let sql = format!(
            "SELECT geo_type, count() as cnt FROM scene_node WHERE dbnum = {} AND has_geo = true GROUP BY geo_type",
            TEST_DBNO
        );

        match project_primary_db().query(&sql).await {
            Ok(mut resp) => {
                let results: Vec<serde_json::Value> = resp.take(0).unwrap_or_default();
                println!("✓ 查询成功，geo_type 分布:");
                for row in results {
                    let geo_type = row
                        .get("geo_type")
                        .and_then(|v| v.as_str())
                        .unwrap_or("None");
                    let cnt = row.get("cnt").and_then(|v| v.as_i64()).unwrap_or(0);
                    println!("  - {}: {}", geo_type, cnt);
                }
            }
            Err(e) => {
                println!("✗ 查询失败: {}", e);
            }
        }
    }

    /// 测试查询正负实体节点数量 (dbnum=1112)
    #[tokio::test]
    async fn test_query_pos_neg_count() {
        println!("=== 测试查询正负实体节点数量 (dbnum=1112) ===");

        // 先初始化数据库连接
        if let Err(e) = aios_core::init_surreal().await {
            let msg = e.to_string();
            if !msg.contains("Already connected") {
                println!("✗ 数据库连接失败: {}", e);
                return;
            }
        }

        // 查询正实体数量
        let sql_pos = format!(
            "SELECT count() FROM scene_node WHERE dbnum = {} AND geo_type = 'Pos' GROUP ALL",
            TEST_DBNO
        );
        // 查询负实体数量
        let sql_neg = format!(
            "SELECT count() FROM scene_node WHERE dbnum = {} AND geo_type IN ['Neg', 'CataNeg', 'CataCrossNeg'] GROUP ALL",
            TEST_DBNO
        );

        let pos_count: i64 = project_primary_db()
            .query_take(&sql_pos, 0)
            .await
            .map(|v: Vec<i64>| v.first().copied().unwrap_or(0))
            .unwrap_or(0);

        let neg_count: i64 = project_primary_db()
            .query_take(&sql_neg, 0)
            .await
            .map(|v: Vec<i64>| v.first().copied().unwrap_or(0))
            .unwrap_or(0);

        println!("✓ 查询结果 (dbnum={}):", TEST_DBNO);
        println!("  - 正实体 (Pos): {}", pos_count);
        println!("  - 负实体 (Neg/CataNeg/CataCrossNeg): {}", neg_count);
    }
}
