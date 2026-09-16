//! 诊断：导出 instances.json 的最小示例（占位）。
//!
//! 说明：
//! - Cargo.toml 里声明了该 example；历史上可能用于排查 instances 导出/缓存内容。
//! - 目前仓库里缺少对应源码会导致 `cargo test`/`cargo build --examples` 失败。
//! - 先提供一个最小可编译的入口，避免构建被阻断；后续可按需要补充真实诊断逻辑。

fn main() {
    println!("diagnose_export_instances: placeholder (no-op)");
}
