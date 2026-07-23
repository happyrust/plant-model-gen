use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use aios_core::pdms_types::{
    BRAN_COMPONENT_NOUN_NAMES, GNERAL_LOOP_OWNER_NOUN_NAMES, GNERAL_PRIM_NOUN_NAMES,
    TOTAL_CATA_GEO_NOUN_NAMES, TOTAL_LOOP_NOUN_NAMES, TOTAL_VERT_NOUN_NAMES, USE_CATE_NOUN_NAMES,
};
use aios_core::{NamedAttrValue, RefU64, RefnoEnum, project_primary_db};
use anyhow::Context;
use once_cell::sync::Lazy;
use parse_pdms_db::parse::EleData;
use pdms_io::PdmsIO;
use pdms_io::io::{EleOperationData, EleOperationDetail};
use serde::{Deserialize, Serialize};

use crate::data_interface::increment_record::IncrGeoUpdateLog;
use crate::versioned_db::model_gen_debt::{
    ModelGenDebtWriteOutcome, debt_upsert_sql, ensure_model_gen_debt_schema,
};
use crate::versioned_db::version_commit::{
    VersionCommitCounts, VersionCommitRequest, VersionCommitSource, commit_version,
    compute_commit_fingerprint, recover_version_commit,
};

static PRIM_NOUN_SET: Lazy<HashSet<&'static str>> =
    Lazy::new(|| GNERAL_PRIM_NOUN_NAMES.iter().copied().collect());

static LOOP_OWNER_NOUN_SET: Lazy<HashSet<&'static str>> =
    Lazy::new(|| GNERAL_LOOP_OWNER_NOUN_NAMES.iter().copied().collect());

static CATA_NOUN_SET: Lazy<HashSet<&'static str>> = Lazy::new(|| {
    TOTAL_CATA_GEO_NOUN_NAMES
        .iter()
        .chain(USE_CATE_NOUN_NAMES.iter())
        .chain(BRAN_COMPONENT_NOUN_NAMES.iter())
        .copied()
        .collect()
});

static LOOP_CONTAINER_NOUN_SET: Lazy<HashSet<&'static str>> = Lazy::new(|| {
    TOTAL_LOOP_NOUN_NAMES
        .iter()
        .chain(TOTAL_VERT_NOUN_NAMES.iter())
        .copied()
        .collect()
});

fn remove_refno_from_log(update_log: &mut IncrGeoUpdateLog, refno: RefnoEnum) {
    update_log.prim_refnos.remove(&refno);
    update_log.loop_owner_refnos.remove(&refno);
    update_log.bran_hanger_refnos.remove(&refno);
    update_log.basic_cata_refnos.remove(&refno);
    update_log.delete_refnos.remove(&refno);
}

fn insert_change_by_noun(
    update_log: &mut IncrGeoUpdateLog,
    refno: RefnoEnum,
    noun_or_type: &str,
    is_delete: bool,
) -> Option<&'static str> {
    remove_refno_from_log(update_log, refno);
    if is_delete {
        update_log.delete_refnos.insert(refno);
        return Some("delete");
    }

    let noun = noun_or_type.trim().to_ascii_uppercase();
    match noun.as_str() {
        "PRIM" => {
            update_log.prim_refnos.insert(refno);
            Some("prim")
        }
        "LOOP" => {
            update_log.loop_owner_refnos.insert(refno);
            Some("loop_owner")
        }
        "BRAN" | "HANG" | "HANGER" => {
            update_log.bran_hanger_refnos.insert(refno);
            Some("bran_hanger")
        }
        "CATA" => {
            update_log.basic_cata_refnos.insert(refno);
            Some("basic_cata")
        }
        noun if PRIM_NOUN_SET.contains(noun) => {
            update_log.prim_refnos.insert(refno);
            Some("prim")
        }
        noun if LOOP_OWNER_NOUN_SET.contains(noun) => {
            update_log.loop_owner_refnos.insert(refno);
            Some("loop_owner")
        }
        noun if CATA_NOUN_SET.contains(noun) => {
            update_log.basic_cata_refnos.insert(refno);
            Some("basic_cata")
        }
        _ => None,
    }
}

fn is_loop_container_noun(noun: &str) -> bool {
    LOOP_CONTAINER_NOUN_SET.contains(noun.trim().to_ascii_uppercase().as_str())
}

fn element_owner_refno(ele: &EleData) -> Option<RefnoEnum> {
    let owner = ele.att_map().get_owner();
    if !owner.is_unset() {
        return Some(owner);
    }

    if !ele.owner.is_unset() {
        return Some(RefnoEnum::from(ele.owner));
    }

    None
}

fn operation_owner_refno(operation: &EleOperationData) -> Option<RefnoEnum> {
    match &operation.detail {
        EleOperationDetail::Add(ele) => element_owner_refno(ele),
        EleOperationDetail::Modified(modified) => element_owner_refno(&modified.current_data),
        _ => None,
    }
}

fn resolve_non_container_owner(
    io: &mut PdmsIO,
    mut owner: Option<RefnoEnum>,
    sesno: u32,
) -> Option<(RefnoEnum, String)> {
    for _ in 0..6 {
        let owner_refno = owner?;
        if owner_refno.is_unset() {
            return None;
        }

        let Some((_, offset)) = io.search_latest_refno(owner_refno.refno(), Some(sesno)) else {
            return Some((owner_refno, String::new()));
        };
        let Ok(owner_ele) = io.parse_raw_element(offset) else {
            return Some((owner_refno, String::new()));
        };
        let owner_noun = owner_ele.att_map().get_type();
        if !is_loop_container_noun(&owner_noun) {
            return Some((owner_refno, owner_noun));
        }
        owner = element_owner_refno(&owner_ele);
    }

    owner.map(|refno| (refno, String::new()))
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PdmsSesnoIncrementFileReport {
    pub dbnum: u32,
    pub project: String,
    pub file_path: PathBuf,
    /// 在打开并采集源文件之前计算，用于 IncrementRun 写前/写后稳定性门禁。
    pub source_sha256_before: String,
    pub requested_start_sesno: u32,
    pub requested_end_sesno: u32,
    pub actual_start_sesno: u32,
    pub actual_end_sesno: u32,
    pub latest_sesno: u32,
    pub session_count: usize,
    pub element_count: usize,
    pub add_count: usize,
    pub modify_count: usize,
    pub delete_count: usize,
    pub none_count: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PdmsSesnoElementChange {
    pub dbnum: u32,
    pub project: String,
    pub file_path: PathBuf,
    pub sesno: u32,
    pub refno: RefnoEnum,
    pub operation: String,
    pub noun: String,
    pub owner_refno: Option<RefnoEnum>,
    #[serde(default)]
    pub changed_attributes: Vec<String>,
    pub impact_decision: String,
    pub impact_reason: String,
    pub classified: bool,
    pub model_category: Option<String>,
    pub model_refno: Option<RefnoEnum>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct PdmsSesnoIncrementOutcome {
    pub files: Vec<PdmsSesnoIncrementFileReport>,
    pub update_log: IncrGeoUpdateLog,
    pub element_changes: Vec<PdmsSesnoElementChange>,
}

#[derive(Debug, Clone)]
pub struct PdmsSesnoCollectedFile {
    pub report: PdmsSesnoIncrementFileReport,
    pub grouped_operations: BTreeMap<u32, Vec<EleOperationData>>,
    pub update_log: IncrGeoUpdateLog,
}

#[derive(Debug, Clone, Default)]
pub struct PdmsSesnoCollectedOutcome {
    pub outcome: PdmsSesnoIncrementOutcome,
    pub files: Vec<PdmsSesnoCollectedFile>,
}

/// specs/022：已写入 `sesno_version_anchor` 表的锚点记录（用于汇总 JSON 与日志）。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VersionAnchorRecord {
    pub dbnum: u32,
    pub sesno: u32,
    pub source: String,
    pub anchored_at: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fingerprint: Option<String>,
    #[serde(default)]
    pub idempotent: bool,
    #[serde(default)]
    pub recovered: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VersionCommitFailureRecord {
    pub dbnum: u32,
    pub from_sesno: u32,
    pub to_sesno: u32,
    pub error: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct PdmsIncrementPersistStats {
    pub file_count: usize,
    pub session_count: usize,
    pub upsert_count: usize,
    pub delete_count: usize,
    pub pe_rows: usize,
    pub att_rows: usize,
    pub uda_rows: usize,
    pub dbnum_info_updates: usize,
    /// specs/023：本批重写的 pe_owner 边行数（INSERT RELATION 行）
    #[serde(default)]
    pub pe_owner_rows: usize,
    /// specs/022：本批增量落库固化的 sesno 锚点（成功路径末尾写入）。
    #[serde(default)]
    pub anchors: Vec<VersionAnchorRecord>,
    /// 每个 dbnum 独立提交；多库命令保留已成功锚点并显式报告失败项。
    #[serde(default)]
    pub commit_failures: Vec<VersionCommitFailureRecord>,
    /// specs/026：本批在 commit apply 事务内幂等写入的模型欠账记录。debt 与数据
    /// 锚点同域提交，写失败即 commit 失败并计入 `commit_failures`，不再单列。
    #[serde(default)]
    pub debt_written: Vec<ModelGenDebtWriteOutcome>,
}

impl PdmsIncrementPersistStats {
    fn merge(&mut self, other: PdmsIncrementPersistStats) {
        self.file_count += other.file_count;
        self.session_count += other.session_count;
        self.upsert_count += other.upsert_count;
        self.delete_count += other.delete_count;
        self.pe_rows += other.pe_rows;
        self.att_rows += other.att_rows;
        self.uda_rows += other.uda_rows;
        self.dbnum_info_updates += other.dbnum_info_updates;
        self.pe_owner_rows += other.pe_owner_rows;
        self.anchors.extend(other.anchors);
        self.commit_failures.extend(other.commit_failures);
        self.debt_written.extend(other.debt_written);
    }
}

impl PdmsSesnoIncrementOutcome {
    pub fn merge(&mut self, other: PdmsSesnoIncrementOutcome) {
        self.update_log
            .prim_refnos
            .extend(other.update_log.prim_refnos);
        self.update_log
            .loop_owner_refnos
            .extend(other.update_log.loop_owner_refnos);
        self.update_log
            .bran_hanger_refnos
            .extend(other.update_log.bran_hanger_refnos);
        self.update_log
            .basic_cata_refnos
            .extend(other.update_log.basic_cata_refnos);
        self.update_log
            .delete_refnos
            .extend(other.update_log.delete_refnos);
        self.files.extend(other.files);
        self.element_changes.extend(other.element_changes);
    }

    pub fn total_session_count(&self) -> usize {
        self.files.iter().map(|file| file.session_count).sum()
    }

    pub fn total_element_count(&self) -> usize {
        self.files.iter().map(|file| file.element_count).sum()
    }
}

impl PdmsSesnoCollectedOutcome {
    pub fn merge(&mut self, other: PdmsSesnoCollectedOutcome) {
        self.outcome.merge(other.outcome);
        self.files.extend(other.files);
    }

    pub fn into_outcome(self) -> PdmsSesnoIncrementOutcome {
        self.outcome
    }
}

fn collected_outcome_for_file(
    report: PdmsSesnoIncrementFileReport,
    grouped_operations: BTreeMap<u32, Vec<EleOperationData>>,
    update_log: IncrGeoUpdateLog,
    element_changes: Vec<PdmsSesnoElementChange>,
) -> PdmsSesnoCollectedOutcome {
    let file_update_log = update_log.clone();
    PdmsSesnoCollectedOutcome {
        outcome: PdmsSesnoIncrementOutcome {
            files: vec![report.clone()],
            update_log,
            element_changes,
        },
        files: vec![PdmsSesnoCollectedFile {
            report,
            grouped_operations,
            update_log: file_update_log,
        }],
    }
}

fn valid_ref0_from_index_refno(refno: &RefU64) -> Option<u32> {
    let ref0 = refno.get_0();
    (ref0 != 0 && ref0 != 0x8000_0001).then_some(ref0)
}

fn collect_file_ref0s(project: &str, file_path: &Path) -> anyhow::Result<BTreeSet<u32>> {
    let mut io = PdmsIO::new(project, file_path, false);
    io.open()
        .with_context(|| format!("PdmsIO::open 失败: {}", file_path.display()))?;
    let index_map = io
        .build_index_map()
        .with_context(|| format!("build_index_map 失败: {}", file_path.display()))?;
    Ok(index_map
        .keys()
        .filter_map(valid_ref0_from_index_refno)
        .collect())
}

fn inactive_db_path(path: &Path) -> bool {
    let inactive_component = path.components().any(|component| {
        matches!(
            component
                .as_os_str()
                .to_string_lossy()
                .to_ascii_lowercase()
                .as_str(),
            "back" | "backup" | "cbas"
        )
    });
    if inactive_component {
        return true;
    }

    path.file_name()
        .and_then(|name| name.to_str())
        .map(|name| {
            let lower = name.to_ascii_lowercase();
            lower.ends_with("_old")
                || lower.ends_with("-old")
                || lower.ends_with(".old")
                || lower.contains("_old.")
                || lower.contains("-old.")
                || lower.contains(" copy")
                || lower.contains("_copy")
                || lower.contains("-copy")
                || lower.ends_with("_new")
                || lower.ends_with("-new")
                || lower.ends_with(".new")
                || lower.contains("_new.")
                || lower.contains("-new.")
                || lower.ends_with("_test")
                || lower.ends_with("-test")
                || lower.ends_with(".test")
                || lower.contains("_test.")
                || lower.contains("-test.")
        })
        .unwrap_or(false)
}

fn file_modified(path: &Path) -> SystemTime {
    std::fs::metadata(path)
        .and_then(|metadata| metadata.modified())
        .unwrap_or(SystemTime::UNIX_EPOCH)
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

#[cfg(feature = "sqlite-index")]
fn discover_active_db_file_for_dbnum(dbnum: u32) -> anyhow::Result<Option<(String, PathBuf)>> {
    use parse_pdms_db::parse::parse_db_basic_info;

    let db_option = crate::data_interface::db_index::load_db_option_from_env()?;
    let roots = crate::data_interface::db_index::derive_project_roots(&db_option)?;
    let mut candidates: Vec<(String, PathBuf)> = Vec::new();

    for (project, root) in roots {
        for entry in walkdir::WalkDir::new(&root)
            .max_depth(8)
            .into_iter()
            .filter_map(|entry| entry.ok())
        {
            if !entry.file_type().is_file() {
                continue;
            }
            let path = entry.path();
            let info = parse_db_basic_info(path.to_path_buf());
            if info.dbnum == dbnum {
                candidates.push((project.clone(), path.to_path_buf()));
            }
        }
    }

    candidates.sort_by(|(_, a), (_, b)| {
        let a_inactive = inactive_db_path(a);
        let b_inactive = inactive_db_path(b);
        a_inactive
            .cmp(&b_inactive)
            .then_with(|| db_candidate_rank(a).cmp(&db_candidate_rank(b)))
            .then_with(|| file_modified(b).cmp(&file_modified(a)))
            .then_with(|| a.to_string_lossy().cmp(&b.to_string_lossy()))
    });

    Ok(candidates.into_iter().next())
}

/// 将本次 pdms-io 增量文件的 ref0/dbnum 信息刷新到 db_meta_info.json。
///
/// 这一步补齐增量生成所需的 ref0->dbnum 映射，避免只靠旧解析期产物时
/// 新库/样例库缺映射导致 CATE 阶段跳过。
pub fn refresh_db_meta_for_increment_files(
    project: &str,
    files: &[PdmsSesnoIncrementFileReport],
) -> anyhow::Result<usize> {
    use parse_pdms_db::parse::parse_db_basic_info;

    let tree_dir = crate::versioned_db::db_meta_info::get_project_tree_dir(project);
    let mut refreshed = 0usize;
    let mut seen = HashSet::new();

    for file in files {
        if file.dbnum == 0 || !seen.insert((file.dbnum, file.file_path.clone())) {
            continue;
        }
        let info = parse_db_basic_info(file.file_path.clone());
        if info.dbnum == 0 {
            continue;
        }
        let ref0s = collect_file_ref0s(project, &file.file_path).with_context(|| {
            format!(
                "收集 db_meta ref0 失败: dbnum={} file={}",
                file.dbnum,
                file.file_path.display()
            )
        })?;
        let file_name = file
            .file_path
            .file_name()
            .map(|name| name.to_string_lossy().to_string())
            .unwrap_or_default();
        crate::versioned_db::db_meta_info::update_db_meta_info_json(
            &tree_dir,
            crate::versioned_db::db_meta_info::DbFileMetaUpdate {
                dbnum: info.dbnum,
                db_type: &info.db_type,
                file_name: &file_name,
                file_path: &file.file_path,
                header_hex_60: None,
                header_debug: None,
                latest_sesno: Some(file.latest_sesno),
                sesno_timestamp: None,
                ref0s,
            },
        )?;
        refreshed += 1;
    }

    if refreshed > 0 {
        let meta_path = tree_dir.join("db_meta_info.json");
        crate::data_interface::db_meta_manager::db_meta().load(&meta_path)?;
    }

    Ok(refreshed)
}

#[derive(Debug, Clone, Copy)]
struct PdmsModelChangeTarget {
    refno: RefnoEnum,
    category: &'static str,
}

fn operation_kind(operation: &EleOperationData) -> &'static str {
    match &operation.detail {
        EleOperationDetail::Add(_) => "add",
        EleOperationDetail::Modified(_) => "modify",
        EleOperationDetail::Deleted => "delete",
        EleOperationDetail::None => "none",
    }
}

#[derive(Debug, Clone)]
struct OperationModelImpact {
    decision: &'static str,
    reason: &'static str,
    changed_attributes: Vec<String>,
}

impl OperationModelImpact {
    fn triggers_model(&self) -> bool {
        self.decision != "neutral"
    }
}

fn collect_modified_attribute_names(modified: &pdms_io::io::ModifiedElement) -> Vec<String> {
    let mut names = modified
        .added_attrs
        .keys()
        .chain(modified.deleted_attrs.keys())
        .chain(modified.modified_attrs.keys())
        .chain(modified.added_explicit_attrs.keys())
        .chain(modified.deleted_explicit_attrs.keys())
        .chain(modified.modified_explicit_attrs.keys())
        .map(|name| crate::version_management::model_impact::normalize_attribute_name(name))
        .filter(|name| !name.is_empty())
        .collect::<Vec<_>>();
    names.extend(
        modified
            .added_uda_attrs
            .keys()
            .chain(modified.deleted_uda_attrs.keys())
            .chain(modified.modified_uda_attrs.keys())
            .map(|id| format!("UDA:{id}")),
    );
    names.sort();
    names.dedup();
    names
}

fn classify_modified_element(
    modified: &pdms_io::io::ModifiedElement,
    model_impact_filter: bool,
) -> OperationModelImpact {
    use crate::version_management::model_impact::{
        AttributeModelImpact, classify_attribute_model_impact,
    };

    let changed_attributes = collect_modified_attribute_names(modified);
    if modified.children_changed.is_some() {
        return OperationModelImpact {
            decision: "trigger",
            reason: "children_changed",
            changed_attributes,
        };
    }
    if changed_attributes
        .iter()
        .any(|name| matches!(name.as_str(), "OWNER"))
    {
        return OperationModelImpact {
            decision: "trigger",
            reason: "owner_changed",
            changed_attributes,
        };
    }
    if changed_attributes
        .iter()
        .any(|name| matches!(name.as_str(), "NOUN" | "TYPE"))
    {
        return OperationModelImpact {
            decision: "trigger",
            reason: "noun_changed",
            changed_attributes,
        };
    }
    if !model_impact_filter {
        return OperationModelImpact {
            decision: "trigger",
            reason: "model_impact_filter_disabled",
            changed_attributes,
        };
    }
    if changed_attributes
        .iter()
        .any(|name| name.starts_with("UDA:"))
    {
        return OperationModelImpact {
            decision: "unknown_fallback",
            reason: "unknown_uda",
            changed_attributes,
        };
    }

    let mut has_model_input = false;
    let mut has_unknown = changed_attributes.is_empty();
    for name in &changed_attributes {
        match classify_attribute_model_impact(name) {
            AttributeModelImpact::AffectsModel => has_model_input = true,
            AttributeModelImpact::KnownNeutral => {}
            AttributeModelImpact::Unknown => has_unknown = true,
        }
    }
    let (decision, reason) = if has_model_input {
        ("trigger", "geometry_attribute")
    } else if has_unknown {
        ("unknown_fallback", "unknown_attribute")
    } else {
        ("neutral", "known_neutral")
    };
    OperationModelImpact {
        decision,
        reason,
        changed_attributes,
    }
}

fn classify_operation(
    operation: &EleOperationData,
    model_impact_filter: bool,
) -> OperationModelImpact {
    match &operation.detail {
        EleOperationDetail::Add(_) => OperationModelImpact {
            decision: "trigger",
            reason: "added",
            changed_attributes: Vec::new(),
        },
        EleOperationDetail::Deleted => OperationModelImpact {
            decision: "trigger",
            reason: "deleted",
            changed_attributes: Vec::new(),
        },
        EleOperationDetail::Modified(modified) => {
            classify_modified_element(modified, model_impact_filter)
        }
        EleOperationDetail::None => OperationModelImpact {
            decision: "neutral",
            reason: "none_operation",
            changed_attributes: Vec::new(),
        },
    }
}

fn apply_pdms_operation(
    io: &mut PdmsIO,
    update_log: &mut IncrGeoUpdateLog,
    operation: &EleOperationData,
    impact: &OperationModelImpact,
) -> Option<PdmsModelChangeTarget> {
    let refno = RefnoEnum::from(operation.refno);
    match &operation.detail {
        EleOperationDetail::Deleted => insert_change_by_noun(update_log, refno, "DELETED", true)
            .map(|category| PdmsModelChangeTarget { refno, category }),
        EleOperationDetail::None => None,
        EleOperationDetail::Modified(_) if !impact.triggers_model() => None,
        _ => {
            let noun = operation.get_noun_type();
            if is_loop_container_noun(&noun) {
                if let Some((owner_refno, owner_noun)) = resolve_non_container_owner(
                    io,
                    operation_owner_refno(operation),
                    operation.sesno,
                ) {
                    if owner_noun.is_empty() {
                        update_log.loop_owner_refnos.insert(owner_refno);
                        return Some(PdmsModelChangeTarget {
                            refno: owner_refno,
                            category: "loop_owner",
                        });
                    }
                    return insert_change_by_noun(update_log, owner_refno, &owner_noun, false).map(
                        |category| PdmsModelChangeTarget {
                            refno: owner_refno,
                            category,
                        },
                    );
                }
                return None;
            }
            match insert_change_by_noun(update_log, refno, &noun, false) {
                Some(category) => Some(PdmsModelChangeTarget { refno, category }),
                None if impact
                    .changed_attributes
                    .iter()
                    .any(|name| matches!(name.as_str(), "NOUN" | "TYPE")) =>
                {
                    insert_change_by_noun(update_log, refno, "DELETED", true)
                        .map(|category| PdmsModelChangeTarget { refno, category })
                }
                None => None,
            }
        }
    }
}

fn named_attr_refno(value: &NamedAttrValue) -> Option<RefnoEnum> {
    match value {
        NamedAttrValue::RefU64Type(value) => Some(RefnoEnum::from(*value)),
        NamedAttrValue::RefnoEnumType(value) => Some(*value),
        _ => None,
    }
    .filter(RefnoEnum::is_valid)
}

fn insert_existing_model_target(
    io: &mut PdmsIO,
    update_log: &mut IncrGeoUpdateLog,
    refno: RefnoEnum,
    sesno: u32,
) {
    let Some((_, offset)) = io.search_latest_refno(refno.refno(), Some(sesno)) else {
        return;
    };
    let Ok(ele) = io.parse_raw_element(offset) else {
        return;
    };
    let noun = ele.att_map().get_type();
    if is_loop_container_noun(&noun) {
        if let Some((owner_refno, owner_noun)) =
            resolve_non_container_owner(io, element_owner_refno(&ele), sesno)
        {
            if owner_noun.is_empty() {
                update_log.loop_owner_refnos.insert(owner_refno);
            } else {
                insert_change_by_noun(update_log, owner_refno, &owner_noun, false);
            }
        }
    } else {
        insert_change_by_noun(update_log, refno, &noun, false);
    }
}

fn apply_critical_model_expansion(
    io: &mut PdmsIO,
    update_log: &mut IncrGeoUpdateLog,
    operation: &EleOperationData,
    impact: &OperationModelImpact,
) {
    let EleOperationDetail::Modified(modified) = &operation.detail else {
        return;
    };
    if !impact.triggers_model() {
        return;
    }

    if impact.changed_attributes.iter().any(|name| name == "OWNER") {
        for (name, (old, new)) in &modified.modified_attrs {
            if crate::version_management::model_impact::normalize_attribute_name(name) == "OWNER" {
                if let Some(old_owner) = named_attr_refno(old) {
                    insert_existing_model_target(
                        io,
                        update_log,
                        old_owner,
                        operation.sesno.saturating_sub(1),
                    );
                }
                if let Some(new_owner) = named_attr_refno(new) {
                    insert_existing_model_target(io, update_log, new_owner, operation.sesno);
                }
            }
        }
        for (name, value) in &modified.deleted_attrs {
            if crate::version_management::model_impact::normalize_attribute_name(name) == "OWNER"
                && let Some(old_owner) = named_attr_refno(value)
            {
                insert_existing_model_target(
                    io,
                    update_log,
                    old_owner,
                    operation.sesno.saturating_sub(1),
                );
            }
        }
        if let Some(new_owner) = operation_owner_refno(operation) {
            insert_existing_model_target(io, update_log, new_owner, operation.sesno);
        }
    }

    if let Some((old_children, new_children)) = &modified.children_changed {
        let old = old_children.0.iter().copied().collect::<HashSet<_>>();
        let new = new_children.0.iter().copied().collect::<HashSet<_>>();
        for child in old.difference(&new) {
            insert_existing_model_target(
                io,
                update_log,
                RefnoEnum::from(*child),
                operation.sesno.saturating_sub(1),
            );
        }
        for child in new.difference(&old) {
            insert_existing_model_target(io, update_log, RefnoEnum::from(*child), operation.sesno);
        }
    }
}

fn current_ele_for_persist(operation: &EleOperationData) -> Option<&EleData> {
    match &operation.detail {
        EleOperationDetail::Add(ele) => Some(ele),
        EleOperationDetail::Modified(modified) => Some(&modified.current_data),
        _ => None,
    }
}

fn inject_children_into_pe_json(mut json: String, children: &[RefU64]) -> String {
    let children_links = children
        .iter()
        .map(|child| child.to_pe_key())
        .collect::<Vec<_>>()
        .join(", ");
    if json.ends_with('}') {
        json.pop();
        let sep = if json.contains(':') { ", " } else { "" };
        json.push_str(&format!("{sep}children: [{children_links}]}}"));
    }
    json
}

/// specs/023 M0/T2（D1 方案 A）：把 cata_hash 注入 pe 行 JSON（string 存储，
/// u64 可能超出 Surreal int/i64 范围）。增量 UPSERT CONTENT 是整行替换——
/// 若不在此注入，一次 modify 就会抹掉 full 解析/回填写入的 cata_hash。
fn inject_cata_hash_into_pe_json(mut json: String, cata_hash: Option<u64>) -> String {
    let Some(hash) = cata_hash else {
        return json;
    };
    if json.ends_with('}') {
        json.pop();
        let sep = if json.contains(':') { ", " } else { "" };
        json.push_str(&format!("{sep}cata_hash: '{hash}'}}"));
    }
    json
}

fn record_target(table: &str, refno: RefU64) -> String {
    format!("{}:{}", table, refno)
}

async fn exec_statements(sqls: &[String], chunk_size: usize) -> anyhow::Result<()> {
    for chunk in sqls.chunks(chunk_size.max(1)) {
        let sql = chunk.join("\n");
        project_primary_db().query(sql).await?.check()?;
    }
    Ok(())
}

#[derive(Debug, Clone, Copy, Default)]
struct Ref0PersistInfo {
    dbnum: i32,
    max_sesno: i32,
    max_ref1: u64,
}

async fn build_increment_delete_statements(refno: RefU64) -> anyhow::Result<Vec<String>> {
    let pe_key = refno.to_pe_key();
    let mut response = project_primary_db()
        .query(format!("SELECT VALUE noun FROM {pe_key} LIMIT 1;"))
        .await?
        .check()?;
    let noun: Option<String> = response.take(0)?;

    let mut sqls = Vec::new();
    if let Some(noun) = noun.filter(|noun| !noun.trim().is_empty()) {
        sqls.push(format!("DELETE {};", record_target(&noun, refno)));
    }
    sqls.push(format!("DELETE {};", record_target("ATT_UDA", refno)));
    sqls.push(format!("DELETE {pe_key};"));
    Ok(sqls)
}

async fn persist_pdms_increment_file(
    report: &PdmsSesnoIncrementFileReport,
    detail: bool,
) -> anyhow::Result<PdmsIncrementPersistStats> {
    if report.actual_start_sesno == 0 || report.actual_end_sesno == 0 {
        return Ok(PdmsIncrementPersistStats {
            file_count: 1,
            ..Default::default()
        });
    }

    let mut io = PdmsIO::new(&report.project, &report.file_path, detail);
    io.open()
        .with_context(|| format!("PdmsIO::open 失败: {}", report.file_path.display()))?;
    let grouped = io
        .collect_increment_eles(Some(
            report.actual_start_sesno as i32..=report.actual_end_sesno as i32,
        ))
        .with_context(|| {
            format!(
                "收集 PDMS 增量落库数据失败: dbnum={} sesno={}..={} file={}",
                report.dbnum,
                report.actual_start_sesno,
                report.actual_end_sesno,
                report.file_path.display()
            )
        })?;

    persist_pdms_increment_grouped(report, &grouped, None, false, None).await
}

async fn persist_pdms_increment_grouped(
    report: &PdmsSesnoIncrementFileReport,
    grouped: &BTreeMap<u32, Vec<EleOperationData>>,
    source_hash: Option<&str>,
    recover_pending: bool,
    update_log: Option<&IncrGeoUpdateLog>,
) -> anyhow::Result<PdmsIncrementPersistStats> {
    let mut stats = PdmsIncrementPersistStats {
        file_count: 1,
        ..Default::default()
    };
    if report.actual_start_sesno == 0 || report.actual_end_sesno == 0 {
        return Ok(stats);
    }

    stats.session_count = grouped.len();

    // ADR-0011 P1：反向索引 schema 代码内 ensure（幂等 DDL，非版本化数据；覆盖所有入口路径）。
    crate::versioned_db::cata_ref_index::ensure_cata_ref_index_schema().await?;

    // Keep mutations in sesno order. The previous implementation buffered PE/ATT
    // separately while executing deletes immediately, so add→delete in one range
    // could be replayed as delete→add and expose the wrong final state.
    let mut mutation_sqls = Vec::new();
    let mut dbnum_info: BTreeMap<u64, Ref0PersistInfo> = BTreeMap::new();

    // specs/023 T004：层级边维护。按 op 顺序收集每个被触及 owner 的**最终**子列表
    // （同批多次触及同一 owner 时 last-wins；每次重写都是全量替换，故语义精确），
    // op 循环结束后统一生成"先全删、后全插"两段语句，且删/插分属**不同请求**提交
    // （见 edge_delete_sqls/edge_insert_sqls 的 apply 顺序）。不与 PE/ATT 语句混排的原因：
    // 同一请求内"删边→重插同 (in,out)"在 versioned 引擎上撞过 unique_pe_owner 唯一索引
    // （rebuild CLI 实测，孤立复现不稳定）；跨请求提交后删除必然可见。
    // 注意：删除元素（DELETE pe:x）时引擎会自动清除其两侧关联边（8030 实测），
    // 这里对被删元素仍登记空重写（owner 侧显式清边），兜底"pe 行已缺失但残留边"的脏数据。
    let mut edge_final: indexmap::IndexMap<RefU64, Vec<RefU64>> = indexmap::IndexMap::new();

    // ADR-0011 P1 step-2：目录引用反向索引 replace-by-source。与 PE/ATT/debt 同处
    // commit_version apply 保护域，但（仿 debt）**不进数据 fingerprint**——索引是附属产物、
    // 不构成数据版本身份。收集本轮被触及的 source（含删除）以整体先删，再插 add/modified 新出边。
    let mut crx_touched_sources: Vec<String> = Vec::new();
    let mut crx_new_edges: Vec<crate::versioned_db::cata_ref_index::RefEdge> = Vec::new();

    for operations in grouped.values() {
        for operation in operations {
            if matches!(&operation.detail, EleOperationDetail::Deleted) {
                stats.delete_count += 1;
                let deleted_sqls = build_increment_delete_statements(operation.refno).await?;
                stats.att_rows += deleted_sqls.len().saturating_sub(1);
                mutation_sqls.extend(deleted_sqls);
                edge_final.insert(operation.refno, Vec::new());
                // 删除元素：清其反向索引出边（不级联删指向它的入边——被删 SCOM 仍可反查引用者）。
                crx_touched_sources.push(operation.refno.to_string());
                continue;
            }

            let Some(ele) = current_ele_for_persist(operation) else {
                continue;
            };
            let mut att = ele.att_map().clone();
            att.map.insert(
                "DBNUM".to_string(),
                aios_core::NamedAttrValue::IntegerType(report.dbnum as i32),
            );
            att.map.insert(
                "dbnum".to_string(),
                aios_core::NamedAttrValue::IntegerType(report.dbnum as i32),
            );
            let refno = operation.refno;
            // ADR-0011 P1：从最终 att 抽 as-written 引用边（除 OWNER/层级/自身轴），与 pe/att 同批。
            // changed source 一律 replace-by-source（过替换无害：重写同一批边）。
            crx_touched_sources.push(refno.to_string());
            crx_new_edges.extend(crate::versioned_db::cata_ref_index::extract_ref_edges(
                refno,
                report.dbnum,
                &att,
            ));
            let pe_data = att.pe(report.dbnum as i32);
            let pe_json = inject_children_into_pe_json(
                pe_data.gen_sur_json(Some(refno.to_pe_key())),
                ele.children.as_slice(),
            );
            let pe_json = inject_cata_hash_into_pe_json(pe_json, att.cal_cata_hash());
            mutation_sqls.push(format!("UPSERT {} CONTENT {};", refno.to_pe_key(), pe_json));
            stats.pe_rows += 1;
            stats.upsert_count += 1;

            let type_name = att.get_type_str().to_string();
            if !type_name.is_empty() {
                if let Some(json) = att.gen_sur_json_exclude(&["id"], None) {
                    mutation_sqls.push(format!(
                        "UPSERT {} CONTENT {};",
                        record_target(&type_name, refno),
                        json
                    ));
                    stats.att_rows += 1;
                }
                if let Some(json) = att.gen_sur_json_uda(&["id"]) {
                    mutation_sqls.push(format!(
                        "UPSERT {} CONTENT {};",
                        record_target("ATT_UDA", refno),
                        aios_core::helper::normalize_sql_string(&json)
                    ));
                    stats.uda_rows += 1;
                } else {
                    // Removing the last UDA is a state change too; leaving the old
                    // row would make current and historical snapshots disagree.
                    mutation_sqls.push(format!("DELETE {};", record_target("ATT_UDA", refno)));
                }
            }

            // specs/023 T004：登记该元素的最终子列表（op 自带 children 全量；
            // PDMS 语义：子列表变化时 owner 必以 Add/Modified 出现在 op 流）。
            edge_final.insert(refno, ele.children.clone().0);

            let refno_u64 = refno.0;
            let ref_0 = (refno_u64 >> 32) as u64;
            let ref_1 = refno_u64 & 0xFFFF_FFFF;
            dbnum_info
                .entry(ref_0)
                .and_modify(|info| {
                    info.max_sesno = info.max_sesno.max(operation.sesno as i32);
                    info.max_ref1 = info.max_ref1.max(ref_1);
                })
                .or_insert(Ref0PersistInfo {
                    dbnum: report.dbnum as i32,
                    max_sesno: operation.sesno as i32,
                    max_ref1: ref_1,
                });
        }
    }

    // specs/023 T004：由 edge_final 生成两段边语句——先删段与后插段。
    // 幂等策略=先删后插（research.md Decision 2：撞 id 且值不同会报错，
    // `INSERT IGNORE RELATION` 语法不存在）；禁止改用 id 区间扫（Decision 1/C3）。
    const PE_OWNER_RELATION_CHUNK: usize = 500;
    let mut edge_delete_sqls: Vec<String> = Vec::with_capacity(edge_final.len());
    let mut edge_insert_sqls: Vec<String> = Vec::new();
    for (owner, children) in &edge_final {
        let owner_key = owner.to_pe_key();
        edge_delete_sqls.push(format!("DELETE {owner_key}<-pe_owner;"));
        if children.is_empty() {
            continue;
        }
        stats.pe_owner_rows += children.len();
        for (chunk_idx, chunk) in children.chunks(PE_OWNER_RELATION_CHUNK).enumerate() {
            let rows = chunk
                .iter()
                .enumerate()
                .map(|(i, child)| {
                    let order = chunk_idx * PE_OWNER_RELATION_CHUNK + i;
                    format!(
                        "{{ id: pe_owner:[{owner_key}, {order}], in: {}, out: {owner_key} }}",
                        child.to_pe_key()
                    )
                })
                .collect::<Vec<_>>()
                .join(",");
            edge_insert_sqls.push(format!("INSERT RELATION INTO pe_owner [{rows}];"));
        }
    }

    let file_name = report
        .file_path
        .file_name()
        .map(|name| name.to_string_lossy().replace('\'', "\\'"))
        .unwrap_or_default();
    let mut dbnum_sqls = Vec::new();
    for (ref_0, info) in dbnum_info {
        dbnum_sqls.push(format!(
            "UPSERT dbnum_info_table:{} SET dbnum = {}, sesno = math::max([sesno?:0, {}]), max_ref1 = math::max([max_ref1?:0, {}]), file_name = '{}';",
            ref_0, info.dbnum, info.max_sesno, info.max_ref1, file_name
        ));
    }
    stats.dbnum_info_updates = dbnum_sqls.len();

    // ADR-0011 P1：把本轮 changed source 的反向索引 replace-by-source 拆成先删段（按 source
    // 分块、含删除元素）与后插段（add/modified 新出边），分属不同 exec 请求提交
    // （同请求删+重插同 id 可能撞约束，见 pe_owner 先例）。
    let mut crx_delete_sqls: Vec<String> = Vec::new();
    for chunk in crx_touched_sources.chunks(500) {
        if let Some(sql) =
            crate::versioned_db::cata_ref_index::delete_sources_sql(report.dbnum, chunk)
        {
            crx_delete_sqls.push(sql);
        }
    }
    let crx_insert_sqls = crate::versioned_db::cata_ref_index::insert_edges_sql(&crx_new_edges, 500);

    let fingerprint = compute_commit_fingerprint(
        report.dbnum,
        report.actual_start_sesno,
        report.actual_end_sesno,
        VersionCommitSource::Incremental,
        source_hash,
        mutation_sqls
            .iter()
            .chain(edge_delete_sqls.iter())
            .chain(edge_insert_sqls.iter())
            .chain(dbnum_sqls.iter())
            .map(String::as_str),
    );
    let commit_counts = VersionCommitCounts {
        pe_rows: stats.pe_rows,
        att_rows: stats.att_rows,
        uda_rows: stats.uda_rows,
        delete_count: stats.delete_count,
        dbnum_info_updates: stats.dbnum_info_updates,
        pe_owner_rows: stats.pe_owner_rows,
    };
    // debt 与数据锚点同处 apply 保护域：数据 fingerprint 不含 debt（debt 是附属产物，
    // 不构成数据版本身份），但 debt 的 UPSERT 与数据写入在同一 commit_version 保护域内。
    let has_debt = update_log.is_some();
    let debt_sql = update_log.map(|log| {
        debt_upsert_sql(
            report.dbnum,
            report.actual_start_sesno,
            report.actual_end_sesno,
            &fingerprint,
            log,
        )
    });
    let request = VersionCommitRequest {
        dbnum: report.dbnum,
        from_sesno: report.actual_start_sesno,
        to_sesno: report.actual_end_sesno,
        source: VersionCommitSource::Incremental,
        fingerprint,
        source_hash: source_hash.map(str::to_string),
        expected_counts: Some(commit_counts.clone()),
    };
    // apply 全成功才创建锚点、转 committed；任一步（含 debt）失败留 pending，
    // recover 重放同一 apply（数据 UPSERT / 边先删后插 / debt UPSERT 均幂等）。
    // 由此消除"数据锚点已提交、debt 未写"导致模型欠账永久丢失、只能整库 full-regen 的窗口。
    let apply = || async move {
        exec_statements(&mutation_sqls, 200).await?;
        // 边先删段与后插段分属不同 exec 调用（不同请求），保证删除对插入可见
        exec_statements(&edge_delete_sqls, 200).await?;
        exec_statements(&edge_insert_sqls, 200).await?;
        exec_statements(&dbnum_sqls, 200).await?;
        if let Some(debt_sql) = debt_sql.as_deref() {
            project_primary_db().query(debt_sql).await?.check()?;
        }
        // ADR-0011 P1：反向索引 replace-by-source（先删后插分属不同请求；不进 fingerprint）。
        // 失败与数据同处 pending，recover 重放同一 apply 幂等收敛。
        exec_statements(&crx_delete_sqls, 200).await?;
        exec_statements(&crx_insert_sqls, 200).await?;
        Ok(commit_counts)
    };
    let outcome = if recover_pending {
        recover_version_commit(request, apply).await?
    } else {
        commit_version(request, apply).await?
    };
    if has_debt {
        stats.debt_written.push(ModelGenDebtWriteOutcome {
            dbnum: outcome.dbnum,
            from_sesno: outcome.from_sesno,
            to_sesno: outcome.to_sesno,
            idempotent: outcome.idempotent,
        });
    }
    stats.anchors.push(VersionAnchorRecord {
        dbnum: outcome.dbnum,
        sesno: outcome.to_sesno,
        source: "incremental".to_string(),
        anchored_at: Some(outcome.anchored_at),
        fingerprint: Some(outcome.fingerprint),
        idempotent: outcome.idempotent,
        recovered: outcome.recovered,
    });

    Ok(stats)
}

pub async fn persist_pdms_increment_files(
    files: &[PdmsSesnoIncrementFileReport],
    detail: bool,
) -> anyhow::Result<PdmsIncrementPersistStats> {
    let mut stats = PdmsIncrementPersistStats::default();
    for file in files {
        stats.merge(persist_pdms_increment_file(file, detail).await?);
    }
    Ok(stats)
}

pub async fn persist_collected_pdms_increment_files(
    files: &[PdmsSesnoCollectedFile],
    recover_pending: bool,
) -> anyhow::Result<PdmsIncrementPersistStats> {
    // debt 现在于各 file 的 commit apply 事务内写入，schema 在此统一 ensure 一次。
    ensure_model_gen_debt_schema().await?;
    let mut stats = PdmsIncrementPersistStats::default();
    for file in files {
        match persist_pdms_increment_grouped(
            &file.report,
            &file.grouped_operations,
            Some(file.report.source_sha256_before.as_str()),
            recover_pending,
            Some(&file.update_log),
        )
        .await
        {
            Ok(file_stats) => stats.merge(file_stats),
            Err(error) => {
                stats.file_count += 1;
                stats.commit_failures.push(VersionCommitFailureRecord {
                    dbnum: file.report.dbnum,
                    from_sesno: file.report.actual_start_sesno,
                    to_sesno: file.report.actual_end_sesno,
                    error: error.to_string(),
                });
            }
        }
    }
    Ok(stats)
}

/// 使用 pdms-io 直接从一个 E3D/PDMS db 文件按 sesno 范围收集增量。
///
/// `cached_sesno` 表示当前系统已解析/缓存到的版本；实际读取范围从
/// `cached_sesno + 1` 到 `to_sesno`（省略则使用文件最新 sesno）。
/// 该函数只读源文件并构造模型增量日志，不写 SurrealDB/SQLite。
pub fn collect_pdms_increment_for_file_with_operations_options(
    project: &str,
    file_path: impl AsRef<Path>,
    cached_sesno: u32,
    to_sesno: Option<u32>,
    detail: bool,
    model_impact_filter: bool,
) -> anyhow::Result<PdmsSesnoCollectedOutcome> {
    let file_path = file_path.as_ref();
    let source_sha256_before = crate::version_management::hashing::sha256_file(file_path)
        .with_context(|| format!("计算增量源文件采集前 hash 失败: {}", file_path.display()))?;
    let mut io = PdmsIO::new(project, file_path, detail);
    io.open()
        .with_context(|| format!("PdmsIO::open 失败: {}", file_path.display()))?;

    let header = io
        .read_pdms_header()
        .with_context(|| format!("读取 PDMS header 失败: {}", file_path.display()))?;
    let dbnum = u32::try_from(header.db_num).unwrap_or_default();
    let latest_sesno = io.get_latest_sesno().with_context(|| {
        format!(
            "读取最新 sesno 失败: dbnum={} file={}",
            dbnum,
            file_path.display()
        )
    })?;
    let requested_start = cached_sesno.saturating_add(1);
    let requested_end = to_sesno.unwrap_or(latest_sesno);

    if requested_end > latest_sesno {
        anyhow::bail!(
            "目标 sesno {} 超过文件最新 sesno {}: {}",
            requested_end,
            latest_sesno,
            file_path.display()
        );
    }

    if requested_start > requested_end {
        let report = PdmsSesnoIncrementFileReport {
            dbnum,
            project: project.to_string(),
            file_path: file_path.to_path_buf(),
            source_sha256_before: source_sha256_before.clone(),
            requested_start_sesno: requested_start,
            requested_end_sesno: requested_end,
            actual_start_sesno: 0,
            actual_end_sesno: 0,
            latest_sesno,
            session_count: 0,
            element_count: 0,
            add_count: 0,
            modify_count: 0,
            delete_count: 0,
            none_count: 0,
        };
        return Ok(collected_outcome_for_file(
            report,
            BTreeMap::new(),
            IncrGeoUpdateLog::default(),
            Vec::new(),
        ));
    }

    let Some(actual_start) = io.get_nearest_large_sesno(requested_start as i32) else {
        let report = PdmsSesnoIncrementFileReport {
            dbnum,
            project: project.to_string(),
            file_path: file_path.to_path_buf(),
            source_sha256_before: source_sha256_before.clone(),
            requested_start_sesno: requested_start,
            requested_end_sesno: requested_end,
            actual_start_sesno: 0,
            actual_end_sesno: 0,
            latest_sesno,
            session_count: 0,
            element_count: 0,
            add_count: 0,
            modify_count: 0,
            delete_count: 0,
            none_count: 0,
        };
        return Ok(collected_outcome_for_file(
            report,
            BTreeMap::new(),
            IncrGeoUpdateLog::default(),
            Vec::new(),
        ));
    };
    let Some(actual_end) = io.get_nearest_less_sesno((requested_end as i32).saturating_add(1))
    else {
        let report = PdmsSesnoIncrementFileReport {
            dbnum,
            project: project.to_string(),
            file_path: file_path.to_path_buf(),
            source_sha256_before: source_sha256_before.clone(),
            requested_start_sesno: requested_start,
            requested_end_sesno: requested_end,
            actual_start_sesno: 0,
            actual_end_sesno: 0,
            latest_sesno,
            session_count: 0,
            element_count: 0,
            add_count: 0,
            modify_count: 0,
            delete_count: 0,
            none_count: 0,
        };
        return Ok(collected_outcome_for_file(
            report,
            BTreeMap::new(),
            IncrGeoUpdateLog::default(),
            Vec::new(),
        ));
    };

    if actual_start > actual_end {
        let report = PdmsSesnoIncrementFileReport {
            dbnum,
            project: project.to_string(),
            file_path: file_path.to_path_buf(),
            source_sha256_before: source_sha256_before.clone(),
            requested_start_sesno: requested_start,
            requested_end_sesno: requested_end,
            actual_start_sesno: actual_start as u32,
            actual_end_sesno: actual_end as u32,
            latest_sesno,
            session_count: 0,
            element_count: 0,
            add_count: 0,
            modify_count: 0,
            delete_count: 0,
            none_count: 0,
        };
        return Ok(collected_outcome_for_file(
            report,
            BTreeMap::new(),
            IncrGeoUpdateLog::default(),
            Vec::new(),
        ));
    }

    // Use collect_increment_eles (published pdms-io dev-3.1). Local forks may also expose
    // collect_increment_eles_with_progress; keep CI aligned with the git branch pin.
    let grouped = io
        .collect_increment_eles(Some(actual_start..=actual_end))
        .with_context(|| {
            format!(
                "收集 PDMS 增量失败: dbnum={} sesno={}..={} file={}",
                dbnum,
                actual_start,
                actual_end,
                file_path.display()
            )
        })?;

    let mut update_log = IncrGeoUpdateLog::default();
    let mut add_count = 0usize;
    let mut modify_count = 0usize;
    let mut delete_count = 0usize;
    let mut none_count = 0usize;
    let mut element_count = 0usize;
    let mut element_changes = Vec::new();

    for operations in grouped.values() {
        for operation in operations {
            element_count += 1;
            let is_none_operation = matches!(&operation.detail, EleOperationDetail::None);
            match &operation.detail {
                EleOperationDetail::Add(_) => add_count += 1,
                EleOperationDetail::Modified(_) => modify_count += 1,
                EleOperationDetail::Deleted => delete_count += 1,
                EleOperationDetail::None => none_count += 1,
            }

            let impact = classify_operation(operation, model_impact_filter);
            let model_change = apply_pdms_operation(&mut io, &mut update_log, operation, &impact);
            apply_critical_model_expansion(&mut io, &mut update_log, operation, &impact);
            let classified = model_change.is_some()
                || is_none_operation
                || !impact.triggers_model()
                || matches!(
                    impact.reason,
                    "owner_changed" | "children_changed" | "noun_changed"
                );
            if !classified {
                println!(
                    "警告：未知 PDMS noun/type {} 对于 refno {}",
                    operation.get_noun_type(),
                    RefnoEnum::from(operation.refno)
                );
            }
            element_changes.push(PdmsSesnoElementChange {
                dbnum,
                project: project.to_string(),
                file_path: file_path.to_path_buf(),
                sesno: operation.sesno as u32,
                refno: RefnoEnum::from(operation.refno),
                operation: operation_kind(operation).to_string(),
                noun: operation.get_noun_type(),
                owner_refno: operation_owner_refno(operation),
                changed_attributes: impact.changed_attributes,
                impact_decision: impact.decision.to_string(),
                impact_reason: impact.reason.to_string(),
                classified,
                model_category: model_change.map(|change| change.category.to_string()),
                model_refno: model_change.map(|change| change.refno),
            });
        }
    }

    let report = PdmsSesnoIncrementFileReport {
        dbnum,
        project: project.to_string(),
        file_path: file_path.to_path_buf(),
        source_sha256_before,
        requested_start_sesno: requested_start,
        requested_end_sesno: requested_end,
        actual_start_sesno: actual_start as u32,
        actual_end_sesno: actual_end as u32,
        latest_sesno,
        session_count: grouped.len(),
        element_count,
        add_count,
        modify_count,
        delete_count,
        none_count,
    };

    Ok(collected_outcome_for_file(
        report,
        grouped,
        update_log,
        element_changes,
    ))
}

pub fn collect_pdms_increment_for_file_with_operations(
    project: &str,
    file_path: impl AsRef<Path>,
    cached_sesno: u32,
    to_sesno: Option<u32>,
    detail: bool,
) -> anyhow::Result<PdmsSesnoCollectedOutcome> {
    collect_pdms_increment_for_file_with_operations_options(
        project,
        file_path,
        cached_sesno,
        to_sesno,
        detail,
        true,
    )
}

pub fn collect_pdms_increment_for_file(
    project: &str,
    file_path: impl AsRef<Path>,
    cached_sesno: u32,
    to_sesno: Option<u32>,
    detail: bool,
) -> anyhow::Result<PdmsSesnoIncrementOutcome> {
    Ok(collect_pdms_increment_for_file_with_operations(
        project,
        file_path,
        cached_sesno,
        to_sesno,
        detail,
    )?
    .into_outcome())
}

/// 通过 db_index.sqlite 将 dbnum 定位到实际 db 文件，再按 sesno 收集增量。
#[cfg(feature = "sqlite-index")]
pub fn collect_pdms_increment_for_dbnums_from_index_with_operations_options(
    project: &str,
    index_path: impl AsRef<Path>,
    dbnums: &[u32],
    cached_sesno: u32,
    to_sesno: Option<u32>,
    detail: bool,
    model_impact_filter: bool,
) -> anyhow::Result<PdmsSesnoCollectedOutcome> {
    let store = crate::data_interface::db_index::DbIndexStore::open(index_path)?;
    let mut outcome = PdmsSesnoCollectedOutcome::default();

    for dbnum in dbnums {
        let active = discover_active_db_file_for_dbnum(*dbnum)?;
        let (record_project, record_path) = if let Some((active_project, active_path)) = active {
            (active_project, active_path)
        } else {
            let Some(record) = store.file_by_dbnum(*dbnum) else {
                anyhow::bail!("db_index.sqlite 中找不到 dbnum={} 对应的文件", dbnum);
            };
            let record_project = if record.project.trim().is_empty() {
                project.to_string()
            } else {
                record.project
            };
            (record_project, PathBuf::from(record.file_path))
        };
        let file_outcome = collect_pdms_increment_for_file_with_operations_options(
            &record_project,
            &record_path,
            cached_sesno,
            to_sesno,
            detail,
            model_impact_filter,
        )
        .with_context(|| format!("dbnum={} 增量收集失败", dbnum))?;
        outcome.merge(file_outcome);
    }

    Ok(outcome)
}

#[cfg(feature = "sqlite-index")]
pub fn collect_pdms_increment_for_dbnums_from_index_with_operations(
    project: &str,
    index_path: impl AsRef<Path>,
    dbnums: &[u32],
    cached_sesno: u32,
    to_sesno: Option<u32>,
    detail: bool,
) -> anyhow::Result<PdmsSesnoCollectedOutcome> {
    collect_pdms_increment_for_dbnums_from_index_with_operations_options(
        project,
        index_path,
        dbnums,
        cached_sesno,
        to_sesno,
        detail,
        true,
    )
}

#[cfg(feature = "sqlite-index")]
pub fn collect_pdms_increment_for_dbnums_from_index(
    project: &str,
    index_path: impl AsRef<Path>,
    dbnums: &[u32],
    cached_sesno: u32,
    to_sesno: Option<u32>,
    detail: bool,
) -> anyhow::Result<PdmsSesnoIncrementOutcome> {
    Ok(
        collect_pdms_increment_for_dbnums_from_index_with_operations(
            project,
            index_path,
            dbnums,
            cached_sesno,
            to_sesno,
            detail,
        )?
        .into_outcome(),
    )
}
