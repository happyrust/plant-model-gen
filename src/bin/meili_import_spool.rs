use std::path::{Path, PathBuf};

use aios_core::options::DbOption;
use clap::Parser;

/// 将解析期生成的 JSONL spool 文件导入 Meilisearch。
#[derive(Debug, Parser)]
struct Args {
    /// 配置文件路径（不带 .toml 后缀），例如 db_options/DbOption-ams
    #[arg(long, short = 'c', default_value = "db_options/DbOption")]
    config: String,

    /// 数据库号（dbnum / site），用于推导默认 spool 文件名 `{dbnum}.jsonl`
    #[arg(long)]
    dbnum: i32,

    /// 显式指定 spool 文件路径（默认：{meili_spool_dir}/{dbnum}.jsonl）
    #[arg(long)]
    spool: Option<String>,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let args = Args::parse();

    let s = config::Config::builder()
        .add_source(config::File::with_name(&args.config))
        .build()
        .map_err(|e| anyhow::anyhow!("Failed to build config: {}", e))?;
    let db_option = s
        .try_deserialize::<DbOption>()
        .map_err(|e| anyhow::anyhow!("Failed to deserialize DbOption: {}", e))?;

    let cfg = aios_database::meili::pdms_index::MeiliEnvConfig::from_db_option(&db_option)
        .ok_or_else(|| anyhow::anyhow!("meili_url not configured in config or env"))?;

    let spool_path: PathBuf = match args.spool {
        Some(p) => PathBuf::from(p),
        None => cfg.spool_dir.join(format!("{}.jsonl", args.dbnum)),
    };
    if !spool_path.is_file() {
        anyhow::bail!("spool file not found: {}", spool_path.display());
    }

    let imported =
        aios_database::meili::pdms_index::import_spool_file(&cfg, Path::new(&spool_path)).await?;
    println!(
        "✅ Meilisearch 导入完成: imported={}, index={}, spool={}",
        imported,
        cfg.index,
        spool_path.display()
    );
    Ok(())
}
