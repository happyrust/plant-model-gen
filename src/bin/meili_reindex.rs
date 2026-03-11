use clap::Parser;

/// 占位的 Meilisearch 重建索引入口。
///
/// 当前仓库测试阶段只需要该 bin 存在，避免 Cargo 在解析 `[[bin]]`
/// 清单时因为缺失文件而阻塞其他单元/接口测试。
#[derive(Debug, Parser)]
struct Args {
    /// 预留参数，便于后续补齐真实重建逻辑时保持 CLI 兼容扩展空间。
    #[arg(long)]
    dry_run: bool,
}

fn main() {
    let args = Args::parse();
    let mode = if args.dry_run { "dry-run" } else { "noop" };
    println!("meili_reindex placeholder: {}", mode);
}
