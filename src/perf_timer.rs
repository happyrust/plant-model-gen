//! 轻量级阶段计时器（非 tracing 依赖），始终可用。
//!
//! 用法：
//! ```rust,ignore
//! let mut timer = PerfTimer::new("gen_model");
//! timer.mark("precheck");
//! // ... do work ...
//! timer.mark("visible_refnos");
//! // ... do work ...
//! timer.print_summary();
//!
//! // 可选：保存结构化报告
//! let metadata = serde_json::json!({"mode": "index_tree"});
//! timer.save_json(&path, metadata).ok();
//! ```
//!
//! 同时提供 `profile_span!` 宏在 `feature = "profile"` 下产出 tracing span，
//! 在非 profile 模式下退化为 0 开销。

use std::time::Instant;
use serde::{Serialize, Deserialize};

/// 阶段计时记录
#[derive(Debug, Clone)]
pub struct StageRecord {
    pub name: String,
    pub started_at: Instant,
    pub ended_at: Option<Instant>,
}

/// 轻量阶段计时器（始终编译；不依赖 tracing feature）
pub struct PerfTimer {
    label: String,
    started: Instant,
    stages: Vec<StageRecord>,
}

impl PerfTimer {
    pub fn new(label: &str) -> Self {
        Self {
            label: label.to_string(),
            started: Instant::now(),
            stages: Vec::new(),
        }
    }

    /// 标记一个新阶段的开始（同时结束前一个阶段）
    pub fn mark(&mut self, stage_name: &str) {
        let now = Instant::now();
        // 结束前一个阶段
        if let Some(last) = self.stages.last_mut() {
            if last.ended_at.is_none() {
                last.ended_at = Some(now);
            }
        }
        self.stages.push(StageRecord {
            name: stage_name.to_string(),
            started_at: now,
            ended_at: None,
        });
    }

    /// 结束当前阶段（不开启新阶段）
    pub fn end_current(&mut self) {
        let now = Instant::now();
        if let Some(last) = self.stages.last_mut() {
            if last.ended_at.is_none() {
                last.ended_at = Some(now);
            }
        }
    }

    /// 总耗时
    pub fn total_ms(&self) -> u128 {
        self.started.elapsed().as_millis()
    }

    /// 输出摘要
    pub fn print_summary(&mut self) {
        self.end_current();
        let total = self.started.elapsed();
        println!("\n[perf] ============ {} 阶段耗时摘要 ============", self.label);
        for stage in &self.stages {
            let dur = stage
                .ended_at
                .unwrap_or_else(Instant::now)
                .duration_since(stage.started_at);
            let pct = if total.as_micros() > 0 {
                (dur.as_micros() as f64 / total.as_micros() as f64) * 100.0
            } else {
                0.0
            };
            println!(
                "[perf]   {:<36} {:>8.1} ms  ({:>5.1}%)",
                stage.name,
                dur.as_secs_f64() * 1000.0,
                pct
            );
        }
        println!(
            "[perf]   {:<36} {:>8.1} ms  (100.0%)",
            "TOTAL",
            total.as_secs_f64() * 1000.0
        );
        println!("[perf] ================================================\n");
    }

    /// 获取所有阶段用于自动化分析
    pub fn stages(&self) -> &[StageRecord] {
        &self.stages
    }

    /// 生成结构化性能报告
    pub fn generate_report(&mut self, metadata: serde_json::Value) -> PerfReport {
        self.end_current();
        let total = self.started.elapsed();
        let total_ms = total.as_millis();

        let stages: Vec<StageSummary> = self.stages.iter().map(|stage| {
            let dur = stage.ended_at
                .unwrap_or_else(Instant::now)
                .duration_since(stage.started_at);
            let pct = if total.as_micros() > 0 {
                (dur.as_micros() as f64 / total.as_micros() as f64) * 100.0
            } else {
                0.0
            };
            StageSummary {
                name: stage.name.clone(),
                duration_ms: dur.as_secs_f64() * 1000.0,
                percentage: pct,
                started_at_offset_ms: stage.started_at.duration_since(self.started).as_millis(),
            }
        }).collect();

        PerfReport {
            label: self.label.clone(),
            total_ms,
            stages,
            timestamp: chrono::Utc::now().to_rfc3339(),
            metadata,
        }
    }

    /// 保存性能报告为 JSON 文件
    pub fn save_json(&mut self, output_path: &std::path::Path, metadata: serde_json::Value) -> std::io::Result<()> {
        let report = self.generate_report(metadata);
        let json = serde_json::to_string_pretty(&report)?;
        if let Some(parent) = output_path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(output_path, json)?;
        println!("[perf] 性能报告已保存: {}", output_path.display());
        Ok(())
    }

    /// 保存性能报告为 CSV 文件
    pub fn save_csv(&mut self, output_path: &std::path::Path, metadata: serde_json::Value) -> std::io::Result<()> {
        use std::io::Write;
        let report = self.generate_report(metadata);

        if let Some(parent) = output_path.parent() {
            std::fs::create_dir_all(parent)?;
        }

        let mut file = std::fs::File::create(output_path)?;

        writeln!(file, "Stage,Duration(ms),Percentage(%),Offset(ms)")?;
        for stage in &report.stages {
            writeln!(
                file,
                "{},{:.2},{:.2},{}",
                stage.name, stage.duration_ms, stage.percentage, stage.started_at_offset_ms
            )?;
        }
        writeln!(file, "TOTAL,{:.2},100.0,0", report.total_ms)?;

        println!("[perf] 性能报告已保存: {}", output_path.display());
        Ok(())
    }
}

/// 阶段摘要（用于结构化输出）
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StageSummary {
    pub name: String,
    pub duration_ms: f64,
    pub percentage: f64,
    pub started_at_offset_ms: u128,
}

/// 性能报告（用于 JSON/CSV 导出）
#[derive(Debug, Serialize, Deserialize)]
pub struct PerfReport {
    pub label: String,
    pub total_ms: u128,
    pub timestamp: String,
    pub metadata: serde_json::Value,
    pub stages: Vec<StageSummary>,
}

/// 便捷宏：在 `feature = "profile"` 时产出 tracing::info_span，否则退化为 no-op。
///
/// 用法：
/// ```rust,ignore
/// profile_span!("stage_name");
/// profile_span!("stage_name", field1 = val1, field2 = val2);
/// ```
///
/// 返回的 guard 必须绑定到 `let _guard = ...` 以保持 span 存活。
#[macro_export]
macro_rules! profile_span {
    ($name:expr) => {{
        #[cfg(feature = "profile")]
        let _guard = tracing::info_span!($name).entered();
        #[cfg(not(feature = "profile"))]
        let _guard = ();
        _guard
    }};
    ($name:expr, $($field:tt)*) => {{
        #[cfg(feature = "profile")]
        let _guard = tracing::info_span!($name, $($field)*).entered();
        #[cfg(not(feature = "profile"))]
        let _guard = ();
        _guard
    }};
}
