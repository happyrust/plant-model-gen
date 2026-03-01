use std::io::{BufWriter, Write};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

/// 线程安全的 SQL 文件写入器，用于模型生成阶段将 SQL 语句写入 .surql 文件而非直接写入 SurrealDB。
pub struct SqlFileWriter {
    inner: Mutex<BufWriter<std::fs::File>>,
    path: PathBuf,
    statement_count: Mutex<usize>,
}

impl SqlFileWriter {
    /// 创建新的 SqlFileWriter，自动创建父目录。
    pub fn new(path: &Path) -> anyhow::Result<Self> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let file = std::fs::File::create(path)?;
        let writer = BufWriter::with_capacity(256 * 1024, file);
        Ok(Self {
            inner: Mutex::new(writer),
            path: path.to_path_buf(),
            statement_count: Mutex::new(0),
        })
    }

    /// 为指定项目和 dbnum 生成默认输出路径。
    ///
    /// 格式：`output/{project}/deferred_sql/{timestamp}_{dbnum}.surql`
    pub fn default_path(project_output_dir: &Path, dbnum: Option<u32>) -> PathBuf {
        let timestamp = chrono::Local::now().format("%Y%m%d_%H%M%S");
        let suffix = dbnum
            .map(|d| format!("{}_{}", timestamp, d))
            .unwrap_or_else(|| format!("{}_all", timestamp));
        project_output_dir
            .join("deferred_sql")
            .join(format!("{}.surql", suffix))
    }

    /// 写入一条 SQL 语句（自动追加换行和分号）。
    pub fn write_statement(&self, sql: &str) -> anyhow::Result<()> {
        let trimmed = sql.trim();
        if trimmed.is_empty() {
            return Ok(());
        }
        let mut writer = self.inner.lock().map_err(|e| anyhow::anyhow!("lock: {}", e))?;
        writer.write_all(trimmed.as_bytes())?;
        if !trimmed.ends_with(';') {
            writer.write_all(b";")?;
        }
        writer.write_all(b"\n")?;
        drop(writer);

        let mut count = self.statement_count.lock().map_err(|e| anyhow::anyhow!("lock: {}", e))?;
        *count += 1;
        Ok(())
    }

    /// 写入多条 SQL 语句。
    pub fn write_statements(&self, sqls: &[String]) -> anyhow::Result<()> {
        let mut writer = self.inner.lock().map_err(|e| anyhow::anyhow!("lock: {}", e))?;
        let mut count = self.statement_count.lock().map_err(|e| anyhow::anyhow!("lock: {}", e))?;
        for sql in sqls {
            let trimmed = sql.trim();
            if trimmed.is_empty() {
                continue;
            }
            writer.write_all(trimmed.as_bytes())?;
            if !trimmed.ends_with(';') {
                writer.write_all(b";")?;
            }
            writer.write_all(b"\n")?;
            *count += 1;
        }
        Ok(())
    }

    /// 写入注释行。
    pub fn write_comment(&self, comment: &str) -> anyhow::Result<()> {
        let mut writer = self.inner.lock().map_err(|e| anyhow::anyhow!("lock: {}", e))?;
        writer.write_all(b"-- ")?;
        writer.write_all(comment.as_bytes())?;
        writer.write_all(b"\n")?;
        Ok(())
    }

    /// 刷新缓冲区到磁盘。
    pub fn flush(&self) -> anyhow::Result<()> {
        let mut writer = self.inner.lock().map_err(|e| anyhow::anyhow!("lock: {}", e))?;
        writer.flush()?;
        Ok(())
    }

    /// 返回文件路径。
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// 返回已写入的语句数。
    pub fn statement_count(&self) -> usize {
        self.statement_count
            .lock()
            .map(|c| *c)
            .unwrap_or(0)
    }
}

/// 从 .surql 文件批量导入 SQL 到 SurrealDB。
///
/// 按 `batch_size` 条语句为一个事务块执行，支持重试。
/// 返回 (成功语句数, 失败语句数)。
pub async fn import_sql_file(
    path: &std::path::Path,
    batch_size: usize,
) -> anyhow::Result<(usize, usize)> {
    use aios_core::model_query_response;
    use std::io::BufRead;

    let file = std::fs::File::open(path)
        .map_err(|e| anyhow::anyhow!("打开 surql 文件失败 {}: {}", path.display(), e))?;
    let reader = std::io::BufReader::new(file);

    let mut statements: Vec<String> = Vec::new();
    for line in reader.lines() {
        let line = line?;
        let trimmed = line.trim();
        // 跳过空行和注释
        if trimmed.is_empty() || trimmed.starts_with("--") {
            continue;
        }
        statements.push(trimmed.to_string());
    }

    let total = statements.len();
    println!(
        "[import_sql] 读取 {} 条 SQL 语句，批次大小={}，预计 {} 批",
        total,
        batch_size,
        (total + batch_size - 1) / batch_size
    );

    let mut success = 0usize;
    let mut failed = 0usize;

    for (batch_idx, chunk) in statements.chunks(batch_size).enumerate() {
        let block = format!(
            "BEGIN TRANSACTION;\n{}\nCOMMIT TRANSACTION;",
            chunk.join("\n")
        );

        let mut retries = 0u32;
        let max_retries = 3u32;
        loop {
            match model_query_response(&block).await {
                Ok(_) => {
                    success += chunk.len();
                    break;
                }
                Err(e) => {
                    retries += 1;
                    if retries > max_retries {
                        eprintln!(
                            "[import_sql] 批次 {} 执行失败（重试 {} 次后放弃）: {}",
                            batch_idx, max_retries, e
                        );
                        failed += chunk.len();
                        break;
                    }
                    let backoff = std::time::Duration::from_millis(100 * retries as u64);
                    eprintln!(
                        "[import_sql] 批次 {} 执行失败，{}ms 后重试 ({}/{}): {}",
                        batch_idx,
                        backoff.as_millis(),
                        retries,
                        max_retries,
                        e
                    );
                    tokio::time::sleep(backoff).await;
                }
            }
        }

        if (batch_idx + 1) % 10 == 0 || batch_idx == (total / batch_size) {
            println!(
                "[import_sql] 进度: {}/{} 条 ({:.1}%)",
                success + failed,
                total,
                (success + failed) as f64 / total as f64 * 100.0
            );
        }
    }

    println!(
        "[import_sql] 导入完成: 成功={}, 失败={}, 总计={}",
        success, failed, total
    );

    Ok((success, failed))
}

impl Drop for SqlFileWriter {
    fn drop(&mut self) {
        if let Ok(mut writer) = self.inner.lock() {
            let _ = writer.flush();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Read;

    #[test]
    fn test_write_and_read() {
        let dir = std::env::temp_dir().join("test_sql_file_writer");
        let _ = std::fs::remove_dir_all(&dir);
        let path = dir.join("test.surql");

        let writer = SqlFileWriter::new(&path).unwrap();
        writer.write_comment("batch 1").unwrap();
        writer
            .write_statement("INSERT IGNORE INTO inst_geo [{id: 1}]")
            .unwrap();
        writer
            .write_statement("INSERT RELATION INTO geo_relate [{in: a, out: b}];")
            .unwrap();
        writer.flush().unwrap();

        assert_eq!(writer.statement_count(), 2);

        let mut content = String::new();
        std::fs::File::open(&path)
            .unwrap()
            .read_to_string(&mut content)
            .unwrap();

        assert!(content.contains("-- batch 1"));
        assert!(content.contains("INSERT IGNORE INTO inst_geo [{id: 1}];"));
        assert!(content.contains("INSERT RELATION INTO geo_relate [{in: a, out: b}];"));

        let _ = std::fs::remove_dir_all(&dir);
    }
}
