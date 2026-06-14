//! MBD 部署前候选发现（.planning/2026-06-12-mbd-deploy-preflight Phase 2/4）
//!
//! 离线读取工程根下的 SYST 库文件，枚举 MDB 元素及其成员 DB（CURD 列表），
//! 并把成员 dbnum 映射到当前 `projects[]` 可定位的 db 文件，输出
//! `available / missing / ambiguous` 状态，供站点部署前依赖完整性检查。
//!
//! 架构边界：与工程扫描、db file resolve 一样放在 aios-database sidecar 侧，
//! web_server 不直接读取 E3D DB 文件。本模块只做只读发现，不写任何持久化。

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::path::{Path, PathBuf};

use aios_core::tool::db_tool::db1_hash;
use anyhow::Result;
use parse_pdms_db::parse::{parse_db_basic_info, parse_file};
use serde::Serialize;

/// 与 parse_sidecar 扫描口径一致的安全上限。
const SCAN_MAX_DEPTH: usize = 8;
const SCAN_MAX_FILES: usize = 200_000;

/// MDB 成员 DB 文件定位状态。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum MdbDbFileLocateStatus {
    /// 唯一定位到一个 db 文件。
    Available,
    /// 工程根下找不到该 dbnum 对应文件。
    Missing,
    /// 多个不同路径声明同一 dbnum，需要用户消歧。
    Ambiguous,
}

/// MDB 成员 DB 的文件定位结果（对应部署前依赖检查的一行）。
#[derive(Debug, Clone, Serialize)]
pub struct MdbDbFileStatus {
    pub dbnum: u32,
    /// 显示用 db 类型：定位到文件时取文件头类型，否则按 SYST STYP 推导。
    pub db_type: String,
    /// SYST 中 DB 元素名称，例如 `/SAMPLE/DESI`。
    pub db_name: String,
    /// 定位到的文件名（available 时非空）。
    pub file_name: String,
    /// 定位到的文件路径（available 时非空）。
    pub file_path: String,
    /// 文件所在工程名（available 时非空）。
    pub source_project: String,
    pub status: MdbDbFileLocateStatus,
    /// ambiguous 时给出全部候选文件路径。
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub candidates: Vec<String>,
}

/// 一个 MDB 候选及其依赖完整性检查结果。
#[derive(Debug, Clone, Serialize)]
pub struct MdbCandidate {
    /// MDB 名称，保留前导 `/`，例如 `/SAMPLE`。
    pub mdb_name: String,
    /// 来源 SYST 所属工程名。
    pub project: String,
    /// 来源 SYST 文件路径（证据可回溯）。
    pub syst_file: String,
    /// CURD 顺序的成员 dbnum 列表。
    pub dbnums: Vec<u32>,
    pub db_files: Vec<MdbDbFileStatus>,
    pub available_count: usize,
    pub missing_count: usize,
    pub ambiguous_count: usize,
    /// 按 db 类型聚合的成员数量，用于下拉框摘要文案（DESI N · CATA N）。
    pub type_counts: BTreeMap<String, usize>,
    /// missing == 0 且 ambiguous == 0 时可部署。
    pub ready_to_deploy: bool,
}

/// MBD 候选发现结果。
#[derive(Debug, Default, Serialize)]
pub struct MdbCandidatesResult {
    pub candidates: Vec<MdbCandidate>,
    pub warnings: Vec<String>,
}

/// 工程根下扫到的 db 文件清单项。
#[derive(Debug, Clone)]
struct DbFileEntry {
    dbnum: u32,
    db_type: String,
    ses_pgno: u32,
    file_name: String,
    file_path: PathBuf,
    project: String,
}

/// 在 `roots = [(project_name, root_path)]` 下做只读 MBD 候选发现。
///
/// 1. 扫描全部 db 文件头建立 dbnum -> 文件清单；
/// 2. 解析每个工程的 SYST 库（同 dbnum 多副本取 ses_pgno 最大者）；
/// 3. 枚举 MDB 元素与 CURD 成员，逐个成员定位文件并标注状态。
pub async fn discover_mdb_candidates(roots: &[(String, PathBuf)]) -> MdbCandidatesResult {
    let mut result = MdbCandidatesResult::default();
    let inventory = collect_db_file_inventory(roots, &mut result.warnings);

    // dbnum -> 候选文件（跨工程根合并，按 canonical 路径去重）
    let mut files_by_dbnum: BTreeMap<u32, Vec<DbFileEntry>> = BTreeMap::new();
    // (project, syst dbnum) -> 最新 SYST 文件
    let mut syst_by_project: BTreeMap<(String, u32), DbFileEntry> = BTreeMap::new();
    for entry in &inventory {
        if entry.db_type.eq_ignore_ascii_case("SYST") {
            let key = (entry.project.clone(), entry.dbnum);
            let replace = syst_by_project
                .get(&key)
                .map(|existing| entry.ses_pgno > existing.ses_pgno)
                .unwrap_or(true);
            if replace {
                syst_by_project.insert(key, entry.clone());
            }
            continue;
        }
        files_by_dbnum
            .entry(entry.dbnum)
            .or_default()
            .push(entry.clone());
    }

    if syst_by_project.is_empty() {
        result
            .warnings
            .push("未在任何工程根下发现 SYST 系统库文件，无法枚举 MDB 候选".to_string());
        return result;
    }

    let mdb_noun_hash = db1_hash("MDB");
    // 同一工程内多个 SYST dbnum 时按 mdb_name 去重，保留先解析到的。
    let mut seen_mdb_keys: BTreeSet<(String, String)> = BTreeSet::new();
    for ((project, _dbnum), syst_entry) in &syst_by_project {
        let parsed = parse_file(&syst_entry.file_path, &None, &syst_entry.file_name, project).await;
        let data = match parsed {
            Ok(data) => data,
            Err(err) => {
                result.warnings.push(format!(
                    "解析 SYST 失败 {}: {err}",
                    syst_entry.file_path.display()
                ));
                continue;
            }
        };

        let Some(mdb_refnos) = data.type_ele_map.get(&mdb_noun_hash) else {
            continue;
        };
        let mut mdb_refnos: Vec<_> = mdb_refnos.iter().copied().collect();
        mdb_refnos.sort();

        for mdb_refno in mdb_refnos {
            let Some(mdb_attr) = data.total_attr_map.get(&mdb_refno) else {
                continue;
            };
            let mdb_name = normalize_mdb_name(&mdb_attr.get_name_or_default());
            if mdb_name.is_empty() {
                continue;
            }
            if !seen_mdb_keys.insert((project.clone(), mdb_name.clone())) {
                continue;
            }
            let members = mdb_attr.get_refu64_vec("CURD").unwrap_or_default();
            if members.is_empty() {
                result
                    .warnings
                    .push(format!("MDB {mdb_name}（{project}）没有 CURD 成员，已跳过"));
                continue;
            }

            let mut candidate = MdbCandidate {
                mdb_name,
                project: project.clone(),
                syst_file: syst_entry.file_path.to_string_lossy().to_string(),
                dbnums: Vec::with_capacity(members.len()),
                db_files: Vec::with_capacity(members.len()),
                available_count: 0,
                missing_count: 0,
                ambiguous_count: 0,
                type_counts: BTreeMap::new(),
                ready_to_deploy: false,
            };

            for member in members {
                let member_attr = data.total_attr_map.get(&member);
                let Some(member_attr) = member_attr else {
                    result.warnings.push(format!(
                        "MDB {}（{}）成员 {} 在 SYST 中无属性记录，已跳过该成员",
                        candidate.mdb_name, project, member
                    ));
                    continue;
                };
                let Some(dbnum) = member_attr.get_i32("NUMBDB").filter(|v| *v > 0) else {
                    result.warnings.push(format!(
                        "MDB {}（{}）成员 {} 缺少 NUMBDB，已跳过该成员",
                        candidate.mdb_name, project, member
                    ));
                    continue;
                };
                let dbnum = dbnum as u32;
                let db_name = member_attr.get_name_or_default();
                let styp_type = stype_label(member_attr.get_i32("STYP"));
                let status = locate_db_file(dbnum, &db_name, styp_type, files_by_dbnum.get(&dbnum));
                match status.status {
                    MdbDbFileLocateStatus::Available => candidate.available_count += 1,
                    MdbDbFileLocateStatus::Missing => candidate.missing_count += 1,
                    MdbDbFileLocateStatus::Ambiguous => candidate.ambiguous_count += 1,
                }
                *candidate
                    .type_counts
                    .entry(status.db_type.clone())
                    .or_default() += 1;
                candidate.dbnums.push(dbnum);
                candidate.db_files.push(status);
            }

            candidate.ready_to_deploy =
                candidate.missing_count == 0 && candidate.ambiguous_count == 0;
            result.candidates.push(candidate);
        }
    }

    result
        .candidates
        .sort_by(|a, b| a.mdb_name.cmp(&b.mdb_name).then(a.project.cmp(&b.project)));
    result
}

/// 给单个成员 dbnum 定位文件并生成状态行。
fn locate_db_file(
    dbnum: u32,
    db_name: &str,
    styp_type: &str,
    entries: Option<&Vec<DbFileEntry>>,
) -> MdbDbFileStatus {
    let mut status = MdbDbFileStatus {
        dbnum,
        db_type: styp_type.to_string(),
        db_name: db_name.to_string(),
        file_name: String::new(),
        file_path: String::new(),
        source_project: String::new(),
        status: MdbDbFileLocateStatus::Missing,
        candidates: Vec::new(),
    };
    let Some(entries) = entries.filter(|entries| !entries.is_empty()) else {
        return status;
    };
    if entries.len() == 1 {
        let entry = &entries[0];
        status.db_type = entry.db_type.clone();
        status.file_name = entry.file_name.clone();
        status.file_path = entry.file_path.to_string_lossy().to_string();
        status.source_project = entry.project.clone();
        status.status = MdbDbFileLocateStatus::Available;
        return status;
    }
    status.db_type = entries[0].db_type.clone();
    status.status = MdbDbFileLocateStatus::Ambiguous;
    status.candidates = entries
        .iter()
        .map(|entry| entry.file_path.to_string_lossy().to_string())
        .collect();
    status
}

/// 扫描所有工程根，读文件头建立 db 文件清单（canonical 路径去重）。
fn collect_db_file_inventory(
    roots: &[(String, PathBuf)],
    warnings: &mut Vec<String>,
) -> Vec<DbFileEntry> {
    let mut entries = Vec::new();
    let mut seen_paths: BTreeSet<PathBuf> = BTreeSet::new();
    let mut visited = 0usize;
    for (project, root) in roots {
        if !root.exists() {
            warnings.push(format!("工程根不存在，已跳过: {}", root.display()));
            continue;
        }
        for entry in walkdir::WalkDir::new(root)
            .max_depth(SCAN_MAX_DEPTH)
            .into_iter()
            .filter_map(|e| e.ok())
        {
            if !entry.file_type().is_file() {
                continue;
            }
            visited += 1;
            if visited > SCAN_MAX_FILES {
                warnings.push(format!(
                    "工程路径扫描文件数超过 {SCAN_MAX_FILES} 上限，清单可能不完整"
                ));
                return entries;
            }
            let path = entry.path();
            if is_hidden_or_dotted(path) {
                continue;
            }
            let info = parse_db_basic_info(path.to_path_buf());
            if info.dbnum == 0 || info.db_type.trim().is_empty() {
                continue;
            }
            let canonical = std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
            if !seen_paths.insert(canonical.clone()) {
                continue;
            }
            entries.push(DbFileEntry {
                dbnum: info.dbnum,
                db_type: info.db_type.trim().to_ascii_uppercase(),
                ses_pgno: info.ses_pgno,
                file_name: path
                    .file_name()
                    .map(|s| s.to_string_lossy().to_string())
                    .unwrap_or_default(),
                file_path: canonical,
                project: project.clone(),
            });
        }
    }
    entries
}

/// 隐藏文件 / 带扩展名文件不是 E3D db 文件（与 sidecar 类型扫描口径一致）。
fn is_hidden_or_dotted(path: &Path) -> bool {
    path.file_name()
        .and_then(|name| name.to_str())
        .map(|name| name.starts_with('.') || name.contains('.'))
        .unwrap_or(true)
}

/// MDB 名称统一保留前导 `/`。
fn normalize_mdb_name(raw: &str) -> String {
    let trimmed = raw.trim();
    if trimmed.is_empty() || trimmed == "unset" {
        return String::new();
    }
    if trimmed.starts_with('/') {
        trimmed.to_string()
    } else {
        format!("/{trimmed}")
    }
}

/// SYST DB 元素 STYP -> 类型标签（与 team_data::match_stype 口径一致）。
fn stype_label(styp: Option<i32>) -> &'static str {
    match styp {
        Some(1) => "DESI",
        Some(2) => "CATA",
        Some(4) => "PROP",
        Some(6) => "ISOD",
        Some(7) => "PADD",
        Some(8) => "DICT",
        Some(9) => "ENGI",
        Some(14) => "SCHE",
        _ => "UNKNOWN",
    }
}

/// 暴露给 sidecar 的便捷入口：从 `SidecarSiteProject` 风格的 (name, path) 列表发现候选。
pub async fn discover_mdb_candidates_for_roots(
    named_roots: Vec<(String, String)>,
) -> Result<MdbCandidatesResult> {
    let roots: Vec<(String, PathBuf)> = named_roots
        .into_iter()
        .map(|(name, path)| (name, PathBuf::from(path)))
        .collect();
    Ok(discover_mdb_candidates(&roots).await)
}
