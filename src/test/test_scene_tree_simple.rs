/// 测试 Scene Tree 模块（简化版）
///
/// 测试目标：
/// - 使用 dbnum 1112
/// - 测试场景树初始化
/// - 测试查询功能
///
/// 运行方式：
/// ```bash
/// cargo test --lib --features gen_model test_scene_tree_simple -- --nocapture
/// ```

#[cfg(test)]
#[cfg(all(feature = "gen_model", not(target_arch = "wasm32")))]
mod tests {
    use aios_core::{RefnoEnum, RefU64, SUL_DB, SurrealQueryExt};
    use crate::scene_tree;

    const TEST_DBNO: i32 = 1112;

    /// 测试场景树 Schema 初始化
    #[tokio::test]
    async fn test_scene_tree_schema_simple() {
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
    async fn test_generation_status_simple() {
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
                    println!("  - {}: has_geo={}, generated={}",
                        refno, status.has_geo, status.generated);
                }
            }
            Err(e) => {
                println!("✗ 查询失败: {}", e);
            }
        }
    }

    /// 测试标记为已生成
    #[tokio::test]
    async fn test_mark_generated_simple() {
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
}
