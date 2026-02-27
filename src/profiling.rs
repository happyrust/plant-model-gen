//! 性能剖析（Chrome Trace）工具。
//!
//! 设计目标：
//! - KISS：只做最小的 tracing-chrome 初始化与一次性输出。
//! - 全局仅初始化一次，避免与其它 tracing_subscriber 初始化冲突。
//! - 产物落在 output/profile 下，文件名带 dbnum 与时间戳，便于多次对比。

use std::path::{Path, PathBuf};

/// 初始化 Chrome Trace（feature=profile 时生效）。
///
/// 注意：
/// - tracing_subscriber 只能 init 一次；若已初始化则直接返回 Ok。
/// - 为了避免 guard 被提前 drop，这里将其保存在全局静态变量中，直到进程结束。
#[cfg(feature = "profile")]
pub fn init_chrome_tracing(trace_path: impl AsRef<Path>) -> anyhow::Result<()> {
    use std::sync::atomic::{AtomicBool, Ordering};
    use tracing_chrome::{ChromeLayerBuilder, FlushGuard};
    use tracing_subscriber::prelude::*;

    static TRACING_INITIALIZED: AtomicBool = AtomicBool::new(false);
    static mut TRACING_GUARD: Option<FlushGuard> = None;

    // 全局 dispatcher 已经 set 过（例如其它服务模式/测试），则不再重复初始化。
    if tracing::dispatcher::has_been_set() {
        return Ok(());
    }

    // 并发场景下确保只初始化一次。
    if TRACING_INITIALIZED
        .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
        .is_err()
    {
        return Ok(());
    }

    let trace_path = trace_path.as_ref();
    if let Some(parent) = trace_path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    if trace_path.exists() {
        let _ = std::fs::remove_file(trace_path);
    }

    let (chrome_layer, guard) = ChromeLayerBuilder::new()
        .file(trace_path)
        // 关闭 args/locations，减少 trace 文件体积与 JSON 风险。
        .include_args(false)
        .include_locations(false)
        .build();

    unsafe {
        TRACING_GUARD = Some(guard);
    }

    // 使用 try_init 避免与已有的 log crate logger (如 simplelog) 冲突
    let _ = tracing_subscriber::registry().with(chrome_layer).try_init();

    println!(
        "[profile] Chrome tracing 已启用，输出: {}",
        trace_path.display()
    );
    Ok(())
}

/// feature=profile 未开启时的空实现，避免调用方加 cfg。
#[cfg(not(feature = "profile"))]
pub fn init_chrome_tracing(_trace_path: impl AsRef<Path>) -> anyhow::Result<()> {
    Ok(())
}

/// 根据 DbOptionExt 生成 trace 文件路径，并初始化 Chrome Trace。
///
/// - stage：用于区分不同流水线（例如 gen_model / full_flow / room_compute）
pub fn init_chrome_tracing_for_db_option(
    db_option: &crate::options::DbOptionExt,
    stage: &str,
) -> anyhow::Result<PathBuf> {
    let dbnum_tag = db_option
        .manual_db_nums
        .as_ref()
        .and_then(|v| {
            if v.len() == 1 {
                Some(format!("{}", v[0]))
            } else if v.is_empty() {
                None
            } else {
                Some(format!("multi_{}", v.len()))
            }
        })
        .unwrap_or_else(|| "all".to_string());

    let ts = chrono::Local::now().format("%Y%m%d_%H%M%S");
    let out = PathBuf::from("output")
        .join("profile")
        .join(format!(
            "chrome_trace_{}_dbnum_{}_{}.json",
            stage, dbnum_tag, ts
        ));

    init_chrome_tracing(&out)?;
    Ok(out)
}

