//! Full Noun cache-first 运行的缺失报告（写入 output/<project>/cache_miss_report.json）。
//!
//! 设计目标：
//! - cache-first/离线生成流程中，cache miss 代表 Prefetch 不完整（可选择严格失败或记录）
//! - 在一次 Full Noun 运行结束后输出一份“可审计”的 JSON 报告，便于补齐 prefetch 或定位缺口
//! - 避免报告过大：每个 bucket 仅保留少量样例 refno

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use aios_core::RefnoEnum;
use once_cell::sync::OnceCell;
use serde::{Deserialize, Serialize};

use crate::options::DbOptionExt;

const REPORT_VERSION: u32 = 1;
const DEFAULT_SAMPLE_LIMIT: usize = 16;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CacheMissBucket {
    /// 总缺失次数（同类 miss 计数）
    pub count: u64,
    /// 样例 refno（最多 sample_limit 个）
    pub samples: Vec<String>,
    /// 可选备注（用于写入更具体的缺失原因/上下文）
    #[serde(skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
}

impl CacheMissBucket {
    fn new(note: Option<String>) -> Self {
        Self {
            count: 0,
            samples: Vec::new(),
            note,
        }
    }
}

/// cache miss 报告（按 stage/kind 聚合）
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CacheMissReport {
    pub version: u32,
    pub run_id: String,
    pub generated_at: String,
    pub mode: String,
    pub project_name: String,
    pub foyer_cache_dir: String,

    /// 聚合 bucket：key = "{stage}:{kind}"
    pub buckets: BTreeMap<String, CacheMissBucket>,

    /// 仅用于控制样例数量；不写入 JSON。
    #[serde(skip)]
    sample_limit: usize,
}

static GLOBAL_REPORT: OnceCell<Mutex<CacheMissReport>> = OnceCell::new();

impl CacheMissReport {
    pub fn new(db_option: &DbOptionExt, mode: impl Into<String>) -> Self {
        let now = chrono::Utc::now();
        let run_id = format!("{}-{}", now.timestamp_millis(), std::process::id());
        Self {
            version: REPORT_VERSION,
            run_id,
            generated_at: now.to_rfc3339(),
            mode: mode.into(),
            project_name: db_option.inner.project_name.clone(),
            foyer_cache_dir: db_option.get_foyer_cache_dir().display().to_string(),
            buckets: BTreeMap::new(),
            sample_limit: DEFAULT_SAMPLE_LIMIT,
        }
    }

    pub fn with_sample_limit(mut self, limit: usize) -> Self {
        self.sample_limit = limit.max(1);
        self
    }

    /// 记录一次 miss（会聚合计数，并保留少量样例 refno）
    pub fn record_refno_miss(
        &mut self,
        stage: &str,
        kind: &str,
        refno: RefnoEnum,
        note: Option<&str>,
    ) {
        let key = format!("{stage}:{kind}");
        let bucket = self
            .buckets
            .entry(key)
            .or_insert_with(|| CacheMissBucket::new(note.map(|s| s.to_string())));
        bucket.count += 1;
        if bucket.samples.len() < self.sample_limit {
            bucket.samples.push(refno.to_string());
        }
        // 若 bucket 原本无 note，但本次提供了 note，则补上（避免覆盖已有更具体的 note）
        if bucket.note.is_none() {
            bucket.note = note.map(|s| s.to_string());
        }
    }

    pub fn record_simple_miss(&mut self, stage: &str, kind: &str, note: Option<&str>) {
        let key = format!("{stage}:{kind}");
        let bucket = self
            .buckets
            .entry(key)
            .or_insert_with(|| CacheMissBucket::new(note.map(|s| s.to_string())));
        bucket.count += 1;
        if bucket.note.is_none() {
            bucket.note = note.map(|s| s.to_string());
        }
    }

    pub fn default_report_path(db_option: &DbOptionExt) -> PathBuf {
        db_option
            .get_project_output_dir()
            .join("cache_miss_report.json")
    }

    pub fn write_to(&self, path: &Path) -> anyhow::Result<()> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        // 原子写：先写临时文件再 rename，避免中途崩溃留下半截 JSON
        let tmp = path.with_extension("json.tmp");
        let json = serde_json::to_string_pretty(self)?;
        fs::write(&tmp, json)?;
        fs::rename(&tmp, path)?;
        Ok(())
    }

    pub fn write_to_default_path(&self, db_option: &DbOptionExt) -> anyhow::Result<PathBuf> {
        let path = Self::default_report_path(db_option);
        self.write_to(&path)?;
        Ok(path)
    }
}

/// 初始化全局 cache miss 报告（单进程只允许初始化一次；重复初始化会被忽略）。
pub fn init_global_cache_miss_report(db_option: &DbOptionExt, mode: impl Into<String>) {
    let report = CacheMissReport::new(db_option, mode);
    let _ = GLOBAL_REPORT.set(Mutex::new(report));
}

/// 在全局报告上执行一次同步更新（若未初始化则静默跳过）。
pub fn with_global_report<R>(f: impl FnOnce(&mut CacheMissReport) -> R) -> Option<R> {
    let lock = GLOBAL_REPORT.get()?;
    let mut guard = lock.lock().expect("cache_miss_report lock poisoned");
    Some(f(&mut guard))
}

/// 拍一份当前全局报告快照（用于写文件）。
pub fn snapshot_global_report() -> Option<CacheMissReport> {
    with_global_report(|r| r.clone())
}
