//! 对比“可见几何子孙节点”查询结果（用于调试 TreeIndex 路径迁移）
//!
//! 用法：
//! - `cargo run --example compare_visible_geo_descendants -- 17496_171564`
//! - `cargo run --example compare_visible_geo_descendants -- 17496/171564`

use aios_core::{RefnoEnum, init_surreal};
use std::str::FromStr;

fn parse_refno(s: &str) -> anyhow::Result<RefnoEnum> {
    // 兼容 `17496_171564` 与 `17496/171564`
    let s = s.trim();
    let s = if s.contains('_') && !s.contains('/') {
        s.replace('_', "/")
    } else {
        s.to_string()
    };
    RefnoEnum::from_str(&s).map_err(|e| anyhow::anyhow!("parse refno failed: {:?}", e))
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let arg = std::env::args().nth(1).unwrap_or_default();
    if arg.is_empty() {
        eprintln!("Usage: compare_visible_geo_descendants <refno>");
        return Ok(());
    }

    // 需要 dbnum 解析（可能会触发 get_pe），因此先初始化 SurrealDB 连接
    init_surreal().await?;

    let refno = parse_refno(&arg)?;
    let descendants = aios_database::fast_model::query_compat::query_visible_geo_descendants(
        refno,
        true,
        Some(".."),
    )
    .await?;

    println!("refno={}: descendants={}", refno, descendants.len());
    Ok(())
}
