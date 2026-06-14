//! 站点部署任务级性能指标采集（spec 004-site-deploy-perf-stats）。
//!
//! sidecar（aios-database CLI）侧的指标聚合器：web_server 派发 parse/generate
//! 作业时注入 `AIOS_TASK_METRICS_PATH`（产物路径，文件名即 task_id）与可选的
//! `AIOS_TASK_METRICS_KIND`；CLI 各阶段（闭包 / 解析 / 生成 / 导出）结束时调用
//! `record_*` 聚合，每次记录都会原子落盘（tmp+rename），进程意外退出也能保留
//! 已完成阶段的指标。env 未设置时全部入口为 no-op，不影响普通 CLI 使用。

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::Instant;

use once_cell::sync::OnceCell;
use serde::{Deserialize, Serialize};

/// web_server 注入的产物路径 env（文件名 stem 即 task_id）。
pub const TASK_METRICS_PATH_ENV: &str = "AIOS_TASK_METRICS_PATH";
/// web_server 注入的任务类型 env（parse | generate）；缺省按已记录阶段推断。
pub const TASK_METRICS_KIND_ENV: &str = "AIOS_TASK_METRICS_KIND";
/// 产物 schema 版本（web_server 入库前校验）。
pub const TASK_METRICS_SCHEMA_VERSION: u32 = 1;

// ─── 产物结构（schema_version = 1） ─────────────────────────────────────────

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct CoveredDbnumMetrics {
    pub dbnum: u32,
    /// manifest 覆盖的 refno 数。
    pub refnos: usize,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ClosureStageMetrics {
    pub seed_count: usize,
    pub visited_count: usize,
    pub rounds: usize,
    pub missing_count: usize,
    pub covered_dbnums: Vec<CoveredDbnumMetrics>,
    pub duration_ms: u64,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ParseDbMetrics {
    pub dbnum: u32,
    pub db_type: String,
    /// 实际解析（落库口径）的元素数。
    pub elements: usize,
    /// 文件内全量元素数（partial/skipped 时用于计算裁剪率）。
    pub total_in_file: usize,
    /// full | partial | skipped
    pub mode: String,
    pub duration_ms: u64,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ParseStageMetrics {
    pub dbs: Vec<ParseDbMetrics>,
    pub total_elements: usize,
    /// failed_sql 转储计数（写入失败诊断）。
    pub error_count: usize,
    pub duration_ms: u64,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct GenerateStageMetrics {
    pub inst_relate: usize,
    pub inst_info: usize,
    pub inst_relate_aabb: usize,
    pub mesh_generated: usize,
    pub mesh_cache_hit: usize,
    pub boolean_success: usize,
    pub boolean_failed: usize,
    pub tubi_count: usize,
    /// PerfTimer 分段耗时（阶段名 -> ms）。
    pub stage_ms: BTreeMap<String, u64>,
    pub error_count: usize,
    pub cache_miss: usize,
    pub duration_ms: u64,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ExportStageMetrics {
    pub parquet_files: usize,
    pub parquet_bytes: u64,
    pub json_files: usize,
    pub json_bytes: u64,
    pub duration_ms: u64,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct TaskStagesMetrics {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub closure: Option<ClosureStageMetrics>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parse: Option<ParseStageMetrics>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub generate: Option<GenerateStageMetrics>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub export: Option<ExportStageMetrics>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskMetricsFile {
    pub schema_version: u32,
    pub task_id: String,
    pub job_kind: String,
    pub started_at: String,
    pub finished_at: Option<String>,
    pub duration_ms: u64,
    pub success: Option<bool>,
    pub stages: TaskStagesMetrics,
}

// ─── 采集器 ─────────────────────────────────────────────────────────────────

struct CollectorInner {
    stages: TaskStagesMetrics,
    /// 解析阶段：apply_sync_filter 先记 mode/total，文件循环尾再补 elements/耗时。
    parse_db_notes: BTreeMap<u32, (String, usize)>,
    finished: bool,
    success: Option<bool>,
}

pub struct TaskMetricsCollector {
    path: PathBuf,
    task_id: String,
    kind_override: Option<String>,
    started: Instant,
    started_at: String,
    inner: Mutex<CollectorInner>,
}

static COLLECTOR: OnceCell<Option<TaskMetricsCollector>> = OnceCell::new();

/// 从 env 初始化全局采集器；未设置 `AIOS_TASK_METRICS_PATH` 时为 None（全程 no-op）。
pub fn init_task_metrics_from_env() {
    let _ = COLLECTOR.get_or_init(|| {
        let path = std::env::var(TASK_METRICS_PATH_ENV).ok()?;
        let path = path.trim();
        if path.is_empty() {
            return None;
        }
        let path = PathBuf::from(path);
        let task_id = path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("unknown-task")
            .to_string();
        let kind_override = std::env::var(TASK_METRICS_KIND_ENV)
            .ok()
            .map(|v| v.trim().to_string())
            .filter(|v| !v.is_empty());
        println!(
            "[task_metrics] 指标采集已启用: task_id={} path={}",
            task_id,
            path.display()
        );
        // 同一任务的闭包 job 与解析 job 是两个 CLI 进程、共用一个产物路径：
        // 启动时合并已有产物的 stages，后写进程不丢前一阶段数据。
        let (stages, started_at) = std::fs::read_to_string(&path)
            .ok()
            .and_then(|content| serde_json::from_str::<TaskMetricsFile>(&content).ok())
            .filter(|file| file.schema_version == TASK_METRICS_SCHEMA_VERSION)
            .map(|file| (file.stages, file.started_at))
            .unwrap_or_else(|| {
                (
                    TaskStagesMetrics::default(),
                    chrono::Local::now().to_rfc3339(),
                )
            });
        Some(TaskMetricsCollector {
            path,
            task_id,
            kind_override,
            started: Instant::now(),
            started_at,
            inner: Mutex::new(CollectorInner {
                stages,
                parse_db_notes: BTreeMap::new(),
                finished: false,
                success: None,
            }),
        })
    });
}

fn with_collector<R>(f: impl FnOnce(&TaskMetricsCollector) -> R) -> Option<R> {
    COLLECTOR.get().and_then(|c| c.as_ref()).map(f)
}

impl TaskMetricsCollector {
    fn infer_kind(&self, inner: &CollectorInner) -> String {
        if let Some(kind) = &self.kind_override {
            return kind.clone();
        }
        if inner.stages.generate.is_some() {
            "generate".to_string()
        } else {
            "parse".to_string()
        }
    }

    /// 原子落盘当前快照（每次记录后调用，保证中断也有部分产物）。
    fn flush(&self, inner: &CollectorInner) {
        let file = TaskMetricsFile {
            schema_version: TASK_METRICS_SCHEMA_VERSION,
            task_id: self.task_id.clone(),
            job_kind: self.infer_kind(inner),
            started_at: self.started_at.clone(),
            finished_at: inner.finished.then(|| chrono::Local::now().to_rfc3339()),
            duration_ms: self.started.elapsed().as_millis() as u64,
            success: inner.success,
            stages: inner.stages.clone(),
        };
        if let Err(e) = write_json_atomic(&self.path, &file) {
            eprintln!(
                "[task_metrics] 指标落盘失败({}): {}",
                self.path.display(),
                e
            );
        }
    }
}

fn write_json_atomic(path: &Path, value: &TaskMetricsFile) -> anyhow::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, serde_json::to_vec_pretty(value)?)?;
    std::fs::rename(&tmp, path)?;
    Ok(())
}

// ─── 阶段记录入口（env 未启用时全部 no-op） ─────────────────────────────────

/// 闭包阶段：写完 manifest 后调用。
pub fn record_closure_stage(
    seed_count: usize,
    visited_count: usize,
    rounds: usize,
    missing_count: usize,
    covered: &BTreeMap<u32, usize>,
    duration_ms: u64,
) {
    with_collector(|c| {
        let mut inner = c.inner.lock().expect("task metrics lock");
        inner.stages.closure = Some(ClosureStageMetrics {
            seed_count,
            visited_count,
            rounds,
            missing_count,
            covered_dbnums: covered
                .iter()
                .map(|(dbnum, refnos)| CoveredDbnumMetrics {
                    dbnum: *dbnum,
                    refnos: *refnos,
                })
                .collect(),
            duration_ms,
        });
        c.flush(&inner);
    });
}

/// 解析阶段：apply_sync_filter 决策点记录 mode 与文件全量元素数。
pub fn note_parse_db_mode(dbnum: u32, mode: &str, total_in_file: usize) {
    with_collector(|c| {
        let mut inner = c.inner.lock().expect("task metrics lock");
        inner
            .parse_db_notes
            .insert(dbnum, (mode.to_string(), total_in_file));
    });
}

/// 解析阶段：单库解析完成（含 skipped：elements=0）。
pub fn record_parse_db(dbnum: u32, db_type: &str, elements: usize, duration_ms: u64) {
    with_collector(|c| {
        let mut inner = c.inner.lock().expect("task metrics lock");
        let (mode, total_in_file) = inner
            .parse_db_notes
            .get(&dbnum)
            .cloned()
            .unwrap_or_else(|| ("full".to_string(), elements));
        let parse = inner.stages.parse.get_or_insert_with(Default::default);
        // 同 dbnum 重复记录时覆盖（重试场景取最后一次）。
        parse.dbs.retain(|d| d.dbnum != dbnum);
        parse.dbs.push(ParseDbMetrics {
            dbnum,
            db_type: db_type.to_string(),
            elements,
            total_in_file: total_in_file.max(elements),
            mode,
            duration_ms,
        });
        parse.total_elements = parse.dbs.iter().map(|d| d.elements).sum();
        parse.db_count_refresh();
        c.flush(&inner);
    });
}

impl ParseStageMetrics {
    fn db_count_refresh(&mut self) {
        // 预留：db_count 由 dbs.len() 派生，序列化时无需冗余字段。
    }
}

/// 解析阶段收尾：错误计数 + 阶段总耗时。
pub fn finish_parse_stage(error_count: usize, duration_ms: u64) {
    with_collector(|c| {
        let mut inner = c.inner.lock().expect("task metrics lock");
        let parse = inner.stages.parse.get_or_insert_with(Default::default);
        parse.error_count = error_count;
        parse.duration_ms = duration_ms;
        c.flush(&inner);
    });
}

/// 生成阶段：mesh / boolean 等累计计数（批量屏障汇总点调用，非热路径）。
pub fn add_generate_counters(mesh_generated: usize, mesh_cache_hit: usize) {
    with_collector(|c| {
        let mut inner = c.inner.lock().expect("task metrics lock");
        let g = inner.stages.generate.get_or_insert_with(Default::default);
        g.mesh_generated += mesh_generated;
        g.mesh_cache_hit += mesh_cache_hit;
    });
}

/// 生成阶段：布尔任务结果累计。
pub fn add_boolean_counters(success: usize, failed: usize) {
    with_collector(|c| {
        let mut inner = c.inner.lock().expect("task metrics lock");
        let g = inner.stages.generate.get_or_insert_with(Default::default);
        g.boolean_success += success;
        g.boolean_failed += failed;
    });
}

/// 生成阶段：PerfTimer 分段耗时。
pub fn record_generate_stage_ms(stages: &[(String, u64)]) {
    with_collector(|c| {
        let mut inner = c.inner.lock().expect("task metrics lock");
        let g = inner.stages.generate.get_or_insert_with(Default::default);
        for (name, ms) in stages {
            *g.stage_ms.entry(name.clone()).or_insert(0) += *ms;
        }
        c.flush(&inner);
    });
}

/// 生成阶段收尾：落库数量（调用方查询统计）、错误与 cache miss、总耗时。
pub fn finish_generate_stage(
    inst_relate: usize,
    inst_info: usize,
    inst_relate_aabb: usize,
    tubi_count: usize,
    error_count: usize,
    cache_miss: usize,
    duration_ms: u64,
) {
    with_collector(|c| {
        let mut inner = c.inner.lock().expect("task metrics lock");
        let g = inner.stages.generate.get_or_insert_with(Default::default);
        g.inst_relate = inst_relate;
        g.inst_info = inst_info;
        g.inst_relate_aabb = inst_relate_aabb;
        g.tubi_count = tubi_count;
        g.error_count = error_count;
        g.cache_miss = cache_miss;
        g.duration_ms = duration_ms;
        c.flush(&inner);
    });
}

/// 导出阶段收尾。
pub fn record_export_stage(
    parquet_files: usize,
    parquet_bytes: u64,
    json_files: usize,
    json_bytes: u64,
    duration_ms: u64,
) {
    with_collector(|c| {
        let mut inner = c.inner.lock().expect("task metrics lock");
        let e = inner.stages.export.get_or_insert_with(Default::default);
        e.parquet_files += parquet_files;
        e.parquet_bytes += parquet_bytes;
        e.json_files += json_files;
        e.json_bytes += json_bytes;
        e.duration_ms += duration_ms;
        c.flush(&inner);
    });
}

/// 任务收尾：标记成功/失败并落最终产物。
pub fn finalize_task_metrics(success: bool) {
    with_collector(|c| {
        let mut inner = c.inner.lock().expect("task metrics lock");
        inner.finished = true;
        inner.success = Some(success);
        c.flush(&inner);
    });
}
