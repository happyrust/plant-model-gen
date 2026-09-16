//! kv-mem 装载性能基准（ADR-0012 切片 2；不接产品 CLI）。
//!
//! 用法：
//!   cargo run --example seed_kvmem_bench -- <tree_dir> <dbnum> [expected_sesno]
//!
//! - `<tree_dir>`：解析产物目录（其下有 `pe_graph/` 种子目录），例如
//!   `output/<project>/scene_tree`；
//! - `<dbnum>`：需已发布 Ready 种子的 dbnum（如 ams7997 的 7997）；
//! - `[expected_sesno]`：可选，缺省 0 表示跳过 sesno 凭据比对。
//!
//! 持久库连接沿用全局 `DbOption`（同其它 CLI，由环境/默认 toml 决定）。

use std::path::PathBuf;
use std::time::Instant;

use aios_database::versioned_db::pe_graph_kvmem;
use anyhow::{Context, Result, anyhow};

#[tokio::main]
async fn main() -> Result<()> {
    let mut args = std::env::args().skip(1);
    let tree_dir = args
        .next()
        .ok_or_else(|| anyhow!("用法: seed_kvmem_bench <tree_dir> <dbnum> [expected_sesno]"))?;
    let dbnum: u32 = args
        .next()
        .ok_or_else(|| anyhow!("缺少 dbnum 参数"))?
        .parse()
        .context("dbnum 需为整数")?;
    let expected_sesno: u32 = match args.next() {
        Some(value) => value.parse().context("expected_sesno 需为整数")?,
        None => 0,
    };
    let tree_dir = PathBuf::from(tree_dir);

    println!("[seed_kvmem_bench] 连接持久库…");
    aios_core::init_surreal().await.context("init_surreal 失败")?;

    println!("[seed_kvmem_bench] 创建 kv-mem 站点…");
    let site = pe_graph_kvmem::create_kvmem_site().await?;

    println!(
        "[seed_kvmem_bench] 装载 dbnum={dbnum} (tree_dir={}, expected_sesno={expected_sesno})…",
        tree_dir.display()
    );
    let total = Instant::now();
    let stats =
        pe_graph_kvmem::load_dbnum_into_kvmem(&site, &tree_dir, dbnum, expected_sesno).await?;
    let total_ms = total.elapsed().as_millis();

    println!("=== kv-mem 装载完成 ===");
    println!("dbnum            = {}", stats.dbnum);
    println!("sesno            = {}", stats.sesno);
    println!("node_count       = {}", stats.node_count);
    println!("edge_count       = {}", stats.edge_count);
    println!("read_validate_ms = {}", stats.read_validate_ms);
    println!("insert_pe_ms     = {}", stats.insert_pe_ms);
    println!("insert_owner_ms  = {}", stats.insert_owner_ms);
    println!("verify_ms        = {}", stats.verify_ms);
    println!("total_ms         = {}", total_ms);
    Ok(())
}
