use aios_core::tool::math_tool::{dquat_to_pdms_ori_xyz_str, dvec3_to_xyz_str};
use aios_core::{RefnoEnum, get_db_option, init_surreal, query_pe_transform};
use anyhow::Result;

/// # 使用方法
///
/// 1. 刷新所有 MDB (默认行为):
///    ```
///    cargo run --example refresh_pe_transform
///    ```
///
/// 2. 刷新指定 MDB:
///    ```
///    cargo run --example refresh_pe_transform ams1112_0001
///    ```
///
/// 3. 刷新指定 dbnum (单个):
///    ```
///    cargo run --example refresh_pe_transform --dbnum 1112
///    ```
///
/// 4. 刷新多个 dbnum (逗号分隔):
///    ```
///    cargo run --example refresh_pe_transform --dbnum 1112,7999,8000
///    ```
#[tokio::main]
async fn main() -> Result<()> {
    init_surreal().await?;

    let args: Vec<String> = std::env::args().collect();

    // 检查是否有 --dbnum 参数
    if let Some(dbnum_idx) = args.iter().position(|x| x == "--dbnum") {
        if let Some(dbnum_str) = args.get(dbnum_idx + 1) {
            // 解析 dbnum 列表 (支持逗号分隔)
            let dbnums: Vec<u32> = dbnum_str
                .split(',')
                .filter_map(|s| s.trim().parse::<u32>().ok())
                .collect();

            if dbnums.is_empty() {
                eprintln!("❌ 无效的 dbnum 参数: {}", dbnum_str);
                eprintln!("用法: --dbnum 1112  或  --dbnum 1112,7999,8000");
                return Ok(());
            }

            println!("🔄 刷新指定数据库的 pe_transform: {:?}", dbnums);
            let count = aios_core::transform::refresh_pe_transform_for_dbnums(&dbnums).await?;
            println!("✅ 刷新完成，共处理 {} 个节点", count);
        } else {
            eprintln!("❌ --dbnum 参数需要指定数据库编号");
            eprintln!("用法: --dbnum 1112  或  --dbnum 1112,7999,8000");
            return Ok(());
        }
    } else {
        // 原有的 MDB 刷新逻辑
        let mdb_arg = args.get(1).cloned();
        let mdb_name = mdb_arg
            .clone()
            .unwrap_or_else(|| get_db_option().mdb_name.clone());

        println!("🔄 刷新 MDB 的 pe_transform: {}", mdb_name);
        let count = aios_core::transform::refresh_pe_transform_for_mdb(mdb_arg).await?;
        println!("✅ 刷新完成，共处理 {} 个节点", count);
    }

    // 验证示例节点
    let sample_ref = RefnoEnum::from("17496/171145");
    println!("\n📊 验证节点 17496/171145:");
    let Some(cache) = query_pe_transform(sample_ref).await? else {
        println!("   ⚠️  pe_transform 缓存未找到");
        return Ok(());
    };

    let Some(world) = cache.world else {
        println!("   ⚠️  world_trans 未找到");
        return Ok(());
    };

    let ori = dquat_to_pdms_ori_xyz_str(&world.rotation.as_dquat(), true);
    let pos = dvec3_to_xyz_str(world.translation.as_dvec3());

    println!("   ✅ 世界坐标:");
    println!("      Orientation: {}", ori);
    println!("      Position: {}", pos);

    Ok(())
}
