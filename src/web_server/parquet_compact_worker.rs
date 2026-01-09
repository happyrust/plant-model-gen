//! Parquet Compact Worker
//!
//! 后台任务，周期性扫描增量 Parquet 文件并合并为主文件。

use std::time::Duration;
use tokio::time::interval;

use crate::fast_model::export_model::parquet_writer::ParquetManager;

/// Worker 配置
#[derive(Clone)]
pub struct CompactWorkerConfig {
    /// 扫描间隔（秒）
    pub scan_interval_secs: u64,
    /// 增量文件触发阈值（超过此数量才触发合并）
    /// 设为 1 表示只要有增量文件就合并
    pub min_incremental_count: usize,
    /// 输出目录（ParquetManager 的 base_dir）
    pub output_dir: String,
}

impl Default for CompactWorkerConfig {
    fn default() -> Self {
        Self {
            scan_interval_secs: 30,
            min_incremental_count: 50,
            output_dir: "output".to_string(),
        }
    }
}

/// 启动后台 compact worker
///
/// 返回 JoinHandle，可用于取消任务
pub fn start_compact_worker(config: CompactWorkerConfig) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        compact_worker_loop(config).await;
    })
}

/// Compact worker 主循环
async fn compact_worker_loop(config: CompactWorkerConfig) {
    let mut ticker = interval(Duration::from_secs(config.scan_interval_secs));

    // 第一次立即执行
    ticker.tick().await;

    loop {
        ticker.tick().await;

        // 扫描并合并
        if let Err(e) = scan_and_compact(&config).await {
            eprintln!("[CompactWorker] 扫描合并失败: {}", e);
        }
    }
}

/// 扫描所有 dbno 并执行合并
async fn scan_and_compact(config: &CompactWorkerConfig) -> anyhow::Result<()> {
    let manager = ParquetManager::new(&config.output_dir);

    // 扫描所有存在增量文件的 dbno
    let dbnos = manager.scan_dbnos_with_incremental()?;

    if dbnos.is_empty() {
        return Ok(());
    }

    println!(
        "[CompactWorker] 检测到 {} 个 dbno 需要合并: {:?}",
        dbnos.len(),
        dbnos
    );

    // 逐个合并
    for dbno in dbnos {
        // 使用 spawn_blocking 在独立线程中执行 IO 密集操作
        let output_dir = config.output_dir.clone();
        let min_count = config.min_incremental_count;

        let result = tokio::task::spawn_blocking(move || {
            let manager = ParquetManager::new(&output_dir);

            // 检查增量文件数量是否达到阈值
            match manager.list_parquet_files(dbno, Some("instances")) {
                Ok(files) => {
                    // 计算增量文件数量（排除主文件）
                    let main_file_name = "instances.parquet".to_string();
                    let incremental_count = files
                        .iter()
                        .filter(|f| *f != &main_file_name)
                        .count();

                    if incremental_count < min_count {
                        return Ok(());
                    }

                    // 执行合并
                    match manager.compact(dbno) {
                        Ok(Some((inst_path, trans_path))) => {
                            // 合并成功，已在 compact 函数中打印日志
                        }
                        Ok(None) => {
                            // 没有需要合并的文件
                        }
                        Err(e) => {
                            eprintln!("[CompactWorker] 合并 dbno={} 失败: {}", dbno, e);
                        }
                    }
                }
                Err(e) => {
                    eprintln!("[CompactWorker] 获取 dbno={} 文件列表失败: {}", dbno, e);
                }
            }

            Ok::<(), anyhow::Error>(())
        })
        .await;

        if let Err(e) = result {
            eprintln!("[CompactWorker] spawn_blocking 失败: {}", e);
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_config_default() {
        let config = CompactWorkerConfig::default();
        assert_eq!(config.scan_interval_secs, 30);
        assert_eq!(config.min_incremental_count, 1);
        assert_eq!(config.output_dir, "output");
    }
}
