use clap::Parser;
use duckdb::Connection;
use meilisearch_sdk::client::Client;
use serde::Serialize;
use std::path::{Path, PathBuf};

/// 批量构建/重建 PDMS 检索索引（Meilisearch）
///
/// 读取 `output/database_models/{dbnum}/pe.parquet`，抽取 `refno + noun + name + dbnum(site)` 写入 Meilisearch。
///
/// 约定：
/// - Meilisearch index primary key = `refno`
/// - `site` 字段使用 dbnum（满足“按 SITE 分组”的需求；无需依赖 WORL/SITE 是否写入 pe 表）
#[derive(Debug, Parser)]
#[command(author, version, about)]
struct Args {
    /// Meilisearch URL，例如 http://127.0.0.1:7700
    #[arg(long)]
    meili_url: String,

    /// Meilisearch API key（可选）
    #[arg(long)]
    meili_api_key: Option<String>,

    /// 索引名（默认 pdms_nodes）
    #[arg(long, default_value = "pdms_nodes")]
    index: String,

    /// output/database_models 根目录
    #[arg(long, default_value = "output/database_models")]
    base_dir: String,

    /// 指定 dbnum（可多次传入）。不传则扫描 base_dir 下所有数字目录。
    #[arg(long)]
    dbnum: Vec<u32>,

    /// 每批写入 Meilisearch 的文档数
    #[arg(long, default_value_t = 10_000)]
    batch_size: usize,

    /// 从 parquet 拉取的分页大小
    #[arg(long, default_value_t = 50_000)]
    page_size: usize,
}

#[derive(Debug, Clone, Serialize)]
struct PdmsNodeDoc {
    refno: String,
    noun: String,
    name: String,
    site: String,
}

fn discover_dbnums(base_dir: &Path) -> anyhow::Result<Vec<u32>> {
    let mut out: Vec<u32> = Vec::new();
    if !base_dir.exists() {
        return Ok(out);
    }
    for entry in std::fs::read_dir(base_dir)? {
        let entry = entry?;
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        let Some(name) = path.file_name().and_then(|s| s.to_str()) else {
            continue;
        };
        if let Ok(n) = name.parse::<u32>() {
            out.push(n);
        }
    }
    out.sort();
    Ok(out)
}

fn open_pe_view(dbnum: u32, base_dir: &Path) -> anyhow::Result<Connection> {
    let conn = Connection::open_in_memory()?;
    let pe_path = base_dir.join(format!("{dbnum}/pe.parquet"));
    if !pe_path.is_file() {
        anyhow::bail!("pe.parquet not found: {}", pe_path.display());
    }
    let sql = format!(
        "CREATE VIEW pe AS SELECT * FROM read_parquet('{}')",
        pe_path.display()
    );
    conn.execute(&sql, [])?;
    Ok(conn)
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let args = Args::parse();
    let base_dir = PathBuf::from(&args.base_dir);

    let dbnums = if !args.dbnum.is_empty() {
        args.dbnum.clone()
    } else {
        discover_dbnums(&base_dir)?
    };
    if dbnums.is_empty() {
        println!("未发现任何 dbnum，base_dir={}", base_dir.display());
        return Ok(());
    }

    let client = Client::new(args.meili_url, args.meili_api_key)?;
    let index = client.index(args.index.as_str());

    // 基础 settings：可筛选字段 + 可搜索字段
    // 注：settings 更新是异步 task；这里尽力提交一次，不强依赖其完成。
    let _ = index
        .set_filterable_attributes(&["noun", "site"])
        .await;
    let _ = index
        .set_searchable_attributes(&["name", "refno"])
        .await;

    for dbnum in dbnums {
        println!("==> indexing dbnum={dbnum} ...");
        let conn = match open_pe_view(dbnum, &base_dir) {
            Ok(c) => c,
            Err(e) => {
                eprintln!("skip dbnum={dbnum}: {e}");
                continue;
            }
        };

        let total: i64 = conn
            .query_row("SELECT COUNT(*) FROM pe", [], |row| row.get(0))
            .unwrap_or(0);
        if total <= 0 {
            println!("dbnum={dbnum}: empty pe");
            continue;
        }
        println!("dbnum={dbnum}: total rows={total}");

        let mut offset: i64 = 0;
        let mut buffer: Vec<PdmsNodeDoc> = Vec::with_capacity(args.batch_size);

        while offset < total {
            let sql = format!(
                "SELECT refno, noun, name, dbnum FROM pe LIMIT {} OFFSET {}",
                args.page_size, offset
            );
            let mut stmt = conn.prepare(&sql)?;
            let mut rows = stmt.query([])?;
            let mut got = 0usize;

            while let Some(row) = rows.next()? {
                got += 1;
                let refno: String = row.get(0)?;
                let noun: String = row.get(1)?;
                let name: String = row.get(2)?;
                let dbnum_val: i32 = row.get(3)?;
                buffer.push(PdmsNodeDoc {
                    refno,
                    noun: noun.trim().to_uppercase(),
                    name,
                    site: dbnum_val.to_string(),
                });

                if buffer.len() >= args.batch_size {
                    index
                        .add_or_replace(&buffer, Some("refno"))
                        .await?
                        .wait_for_completion(&client, None, None)
                        .await?;
                    buffer.clear();
                }
            }

            if got == 0 {
                break;
            }
            offset += got as i64;
            println!("dbnum={dbnum}: indexed {offset}/{total}");
        }

        if !buffer.is_empty() {
            index
                .add_or_replace(&buffer, Some("refno"))
                .await?
                .wait_for_completion(&client, None, None)
                .await?;
            buffer.clear();
        }

        println!("✅ dbnum={dbnum} done");
    }

    Ok(())
}
