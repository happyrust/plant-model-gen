//! 进程内运行日志环形缓冲(spec 005-server-runtime-logs)。
//!
//! `TeeLogger` 包装既有全局 logger(env_logger):原样转发控制台输出,同时把 `info`
//! 及以上级别的记录写入有界 ring,供 `/api/logs?type=server.runtime` 查询。
//! 无持久化——定位是"看进程刚才发生了什么",重启即清。

use std::collections::VecDeque;
use std::sync::Mutex;

/// ring 容量(旧记录淘汰)。
const RING_CAPACITY: usize = 5000;
/// 单条 message 截断上限。
const MESSAGE_LIMIT: usize = 2 * 1024;

#[derive(Debug, Clone)]
pub struct RuntimeLogEntry {
    pub ts_ms: i64,
    /// "error" / "warn" / "info"
    pub level: &'static str,
    pub target: String,
    pub message: String,
}

static RUNTIME_LOG_RING: Mutex<VecDeque<RuntimeLogEntry>> = Mutex::new(VecDeque::new());

fn push_entry(entry: RuntimeLogEntry) {
    if let Ok(mut ring) = RUNTIME_LOG_RING.lock() {
        if ring.len() >= RING_CAPACITY {
            ring.pop_front();
        }
        ring.push_back(entry);
    }
}

/// 快照(newest-first 由调用方处理;这里按写入序返回拷贝)。
pub fn snapshot() -> Vec<RuntimeLogEntry> {
    RUNTIME_LOG_RING
        .lock()
        .map(|ring| ring.iter().cloned().collect())
        .unwrap_or_default()
}

struct TeeLogger {
    inner: Box<dyn log::Log>,
}

impl log::Log for TeeLogger {
    fn enabled(&self, metadata: &log::Metadata) -> bool {
        self.inner.enabled(metadata)
    }

    fn log(&self, record: &log::Record) {
        self.inner.log(record);

        let level = match record.level() {
            log::Level::Error => "error",
            log::Level::Warn => "warn",
            log::Level::Info => "info",
            // debug/trace 噪声大,不入 ring(spec 005 Non-Goals)。
            _ => return,
        };
        let mut message = record.args().to_string();
        if message.len() > MESSAGE_LIMIT {
            message = message.chars().take(MESSAGE_LIMIT).collect();
        }
        push_entry(RuntimeLogEntry {
            ts_ms: chrono::Utc::now().timestamp_millis(),
            level,
            target: record.target().to_string(),
            message,
        });
    }

    fn flush(&self) {
        self.inner.flush();
    }
}

/// 以 Tee 方式安装 env_logger(供 web_server bin 启动时调用,替代 `Builder::init()`)。
///
/// 保持原有过滤语义(`RUST_LOG`,默认 debug);最大日志级别同步设置,
/// 确保转发输出与既有行为一致。
pub fn install_tee_env_logger(default_filter: &str) {
    let env_logger = env_logger::Builder::from_env(
        env_logger::Env::default().default_filter_or(default_filter),
    )
    .build();
    let max_level = env_logger.filter();
    if log::set_boxed_logger(Box::new(TeeLogger {
        inner: Box::new(env_logger),
    }))
    .is_ok()
    {
        log::set_max_level(max_level);
    }
}
