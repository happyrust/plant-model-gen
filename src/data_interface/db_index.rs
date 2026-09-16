//! DbIndexStore — 全库 ref0/dbnum 预扫描索引（index-only）
//!
//! 目标：用 `pdms-io` 的 B+树索引能力（只读文件头 + 遍历索引页，不解析元素属性）
//! 扫描站点所有工程根下的全部 db 文件，记录 `dbnum / db_type / owned ref0`，
//! 写入站点级独立 SQLite（`db_index.sqlite`），形成覆盖全库（含尚未导入 SurrealDB
//! 的元件/字典/规格库）的全局 `ref0 -> dbnum` 映射。
//!
//! 与现有 `db_meta_info.json` / [`crate::data_interface::db_meta_manager`] 的关系：
//! - `db_meta_info.json` 是“解析期产物”，其 `ref0_to_dbnum` 只覆盖已解析的库；保留不变。
//! - 本模块产出的 `db_index.sqlite` 是“预扫描产物”，覆盖全部 db 文件，用于解析前的
//!   依赖推导与系统库（SYST/DICT/GLOB）恒解析决策。

use anyhow::{Context, Result};
use rusqlite::{Connection, OptionalExtension, params};
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

use parse_pdms_db::parse::parse_db_basic_info;

/// 站点级索引文件名（位于 `runtime/admin_sites/<site_id>/`）。
pub const DB_INDEX_FILE_NAME: &str = "db_index.sqlite";

/// db 文件索引记录（对应 `db_file_index` 表一行）。
#[derive(Debug, Clone)]
pub struct DbFileRecord {
    pub dbnum: u32,
    pub db_type: String,
    pub file_name: String,
    pub file_path: String,
    pub project: String,
    pub latest_sesno: u32,
    /// 过期判定指纹：`{mtime_nanos}:{size}`（解析前 cheap pre-check 用）。
    pub fingerprint: String,
}

/// 一次预扫描的统计结果。
#[derive(Debug, Default, Clone)]
pub struct ScanReport {
    /// 实际（重新）扫描并写入的 db 文件数。
    pub scanned: usize,
    /// 指纹未变、跳过的 db 文件数。
    pub skipped: usize,
    /// 索引库内 db 文件总数（扫描后）。
    pub db_files: usize,
    /// 本次写入的 ref0 总数。
    pub ref0_total: usize,
    /// 逐文件错误（不致命，记录后继续）。
    pub errors: Vec<String>,
}

/// 单次预扫描的增量进度快照。
#[derive(Debug, Clone)]
pub struct ScanProgress {
    pub project: String,
    pub current_file: String,
    pub processed_files: usize,
    pub scanned: usize,
    pub skipped: usize,
    pub ref0_total: usize,
    pub errors: usize,
}

/// 站点级 ref0/dbnum 索引存储（SQLite）。
pub struct DbIndexStore {
    conn: Connection,
}

impl DbIndexStore {
    /// 打开（必要时创建）索引库并建表。
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref();
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let conn = Connection::open(path)
            .with_context(|| format!("打开 db_index.sqlite 失败: {}", path.display()))?;
        conn.execute_batch(
            "PRAGMA journal_mode=WAL; PRAGMA busy_timeout=5000; PRAGMA synchronous=NORMAL;",
        )
        .context("初始化 db_index pragma 失败")?;
        let store = Self { conn };
        store.ensure_schema()?;
        Ok(store)
    }

    fn ensure_schema(&self) -> Result<()> {
        // 迁移：旧版 ref0_owner(ref0 PRIMARY KEY, dbnum) 以 ref0 为全局主键，既无法表达
        // 「同一 dbnum 拥有的多个 ref0」（跨库同 ref0 会相互覆盖），也未记录 dbfile。
        // 检测到旧结构（缺 file_name 列）则丢弃重建——ref0_owner 是可由预扫重建的缓存，
        // 迁移后需跑一次全量 `scan-db-index` 重新填充。
        let ref0_owner_exists = self
            .conn
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='ref0_owner'",
                [],
                |r| r.get::<_, i64>(0),
            )
            .unwrap_or(0)
            > 0;
        if ref0_owner_exists {
            let has_file_name = self
                .conn
                .query_row(
                    "SELECT COUNT(*) FROM pragma_table_info('ref0_owner') WHERE name='file_name'",
                    [],
                    |r| r.get::<_, i64>(0),
                )
                .unwrap_or(0)
                > 0;
            if !has_file_name {
                self.conn
                    .execute_batch("DROP TABLE IF EXISTS ref0_owner;")
                    .context("迁移 ref0_owner 旧结构失败")?;
            }
        }
        self.conn
            .execute_batch(
                "CREATE TABLE IF NOT EXISTS db_file_index (
                    dbnum        INTEGER PRIMARY KEY,
                    db_type      TEXT    NOT NULL DEFAULT '',
                    file_name    TEXT    NOT NULL DEFAULT '',
                    file_path    TEXT    NOT NULL DEFAULT '',
                    project      TEXT    NOT NULL DEFAULT '',
                    latest_sesno INTEGER NOT NULL DEFAULT 0,
                    fingerprint  TEXT    NOT NULL DEFAULT '',
                    scanned_at   TEXT    NOT NULL DEFAULT ''
                );
                -- 同一 dbnum 可拥有多个 ref0，但一个 ref0 只能属于一个 dbnum。
                CREATE TABLE IF NOT EXISTS ref0_owner (
                    dbnum     INTEGER NOT NULL,
                    ref0      INTEGER NOT NULL,
                    file_name TEXT    NOT NULL DEFAULT '',
                    PRIMARY KEY (dbnum, ref0)
                );
                CREATE TABLE IF NOT EXISTS db_dependency (
                    src_dbnum INTEGER NOT NULL,
                    dst_dbnum INTEGER NOT NULL,
                    PRIMARY KEY (src_dbnum, dst_dbnum)
                );
                CREATE UNIQUE INDEX IF NOT EXISTS unique_ref0_owner_ref0 ON ref0_owner(ref0);
                CREATE INDEX IF NOT EXISTS idx_db_file_index_type ON db_file_index(db_type);
                CREATE INDEX IF NOT EXISTS idx_db_dependency_src ON db_dependency(src_dbnum);",
            )
            .context("建立 db_index schema 失败")?;
        Ok(())
    }

    /// 读取某 dbnum 已存指纹（用于过期判定）。
    pub fn fingerprint_of(&self, dbnum: u32) -> Option<String> {
        self.conn
            .query_row(
                "SELECT fingerprint FROM db_file_index WHERE dbnum = ?1",
                params![dbnum],
                |row| row.get::<_, String>(0),
            )
            .optional()
            .ok()
            .flatten()
    }

    /// upsert 一条 db 文件记录。
    pub fn upsert_db_file(&self, rec: &DbFileRecord) -> Result<()> {
        let now = chrono::Utc::now().to_rfc3339();
        self.conn
            .execute(
                "INSERT INTO db_file_index
                    (dbnum, db_type, file_name, file_path, project, latest_sesno, fingerprint, scanned_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
                 ON CONFLICT(dbnum) DO UPDATE SET
                    db_type=excluded.db_type,
                    file_name=excluded.file_name,
                    file_path=excluded.file_path,
                    project=excluded.project,
                    latest_sesno=excluded.latest_sesno,
                    fingerprint=excluded.fingerprint,
                    scanned_at=excluded.scanned_at",
                params![
                    rec.dbnum,
                    rec.db_type,
                    rec.file_name,
                    rec.file_path,
                    rec.project,
                    rec.latest_sesno,
                    rec.fingerprint,
                    now
                ],
            )
            .with_context(|| format!("写入 db_file_index 失败 dbnum={}", rec.dbnum))?;
        Ok(())
    }

    /// 用最新 owned ref0 集合覆盖某 dbnum 的 ref0_owner 记录（事务）。
    ///
    /// - `file_name`：该 dbnum 对应的 dbfile 文件名，随 ref0 一并存储。
    /// 唯一索引保证一个 ref0 只对应一个 dbnum；冲突会回滚整个事务。
    pub fn replace_ref0_owners(&self, dbnum: u32, file_name: &str, ref0s: &[u32]) -> Result<()> {
        let tx = self.conn.unchecked_transaction()?;
        tx.execute("DELETE FROM ref0_owner WHERE dbnum = ?1", params![dbnum])?;
        {
            let mut stmt = tx.prepare(
                "INSERT INTO ref0_owner (dbnum, ref0, file_name) VALUES (?1, ?2, ?3)",
            )?;
            for &ref0 in ref0s {
                stmt.execute(params![dbnum, ref0, file_name])?;
            }
        }
        tx.commit()?;
        Ok(())
    }

    /// 全局唯一 ref0 -> dbnum。
    pub fn dbnum_by_ref0(&self, ref0: u32) -> Option<u32> {
        self.conn
            .query_row(
                "SELECT dbnum FROM ref0_owner WHERE ref0 = ?1",
                params![ref0],
                |row| row.get::<_, u32>(0),
            )
            .optional()
            .ok()
            .flatten()
    }

    /// 取某 dbnum 拥有的（去重、升序）ref0 列表。
    pub fn ref0s_by_dbnum(&self, dbnum: u32) -> Vec<u32> {
        let mut out = Vec::new();
        let Ok(mut stmt) = self
            .conn
            .prepare("SELECT ref0 FROM ref0_owner WHERE dbnum = ?1 ORDER BY ref0")
        else {
            return out;
        };
        if let Ok(rows) = stmt.query_map(params![dbnum], |row| row.get::<_, u32>(0)) {
            for r in rows.flatten() {
                out.push(r);
            }
        }
        out
    }

    /// 取某 dbnum 在 ref0_owner 中记录的 dbfile 文件名。
    pub fn dbfile_of(&self, dbnum: u32) -> Option<String> {
        self.conn
            .query_row(
                "SELECT file_name FROM ref0_owner WHERE dbnum = ?1 LIMIT 1",
                params![dbnum],
                |row| row.get::<_, String>(0),
            )
            .optional()
            .ok()
            .flatten()
    }

    /// 一组 ref0 -> 去重 dbnum 列表。
    pub fn resolve_dbnums(&self, ref0s: &[u32]) -> Vec<u32> {
        let mut set: BTreeSet<u32> = BTreeSet::new();
        for &ref0 in ref0s {
            if let Some(dbnum) = self.dbnum_by_ref0(ref0) {
                set.insert(dbnum);
            }
        }
        set.into_iter().collect()
    }

    /// 按 db_type 取 dbnum 列表（大小写不敏感）。
    pub fn dbnums_by_type(&self, db_type: &str) -> Vec<u32> {
        let mut out = Vec::new();
        let Ok(mut stmt) = self
            .conn
            .prepare("SELECT dbnum FROM db_file_index WHERE UPPER(db_type) = UPPER(?1)")
        else {
            return out;
        };
        if let Ok(rows) = stmt.query_map(params![db_type], |row| row.get::<_, u32>(0)) {
            for dbnum in rows.flatten() {
                out.push(dbnum);
            }
        }
        out
    }

    /// 全量导出 `ref0 -> dbnum` 映射（内存定位器用；行数 = Σ每库 ref0 数，量级小）。
    pub fn all_ref0_owners(&self) -> Vec<(u32, u32)> {
        let mut out = Vec::new();
        let Ok(mut stmt) = self.conn.prepare("SELECT ref0, dbnum FROM ref0_owner") else {
            return out;
        };
        if let Ok(rows) =
            stmt.query_map([], |row| Ok((row.get::<_, u32>(0)?, row.get::<_, u32>(1)?)))
        {
            for r in rows.flatten() {
                out.push(r);
            }
        }
        out
    }

    /// 全量导出 db 文件记录（内存定位器用）。
    pub fn all_db_files(&self) -> Vec<DbFileRecord> {
        let mut out = Vec::new();
        let Ok(mut stmt) = self.conn.prepare(
            "SELECT dbnum, db_type, file_name, file_path, project, latest_sesno, fingerprint
             FROM db_file_index",
        ) else {
            return out;
        };
        if let Ok(rows) = stmt.query_map([], |row| {
            Ok(DbFileRecord {
                dbnum: row.get(0)?,
                db_type: row.get(1)?,
                file_name: row.get(2)?,
                file_path: row.get(3)?,
                project: row.get(4)?,
                latest_sesno: row.get(5)?,
                fingerprint: row.get(6)?,
            })
        }) {
            for r in rows.flatten() {
                out.push(r);
            }
        }
        out
    }

    /// 取某 dbnum 的文件记录。
    pub fn file_by_dbnum(&self, dbnum: u32) -> Option<DbFileRecord> {
        self.conn
            .query_row(
                "SELECT dbnum, db_type, file_name, file_path, project, latest_sesno, fingerprint
                 FROM db_file_index WHERE dbnum = ?1",
                params![dbnum],
                |row| {
                    Ok(DbFileRecord {
                        dbnum: row.get(0)?,
                        db_type: row.get(1)?,
                        file_name: row.get(2)?,
                        file_path: row.get(3)?,
                        project: row.get(4)?,
                        latest_sesno: row.get(5)?,
                        fingerprint: row.get(6)?,
                    })
                },
            )
            .optional()
            .ok()
            .flatten()
    }

    /// 索引库内 db 文件总数。
    pub fn db_file_count(&self) -> usize {
        self.conn
            .query_row("SELECT COUNT(*) FROM db_file_index", [], |row| {
                row.get::<_, i64>(0)
            })
            .map(|n| n as usize)
            .unwrap_or(0)
    }

    /// 覆盖式记录 src 设计库的外部依赖 dbnum 集合（跳过自依赖）。
    pub fn record_dependencies(&self, src_dbnum: u32, dst_dbnums: &[u32]) -> Result<()> {
        let tx = self.conn.unchecked_transaction()?;
        tx.execute(
            "DELETE FROM db_dependency WHERE src_dbnum = ?1",
            params![src_dbnum],
        )?;
        {
            let mut stmt = tx.prepare(
                "INSERT OR IGNORE INTO db_dependency (src_dbnum, dst_dbnum) VALUES (?1, ?2)",
            )?;
            for &dst in dst_dbnums {
                if dst != src_dbnum {
                    stmt.execute(params![src_dbnum, dst])?;
                }
            }
        }
        tx.commit()?;
        Ok(())
    }

    /// 取 src 的直接依赖 dbnum。
    pub fn dst_dbnums_of(&self, src_dbnum: u32) -> Vec<u32> {
        let mut out = Vec::new();
        let Ok(mut stmt) = self
            .conn
            .prepare("SELECT dst_dbnum FROM db_dependency WHERE src_dbnum = ?1")
        else {
            return out;
        };
        if let Ok(rows) = stmt.query_map(params![src_dbnum], |row| row.get::<_, u32>(0)) {
            for dst in rows.flatten() {
                out.push(dst);
            }
        }
        out
    }

    /// 从 seeds 出发沿 db_dependency 边求传递闭包（BFS，带环检测）。
    /// 返回不含 seeds 本身的外部依赖 dbnum 列表。
    pub fn resolve_related_closure(&self, seeds: &[u32]) -> Vec<u32> {
        use std::collections::VecDeque;
        let seed_set: BTreeSet<u32> = seeds.iter().copied().collect();
        let mut visited: BTreeSet<u32> = BTreeSet::new();
        let mut result: BTreeSet<u32> = BTreeSet::new();
        let mut queue: VecDeque<u32> = seeds.iter().copied().collect();
        while let Some(cur) = queue.pop_front() {
            if !visited.insert(cur) {
                continue;
            }
            for dst in self.dst_dbnums_of(cur) {
                if !seed_set.contains(&dst) {
                    result.insert(dst);
                }
                if !visited.contains(&dst) {
                    queue.push_back(dst);
                }
            }
        }
        result.into_iter().collect()
    }
}

/// 文件过期指纹：`{mtime_nanos}:{size}`。
fn file_fingerprint(path: &Path) -> Result<String> {
    let md = std::fs::metadata(path)?;
    let size = md.len();
    let mtime = md
        .modified()
        .ok()
        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    Ok(format!("{mtime}:{size}"))
}

fn inactive_db_path(path: &Path) -> bool {
    if path.components().any(|component| {
        matches!(
            component
                .as_os_str()
                .to_string_lossy()
                .to_ascii_lowercase()
                .as_str(),
            "back" | "backup" | "cbas"
        )
    }) {
        return true;
    }

    let name = path
        .file_name()
        .map(|s| s.to_string_lossy().to_ascii_lowercase())
        .unwrap_or_default();
    name.ends_with("_old")
        || name.ends_with("-old")
        || name.ends_with(".old")
        || name.contains("_old.")
        || name.contains("-old.")
        || name.contains(" copy")
        || name.contains("_copy")
        || name.contains("-copy")
        || name.ends_with("_new")
        || name.ends_with("-new")
        || name.ends_with(".new")
        || name.contains("_new.")
        || name.contains("-new.")
        || name.ends_with("_test")
        || name.ends_with("-test")
        || name.ends_with(".test")
        || name.contains("_test.")
        || name.contains("-test.")
}

fn db_candidate_rank(path: &Path) -> (u8, usize, String) {
    let parent = path
        .parent()
        .and_then(|p| p.file_name())
        .map(|s| s.to_string_lossy().to_ascii_lowercase())
        .unwrap_or_default();
    let parent_rank = if parent.ends_with("000") { 0 } else { 1 };
    let depth = path.components().count();
    (
        parent_rank,
        usize::MAX.saturating_sub(depth),
        path.to_string_lossy().to_string(),
    )
}

/// index-only 扫描单个 db 文件：打开 -> 取最新会话号 -> 遍历整棵 B+树索引取全部 owned ref0。
///
/// 不解析元素记录/属性（不调用 `parse_db_basic_data`）。
fn scan_one_db(project: &str, path: &Path) -> Result<(u32, Vec<u32>)> {
    let mut io = pdms_io::PdmsIO::new(project, path, false);
    io.open()
        .with_context(|| format!("PdmsIO::open 失败: {}", path.display()))?;
    let latest_sesno = io.get_latest_sesno().unwrap_or(0);
    let index_map = io
        .build_index_map()
        .with_context(|| format!("build_index_map 失败: {}", path.display()))?;

    let mut ref0s: BTreeSet<u32> = BTreeSet::new();
    for refno in index_map.keys() {
        let ref0 = refno.get_0();
        // 跳过 B+树起始标记 0x80000001 与无效 0。
        if ref0 != 0 && ref0 != 0x8000_0001 {
            ref0s.insert(ref0);
        }
    }
    Ok((latest_sesno, ref0s.into_iter().collect()))
}

/// 对一组 `(project_name, root_path)` 做 index-only 预扫描，写入索引库。
///
/// - `force=false`：按指纹（mtime+size）增量，未变更的 db 跳过，不打开 PdmsIO。
/// - `force=true`：忽略指纹，全部重扫。
///
/// 单个文件失败/panic 不致命，记入 `ScanReport.errors` 后继续。
pub fn prescan_roots(store: &DbIndexStore, roots: &[(String, PathBuf)], force: bool) -> ScanReport {
    prescan_roots_with_progress(store, roots, force, |_| {})
}

/// 带进度回调的 index-only 预扫描。
///
/// 回调在发现一个有效 db 文件并完成扫描/跳过/记录错误后触发；调用方可自行节流。
pub fn prescan_roots_with_progress<F>(
    store: &DbIndexStore,
    roots: &[(String, PathBuf)],
    force: bool,
    mut on_progress: F,
) -> ScanReport
where
    F: FnMut(ScanProgress),
{
    let mut report = ScanReport::default();
    let mut candidates = Vec::new();

    for (project, root) in roots {
        if !root.exists() {
            report
                .errors
                .push(format!("工程根不存在: {}", root.display()));
            continue;
        }
        for entry in walkdir::WalkDir::new(root)
            .max_depth(8)
            .into_iter()
            .filter_map(|e| e.ok())
        {
            if !entry.file_type().is_file() {
                continue;
            }
            let path = entry.path();
            if inactive_db_path(path) {
                continue;
            }

            // cheap 读头：拿 dbnum/db_type，非 db 文件（dbnum=0）跳过。
            let info = parse_db_basic_info(path.to_path_buf());
            if info.dbnum == 0 {
                continue;
            }
            candidates.push((project.clone(), path.to_path_buf(), info));
        }
    }

    candidates.sort_by_key(|(_, path, info)| (info.dbnum, db_candidate_rank(path)));
    let mut seen_dbnums = BTreeSet::new();
    for (project, path, info) in candidates {
        if !seen_dbnums.insert(info.dbnum) {
            continue;
        }

        let fingerprint = match file_fingerprint(&path) {
            Ok(fp) => fp,
            Err(_) => continue,
        };

        // 增量：指纹未变则跳过（不打开 PdmsIO）。
        if !force {
            if let Some(stored) = store.fingerprint_of(info.dbnum) {
                if stored == fingerprint {
                    report.skipped += 1;
                    emit_scan_progress(&report, &project, &path, &mut on_progress);
                    continue;
                }
            }
        }

        // 单文件扫描隔离 panic，避免一个坏文件中断整轮预扫。
        let scan = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            scan_one_db(&project, &path)
        }));
        match scan {
            Ok(Ok((latest_sesno, ref0s))) => {
                let rec = DbFileRecord {
                    dbnum: info.dbnum,
                    db_type: info.db_type.clone(),
                    file_name: path
                        .file_name()
                        .map(|s| s.to_string_lossy().to_string())
                        .unwrap_or_default(),
                    file_path: path.to_string_lossy().to_string(),
                    project: project.clone(),
                    latest_sesno,
                    fingerprint,
                };
                if let Err(e) = store.upsert_db_file(&rec) {
                    report
                        .errors
                        .push(format!("{}: upsert 失败 {}", path.display(), e));
                    continue;
                }
                if let Err(e) = store.replace_ref0_owners(info.dbnum, &rec.file_name, &ref0s) {
                    report
                        .errors
                        .push(format!("{}: 写 ref0_owner 失败 {}", path.display(), e));
                    continue;
                }
                report.scanned += 1;
                report.ref0_total += ref0s.len();
            }
            Ok(Err(e)) => report.errors.push(format!("{}: {}", path.display(), e)),
            Err(_) => report
                .errors
                .push(format!("{}: 扫描时 panic（已跳过）", path.display())),
        }
        emit_scan_progress(&report, &project, &path, &mut on_progress);
    }

    report.db_files = store.db_file_count();
    report
}

fn emit_scan_progress<F>(report: &ScanReport, project: &str, path: &Path, on_progress: &mut F)
where
    F: FnMut(ScanProgress),
{
    on_progress(ScanProgress {
        project: project.to_string(),
        current_file: path.to_string_lossy().to_string(),
        processed_files: report.scanned + report.skipped + report.errors.len(),
        scanned: report.scanned,
        skipped: report.skipped,
        ref0_total: report.ref0_total,
        errors: report.errors.len(),
    });
}

/// 从单库完整属性数据中抽取外向引用的 ref0 集合（RefU64Type / RefU64Array 属性值）。
fn extract_outbound_ref0s(data: &parse_pdms_db::parse::PdmsDbData) -> Vec<u32> {
    use aios_core::NamedAttrValue;
    let mut set: BTreeSet<u32> = BTreeSet::new();
    let push = |r0: u32, set: &mut BTreeSet<u32>| {
        if r0 != 0 && r0 != 0x8000_0001 {
            set.insert(r0);
        }
    };
    for entry in data.total_attr_map.iter() {
        for value in entry.value().map.values() {
            match value {
                NamedAttrValue::RefU64Type(r) => push(r.get_0(), &mut set),
                NamedAttrValue::RefU64Array(arr) => {
                    for &refno_enum in arr {
                        push(refno_enum.refno().get_0(), &mut set);
                    }
                }
                NamedAttrValue::RefnoEnumType(refno_enum) => {
                    push(refno_enum.refno().get_0(), &mut set);
                }
                _ => {}
            }
        }
    }
    set.into_iter().collect()
}

fn collect_design_db_candidates(
    roots: &[(String, PathBuf)],
    targets: Option<&BTreeSet<u32>>,
) -> Vec<(String, PathBuf, u32)> {
    let mut candidates = Vec::new();
    for (project, root) in roots {
        if !root.exists() {
            continue;
        }
        for entry in walkdir::WalkDir::new(root)
            .max_depth(8)
            .into_iter()
            .filter_map(|e| e.ok())
        {
            if !entry.file_type().is_file() {
                continue;
            }
            let path = entry.path();
            if inactive_db_path(path) {
                continue;
            }
            let info = parse_db_basic_info(path.to_path_buf());
            if info.dbnum == 0 || !info.db_type.eq_ignore_ascii_case("DESI") {
                continue;
            }
            if let Some(targets) = targets {
                if !targets.contains(&info.dbnum) {
                    continue;
                }
            }
            candidates.push((project.clone(), path.to_path_buf(), info.dbnum));
        }
    }

    candidates.sort_by_key(|(_, path, dbnum)| (*dbnum, db_candidate_rank(path)));
    let mut seen = BTreeSet::new();
    candidates
        .into_iter()
        .filter(|(_, _, dbnum)| seen.insert(*dbnum))
        .collect()
}

/// 异步：扫描 roots 下的 DESI 设计库，解析其属性并抽取外向 ref0。
///
/// 返回 `(src_dbnum, outbound_ref0s)` 列表；**不持有 SQLite 连接**，便于在 async 上下文调用
/// （写库由调用方在 spawn_blocking 中完成，避免把 rusqlite Connection 跨 await 持有）。
pub async fn collect_design_outbound(roots: &[(String, PathBuf)]) -> Vec<(u32, Vec<u32>)> {
    let mut out: Vec<(u32, Vec<u32>)> = Vec::new();
    for (project, path, dbnum) in collect_design_db_candidates(roots, None) {
        let file_name = path
            .file_name()
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_default();
        match parse_pdms_db::parse::parse_file(&path, &None, &file_name, &project).await {
            Ok(data) => {
                let ref0s = extract_outbound_ref0s(&data);
                if !ref0s.is_empty() {
                    out.push((dbnum, ref0s));
                }
            }
            Err(e) => {
                log::warn!("collect_design_outbound 解析失败 {}: {}", path.display(), e);
            }
        }
    }
    out
}

/// 异步：只扫描指定 dbnum 的 DESI 设计库并抽取外向 ref0。
///
/// quick deploy 通常只关心用户指定的目标库；如果这里继续解析 roots 下所有 DESI，
/// 会把一次单库快测放大成全工程深扫。
pub async fn collect_design_outbound_for_dbnums(
    roots: &[(String, PathBuf)],
    dbnums: &[u32],
) -> Vec<(u32, Vec<u32>)> {
    let targets: BTreeSet<u32> = dbnums.iter().copied().filter(|dbnum| *dbnum > 0).collect();
    if targets.is_empty() {
        return Vec::new();
    }

    let mut out: Vec<(u32, Vec<u32>)> = Vec::new();
    for (project, path, dbnum) in collect_design_db_candidates(roots, Some(&targets)) {
        let file_name = path
            .file_name()
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_default();
        match parse_pdms_db::parse::parse_file(&path, &None, &file_name, &project).await {
            Ok(data) => {
                let ref0s = extract_outbound_ref0s(&data);
                if !ref0s.is_empty() {
                    out.push((dbnum, ref0s));
                }
            }
            Err(e) => {
                log::warn!(
                    "collect_design_outbound_for_dbnums 解析失败 {}: {}",
                    path.display(),
                    e
                );
            }
        }
    }
    out
}

/// 从 `DB_OPTION_FILE` 环境变量（缺省 `db_options/DbOption`）加载 CLI 配置。
pub fn load_db_option_from_env() -> anyhow::Result<aios_core::options::DbOption> {
    let config_name =
        std::env::var("DB_OPTION_FILE").unwrap_or_else(|_| "db_options/DbOption".to_string());
    let config_path = format!("{}.toml", config_name);
    let content = std::fs::read_to_string(&config_path)
        .map_err(|e| anyhow::anyhow!("读取配置 {} 失败: {}", config_path, e))?;
    toml::from_str(&content).map_err(|e| anyhow::anyhow!("解析配置 {} 失败: {}", config_path, e))
}

/// 从配置派生 `(project_name, root)` 工程根列表（预扫描 / 闭包 pass 口径一致）。
pub fn derive_project_roots(
    db_option: &aios_core::options::DbOption,
) -> anyhow::Result<Vec<(String, PathBuf)>> {
    let project_name = db_option.project_name.clone();
    let mut roots: Vec<(String, PathBuf)> = Vec::new();
    for p in &db_option.included_projects {
        if let Some(path) = db_option.get_project_path(p) {
            if path.exists() {
                roots.push((p.clone(), path));
            }
        }
    }
    if roots.is_empty() {
        let base = PathBuf::from(&db_option.project_path);
        let candidate = base.join(&project_name);
        let root = if candidate.exists() { candidate } else { base };
        if root.exists() {
            roots.push((project_name.clone(), root));
        }
    }
    if roots.is_empty() {
        anyhow::bail!(
            "未能从配置派生有效工程根 (project_path={}, project_name={})",
            db_option.project_path,
            project_name
        );
    }
    Ok(roots)
}

/// CLI 口径的 `db_index.sqlite` 落盘路径：`<output_root>/<project>/scene_tree/db_index.sqlite`
/// （与 `db_meta_info.json` / `cata_closure.json` 同目录，读写口径同源）。
pub fn default_index_path(project_name: &str) -> PathBuf {
    crate::versioned_db::db_meta_info::get_project_tree_dir(project_name).join(DB_INDEX_FILE_NAME)
}

/// CLI 入口：从 `DB_OPTION_FILE` 配置派生工程根，做 index-only 全量/增量预扫 +
/// 设计库精确依赖边，写入 [`default_index_path`]。
///
/// `force=true` 全量重扫；`force=false` 按指纹增量。
pub async fn rebuild_from_config(force: bool) -> anyhow::Result<ScanReport> {
    let db_option = load_db_option_from_env()?;
    let project_name = db_option.project_name.clone();
    let roots = derive_project_roots(&db_option)?;

    let out_path = default_index_path(&project_name);

    // Phase 1（同步）：index-only 归属扫描；在 await 前释放连接以保持 Send。
    let report = {
        let store = DbIndexStore::open(&out_path)?;
        prescan_roots(&store, &roots, force)
    };

    // Phase 2（async）：设计库外向引用（不持有连接）。
    // 单库/少量库部署时只解析目标 DESI，避免把一次快速部署放大成全工程 DESI 深扫。
    let manual_db_nums = db_option
        .manual_db_nums
        .as_deref()
        .unwrap_or_default()
        .to_vec();
    let outbound = if manual_db_nums.iter().any(|dbnum| *dbnum > 0) {
        collect_design_outbound_for_dbnums(&roots, &manual_db_nums).await
    } else {
        collect_design_outbound(&roots).await
    };

    // Phase 3（同步）：记录精确依赖边。
    let mut edges = 0usize;
    {
        let store = DbIndexStore::open(&out_path)?;
        for (src, ref0s) in &outbound {
            let mut dsts = store.resolve_dbnums(ref0s);
            dsts.retain(|d| d != src);
            store.record_dependencies(*src, &dsts)?;
            edges += dsts.len();
        }
    }

    println!(
        "✅ db_index 重建: {} 库, {} 条 ref0 映射, {} 条依赖边 → {}",
        report.db_files,
        report.ref0_total,
        edges,
        out_path.display()
    );
    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_store_roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(DB_INDEX_FILE_NAME);
        let store = DbIndexStore::open(&path).unwrap();

        store
            .upsert_db_file(&DbFileRecord {
                dbnum: 17496,
                db_type: "DESI".to_string(),
                file_name: "aps250132_0001".to_string(),
                file_path: "/x/aps250132_0001".to_string(),
                project: "AvevaPlantSample".to_string(),
                latest_sesno: 12,
                fingerprint: "111:222".to_string(),
            })
            .unwrap();
        // 同一 ref0 传入两次，验证 (dbnum, ref0) 主键去重。
        store
            .replace_ref0_owners(17496, "aps250132_0001", &[2013286676, 100, 100])
            .unwrap();

        assert_eq!(store.dbnum_by_ref0(2013286676), Some(17496));
        assert_eq!(store.dbnum_by_ref0(100), Some(17496));
        assert_eq!(store.dbnum_by_ref0(999), None);
        assert_eq!(store.resolve_dbnums(&[2013286676, 100, 999]), vec![17496]);
        assert_eq!(store.dbnums_by_type("desi"), vec![17496]);
        assert_eq!(store.fingerprint_of(17496).as_deref(), Some("111:222"));
        assert_eq!(store.db_file_count(), 1);

        // dbnum -> 去重后的 ref0 列表（升序），以及随存的 dbfile。
        assert_eq!(store.ref0s_by_dbnum(17496), vec![100, 2013286676]);
        assert_eq!(store.dbfile_of(17496).as_deref(), Some("aps250132_0001"));

        // 覆盖式更新 ref0（迁移/删除场景）。
        store
            .replace_ref0_owners(17496, "aps250132_0001", &[2013286676])
            .unwrap();
        assert_eq!(store.dbnum_by_ref0(100), None);
        assert_eq!(store.dbnum_by_ref0(2013286676), Some(17496));
        assert_eq!(store.ref0s_by_dbnum(17496), vec![2013286676]);
    }

    #[test]
    fn test_dependency_closure() {
        let dir = tempfile::tempdir().unwrap();
        let store = DbIndexStore::open(dir.path().join(DB_INDEX_FILE_NAME)).unwrap();
        // 100(DESI) -> 200(CATA) -> 300(DICT)；200 -> 400；环 300 -> 200。
        store.record_dependencies(100, &[200]).unwrap();
        store.record_dependencies(200, &[300, 400]).unwrap();
        store.record_dependencies(300, &[200]).unwrap(); // 制造环
        let closure = store.resolve_related_closure(&[100]);
        assert_eq!(closure, vec![200, 300, 400]);

        // 自依赖被跳过。
        store.record_dependencies(500, &[500, 600]).unwrap();
        assert_eq!(store.dst_dbnums_of(500), vec![600]);
    }
}
