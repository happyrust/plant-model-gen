use aios_core::options::DbOption;
use anyhow::Context;
use log::warn;
use meilisearch_sdk::client::Client;
use serde::{Deserialize, Serialize};
use std::fs::{self, File};
use std::io::{BufRead, BufReader, BufWriter, Write};
use std::path::{Path, PathBuf};

/// 解析阶段写入的 PDMS 检索文档（用于 Meilisearch）。
///
/// 主键：id = "{dbnum}_{refno}"（解决跨 dbnum refno 覆盖问题）
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PdmsNodeDoc {
    pub id: String,
    pub refno: String,
    pub noun: String,
    pub name: String,
    pub site: String,
}

#[derive(Debug, Clone)]
pub struct MeiliEnvConfig {
    pub url: String,
    pub api_key: Option<String>,
    pub index: String,
    pub spool_dir: PathBuf,
    pub batch_size: usize,
}

impl MeiliEnvConfig {
    pub fn from_db_option(opt: &DbOption) -> Option<Self> {
        let url = opt
            .meili_url
            .as_ref()
            .map(|v| v.trim().to_string())
            .filter(|v| !v.is_empty())
            .or_else(|| {
                std::env::var("MEILI_URL")
                    .ok()
                    .map(|v| v.trim().to_string())
                    .filter(|v| !v.is_empty())
            })?;

        let api_key = opt
            .meili_api_key
            .as_ref()
            .map(|v| v.trim().to_string())
            .filter(|v| !v.is_empty())
            .or_else(|| {
                std::env::var("MEILI_API_KEY")
                    .ok()
                    .map(|v| v.trim().to_string())
                    .filter(|v| !v.is_empty())
            });

        let index = opt
            .meili_pdms_index
            .as_ref()
            .map(|v| v.trim().to_string())
            .filter(|v| !v.is_empty())
            .or_else(|| {
                std::env::var("MEILI_PDMS_INDEX")
                    .ok()
                    .map(|v| v.trim().to_string())
                    .filter(|v| !v.is_empty())
            })
            .unwrap_or_else(|| "pdms_nodes".to_string());

        let spool_dir = opt
            .meili_spool_dir
            .as_ref()
            .map(|v| v.trim().to_string())
            .filter(|v| !v.is_empty())
            .or_else(|| {
                std::env::var("MEILI_SPOOL_DIR")
                    .ok()
                    .map(|v| v.trim().to_string())
                    .filter(|v| !v.is_empty())
            })
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("output/meili_spool"));
        let batch_size = std::env::var("MEILI_BATCH_SIZE")
            .ok()
            .and_then(|v| v.trim().parse::<usize>().ok())
            .unwrap_or(10_000);
        Some(Self {
            url,
            api_key,
            index,
            spool_dir,
            batch_size,
        })
    }
}

pub struct PdmsSpoolWriter {
    path: PathBuf,
    w: BufWriter<File>,
    pub written: usize,
}

impl PdmsSpoolWriter {
    pub fn create(spool_dir: &Path, dbnum: i32) -> anyhow::Result<Self> {
        fs::create_dir_all(spool_dir)
            .with_context(|| format!("create meili spool dir failed: {}", spool_dir.display()))?;
        let path = spool_dir.join(format!("{dbnum}.jsonl"));
        let f = File::create(&path)
            .with_context(|| format!("create spool file failed: {}", path.display()))?;
        Ok(Self {
            path,
            w: BufWriter::new(f),
            written: 0,
        })
    }

    pub fn write_doc(&mut self, doc: &PdmsNodeDoc) -> anyhow::Result<()> {
        let line = serde_json::to_string(doc).context("serialize meili doc failed")?;
        self.w.write_all(line.as_bytes())?;
        self.w.write_all(b"\n")?;
        self.written += 1;
        Ok(())
    }

    pub fn flush(&mut self) -> anyhow::Result<()> {
        self.w.flush()?;
        Ok(())
    }

    pub fn path(&self) -> &Path {
        &self.path
    }
}

pub async fn import_spool_file(cfg: &MeiliEnvConfig, spool_path: &Path) -> anyhow::Result<usize> {
    if !spool_path.is_file() {
        anyhow::bail!("spool file not found: {}", spool_path.display());
    }

    let client = Client::new(cfg.url.as_str(), cfg.api_key.clone())?;
    let index = client.index(cfg.index.as_str());

    // settings 更新是异步 task；这里尽力提交一次，不强依赖其完成。
    let _ = index.set_filterable_attributes(&["noun", "site"]).await;
    let _ = index.set_searchable_attributes(&["name", "refno"]).await;

    let f = File::open(spool_path)
        .with_context(|| format!("open spool file failed: {}", spool_path.display()))?;
    let reader = BufReader::new(f);

    let mut buf: Vec<PdmsNodeDoc> = Vec::with_capacity(cfg.batch_size);
    let mut total = 0usize;

    for line in reader.lines() {
        let line = line?;
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let doc: PdmsNodeDoc = match serde_json::from_str(line) {
            Ok(v) => v,
            Err(e) => {
                // 允许 spool 文件尾部存在“半行”（例如进程被强杀导致最后一行不完整）。
                // 这类错误不应阻塞整批导入；直接跳过即可。
                warn!(
                    "skip invalid jsonl line(spool={}): {}",
                    spool_path.display(),
                    e
                );
                continue;
            }
        };
        buf.push(doc);
        if buf.len() >= cfg.batch_size {
            index
                .add_or_replace(&buf, Some("id"))
                .await?
                .wait_for_completion(&client, None, None)
                .await?;
            total += buf.len();
            buf.clear();
        }
    }

    if !buf.is_empty() {
        index
            .add_or_replace(&buf, Some("id"))
            .await?
            .wait_for_completion(&client, None, None)
            .await?;
        total += buf.len();
        buf.clear();
    }

    Ok(total)
}
