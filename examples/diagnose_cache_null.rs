//! 诊断 cache 反序列化失败 "invalid type: null, expected f32" 的根因。
//! 从 foyer cache 中读取指定 batch 的原始 JSON payload，
//! 找出所有 `:null` 出现的位置及其前后上下文。
//!
//! 用法: cargo run --example diagnose_cache_null --features sqlite-index

use std::path::PathBuf;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // 目标 batch
    let failing_batches = vec![
        (7997u32, "7997_1105"),
        (7997, "7997_1109"),
        (7997, "7997_1116"),
        (7997, "7997_1119"),
    ];

    let cache_dir = PathBuf::from("output/AvevaMarineSample/instance_cache");
    println!("📂 cache_dir: {}", cache_dir.display());

    let cache = aios_database::fast_model::instance_cache::InstanceCacheManager::new(&cache_dir).await?;

    // 列出 dbnum=7997 的所有 batch
    let batches = cache.list_batches(7997);
    println!("📋 dbnum=7997 共 {} 个 batch: {:?}", batches.len(), &batches[..batches.len().min(10)]);

    for (dbnum, batch_id) in &failing_batches {
        println!("\n{'='*60}");
        println!("🔍 检查 batch: dbnum={}, batch_id={}", dbnum, batch_id);

        // 直接读取原始 payload（绕过反序列化）
        let key = aios_database::fast_model::instance_cache::InstanceCacheKey {
            dbnum: *dbnum,
            batch_id: batch_id.to_string(),
        };

        // 我们需要访问 cache 的底层 API 来获取原始 payload
        // 但 InstanceCacheManager 封装了这一层，只暴露了 get() 方法
        // get() 内部会尝试反序列化，失败就返回 None
        // 所以我们需要用另一种方式：直接调用 foyer cache 的底层

        // 方案：在 get() 返回 None 时，我们知道反序列化失败了
        // 让我们用 serde_json::Value 来做宽松反序列化
        match cache.get(*dbnum, batch_id).await {
            Some(_batch) => {
                println!("✅ batch 反序列化成功！（可能已被修复）");
            }
            None => {
                println!("❌ batch 反序列化失败（如预期）");
                println!("   需要从底层获取原始 payload 来分析...");
            }
        }
    }

    // 分析方案2：直接从 foyer 磁盘文件读取
    // foyer 使用二进制格式，不容易直接读取
    // 更好的方案：修改 instance_cache.rs 的 get() 方法，在反序列化失败时 dump 原始 JSON

    println!("\n\n📝 建议：在 instance_cache.rs 的 get() 方法中，反序列化失败时 dump 原始 payload 到文件");
    println!("   然后用 serde_json::Value 做宽松反序列化来定位 null 字段");

    Ok(())
}
