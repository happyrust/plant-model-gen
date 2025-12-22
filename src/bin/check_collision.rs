//! 碰撞检测 CLI 工具
//!
//! 用法:
//! ```bash
//! cargo run --bin check_collision --features duckdb-export -- --help
//! cargo run --bin check_collision --features duckdb-export -- --limit 1000
//! cargo run --bin check_collision --features duckdb-export -- --noun PIPE
//! ```

use aios_database::fast_model::{CollisionConfig, CollisionDetector};
use clap::Parser;
use std::path::PathBuf;

#[derive(Parser, Debug)]
#[command(name = "check_collision")]
#[command(about = "基于 DuckDB + Parry3D 的碰撞检测工具")]
struct Args {
    /// 网格目录
    #[arg(short, long)]
    mesh_dir: Option<PathBuf>,

    /// 碰撞检测容差 (米)
    #[arg(short, long, default_value = "0.001")]
    tolerance: f32,

    /// 并发任务数
    #[arg(short, long, default_value = "8")]
    concurrency: usize,

    /// 限制候选对数量
    #[arg(short, long)]
    limit: Option<usize>,

    /// 按类型过滤 (如 PIPE, EQUI 等)
    #[arg(short, long)]
    noun: Option<String>,

    /// 输出 JSON 格式
    #[arg(long)]
    json: bool,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // 初始化日志
    tracing_subscriber::fmt::init();

    let args = Args::parse();

    // 构建配置
    let mut config = CollisionConfig::default();
    config.tolerance = args.tolerance;
    config.concurrency = args.concurrency;
    config.limit = args.limit;

    if let Some(mesh_dir) = args.mesh_dir {
        config.mesh_dir = mesh_dir;
    }

    println!("=== 碰撞检测工具 (DuckDB) ===");
    println!("数据源: assets/web_duckdb/latest.json");
    println!("网格目录: {:?}", config.mesh_dir);
    println!("容差: {}m", config.tolerance);
    println!("并发: {}", config.concurrency);
    if let Some(limit) = config.limit {
        println!("限制: {}", limit);
    }
    if let Some(ref noun) = args.noun {
        println!("类型过滤: {}", noun);
    }
    println!();

    // 创建检测器
    let detector = CollisionDetector::new(config)?;

    // 执行检测
    let (events, stats) = detector.detect_all(args.noun.as_deref()).await?;

    // 输出结果
    if args.json {
        let output = serde_json::json!({
            "stats": stats,
            "events": events,
        });
        println!("{}", serde_json::to_string_pretty(&output)?);
    } else {
        println!("=== 检测结果 ===");
        println!("候选对数: {}", stats.candidate_pairs);
        println!("碰撞事件: {}", stats.collision_events);
        println!("粗筛耗时: {}ms", stats.broad_phase_ms);
        println!("精算耗时: {}ms", stats.narrow_phase_ms);
        println!("总耗时: {}ms", stats.total_ms);
        println!();

        if !events.is_empty() {
            println!("碰撞列表 (前 20 个):");
            for (i, event) in events.iter().take(20).enumerate() {
                println!(
                    "  {}. {} <-> {} | 穿透: {:.4}m",
                    i + 1,
                    event.pair.0 .0,
                    event.pair.1 .0,
                    event.penetration_depth,
                );
            }
            if events.len() > 20 {
                println!("  ... 还有 {} 个碰撞事件", events.len() - 20);
            }
        }
    }

    Ok(())
}
