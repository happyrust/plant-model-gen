use aios_core::RefU64;
use anyhow::Context;
use pdms_io::PdmsIO;
use pdms_io::defines::IndexPageData;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashSet, VecDeque};
use std::path::{Path, PathBuf};

#[derive(Clone, Debug)]
pub struct HistoryBaselineInspectRequest {
    pub project_name: String,
    pub source_db_file: PathBuf,
    pub target_sesno: u32,
    pub parse_sample_limit: usize,
    pub require_exact_sesno: bool,
    pub detail: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct HistoryBaselineInspectResponse {
    pub project_name: String,
    pub source_db_file: PathBuf,
    pub header_dbnum: u32,
    pub requested_sesno: u32,
    pub resolved_sesno: u32,
    pub latest_sesno: u32,
    pub exact_sesno_found: bool,
    pub session_index_root_pageno: u32,
    pub session_end_pgno: u32,
    pub page_size: usize,
    pub visible_refno_count: usize,
    pub visible_offset_count: usize,
    pub duplicate_refno_count: usize,
    pub index_error_count: usize,
    pub parsed_sample_count: usize,
    pub parse_error_count: usize,
    pub sample_noun_counts: BTreeMap<String, usize>,
    pub sample_refnos: Vec<HistoryBaselineSampleRefno>,
    pub index_errors: Vec<HistoryBaselineIndexError>,
    pub parse_errors: Vec<HistoryBaselineParseError>,
    pub index_stats: HistoryBaselineIndexStats,
    pub full_state_enumeration_supported: bool,
    pub persistence_performed: bool,
    pub recommended_next_action: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct HistoryBaselineSampleRefno {
    pub refno_u64: u64,
    pub refno: String,
    pub offset: u64,
    pub noun: String,
    pub owner_u64: u64,
    pub sesno: i32,
    pub child_count: usize,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct HistoryBaselineParseError {
    pub refno_u64: u64,
    pub refno: String,
    pub offset: u64,
    pub error: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct HistoryBaselineIndexError {
    pub page_no: u32,
    pub error: String,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct HistoryBaselineIndexStats {
    pub visited_index_nodes: usize,
    pub visited_leaf_nodes: usize,
    pub visited_total_nodes: usize,
    pub total_index_entries: usize,
    pub skipped_start_marker_entries: usize,
    pub skipped_invalid_entries: usize,
    pub skipped_zero_child_pages: usize,
    pub skipped_unreadable_index_pages: usize,
}

pub async fn inspect_history_baseline(
    request: HistoryBaselineInspectRequest,
) -> anyhow::Result<HistoryBaselineInspectResponse> {
    let source_db_file = request.source_db_file.clone();
    let mut io = PdmsIO::new(&request.project_name, &source_db_file, request.detail);
    io.open()
        .with_context(|| format!("PdmsIO::open failed: {}", source_db_file.display()))?;

    let header = io
        .read_pdms_header()
        .with_context(|| format!("read PDMS header failed: {}", source_db_file.display()))?;
    let header_dbnum = u32::try_from(header.db_num).unwrap_or_default();
    let latest_sesno = io
        .get_latest_sesno()
        .with_context(|| format!("read latest sesno failed: {}", source_db_file.display()))?;

    let (resolved_sesno, exact_sesno_found) =
        resolve_target_sesno(&mut io, request.target_sesno, request.require_exact_sesno)?;
    let (session_index_root_pageno, session_end_pgno) = {
        let ses_data = io
            .get_ses_data(resolved_sesno)
            .with_context(|| format!("read session {} failed", resolved_sesno))?;
        (ses_data.index_root_pageno, ses_data.end_pgno)
    };

    if session_index_root_pageno == 0 {
        anyhow::bail!(
            "session {} has empty index_root_pageno in {}",
            resolved_sesno,
            source_db_file.display()
        );
    }

    let (visible_offsets, index_stats, duplicate_refno_count, index_errors) =
        collect_visible_offsets_at_session(&mut io, session_index_root_pageno)?;

    let (parsed_sample_count, parse_error_count, sample_noun_counts, sample_refnos, parse_errors) =
        parse_visible_sample(&mut io, &visible_offsets, request.parse_sample_limit).await;

    let index_error_count = index_stats.skipped_unreadable_index_pages;
    let full_state_enumeration_supported = exact_sesno_found
        && !visible_offsets.is_empty()
        && session_index_root_pageno != 0
        && index_error_count == 0;
    let recommended_next_action = if full_state_enumeration_supported && parse_error_count == 0 {
        "target_sesno_index_enumeration_ready; next step is SurrealDB hydrate using the same visible refno set"
            .to_string()
    } else if !exact_sesno_found {
        "requested_sesno_not_found; rerun with an existing session or provide a physical baseline snapshot"
            .to_string()
    } else if visible_offsets.is_empty() {
        "target_sesno_index_empty; do not publish as a visual baseline".to_string()
    } else if index_error_count > 0 {
        "target_sesno_index_not_publishable; index traversal is incomplete, so use a physical baseline snapshot, restore a published baseline package, or add a proven pdms-io full-state hydrate provider"
            .to_string()
    } else {
        "parse_errors_present; inspect parse_errors before enabling baseline hydrate".to_string()
    };

    Ok(HistoryBaselineInspectResponse {
        project_name: request.project_name,
        source_db_file,
        header_dbnum,
        requested_sesno: request.target_sesno,
        resolved_sesno,
        latest_sesno,
        exact_sesno_found,
        session_index_root_pageno,
        session_end_pgno,
        page_size: io.page_size,
        visible_refno_count: visible_offsets.len(),
        visible_offset_count: visible_offsets.len() + duplicate_refno_count,
        duplicate_refno_count,
        index_error_count,
        parsed_sample_count,
        parse_error_count,
        sample_noun_counts,
        sample_refnos,
        index_errors,
        parse_errors,
        index_stats,
        full_state_enumeration_supported,
        persistence_performed: false,
        recommended_next_action,
    })
}

fn resolve_target_sesno(
    io: &mut PdmsIO,
    requested_sesno: u32,
    require_exact_sesno: bool,
) -> anyhow::Result<(u32, bool)> {
    if io.get_ses_pageno(requested_sesno as i32).is_some() {
        return Ok((requested_sesno, true));
    }

    if require_exact_sesno {
        anyhow::bail!("requested session {} does not exist", requested_sesno);
    }

    let Some(resolved) = io.get_nearest_less_sesno((requested_sesno as i32).saturating_add(1))
    else {
        anyhow::bail!(
            "no session less than or equal to {} exists",
            requested_sesno
        );
    };
    Ok((resolved as u32, resolved as u32 == requested_sesno))
}

fn collect_visible_offsets_at_session(
    io: &mut PdmsIO,
    root_pgno: u32,
) -> anyhow::Result<(
    BTreeMap<RefU64, u64>,
    HistoryBaselineIndexStats,
    usize,
    Vec<HistoryBaselineIndexError>,
)> {
    let mut stats = HistoryBaselineIndexStats::default();
    let mut visible_offsets = BTreeMap::<RefU64, u64>::new();
    let mut duplicate_refno_count = 0usize;
    let mut index_errors = Vec::new();
    let mut queue = VecDeque::new();
    let mut visited = HashSet::new();
    queue.push_back(root_pgno);

    while let Some(pgno) = queue.pop_front() {
        if pgno == 0 {
            stats.skipped_zero_child_pages += 1;
            continue;
        }
        if !visited.insert(pgno) {
            continue;
        }

        let index_data = match io.read_index_data(pgno) {
            Ok(index_data) => index_data,
            Err(e) if pgno == root_pgno => {
                return Err(e).with_context(|| format!("read root index page {} failed", pgno));
            }
            Err(e) => {
                stats.skipped_unreadable_index_pages += 1;
                if index_errors.len() < 20 {
                    index_errors.push(HistoryBaselineIndexError {
                        page_no: pgno,
                        error: e.to_string(),
                    });
                }
                continue;
            }
        };
        stats.visited_total_nodes += 1;
        let max_entries = entry_count(&index_data, io.page_size);
        stats.total_index_entries += max_entries;

        if index_data.level == 0 {
            stats.visited_leaf_nodes += 1;
            for loc in index_data.refno_locs.iter().take(max_entries) {
                if loc.is_start_page() {
                    stats.skipped_start_marker_entries += 1;
                    continue;
                }
                if loc.refno_0 == 0
                    || loc.refno_1 == 0
                    || loc.pgno == 0
                    || loc.offset == 0
                    || loc.flag != 1
                {
                    stats.skipped_invalid_entries += 1;
                    continue;
                }
                let refno = loc.get_refno();
                let offset = loc.get_att_offset_with_page_size(io.page_size);
                if visible_offsets.insert(refno, offset).is_some() {
                    duplicate_refno_count += 1;
                }
            }
        } else {
            stats.visited_index_nodes += 1;
            for loc in index_data.refno_locs.iter().take(max_entries) {
                if loc.pgno == 0 {
                    stats.skipped_zero_child_pages += 1;
                    continue;
                }
                queue.push_back(loc.pgno);
            }
        }
    }

    Ok((visible_offsets, stats, duplicate_refno_count, index_errors))
}

fn entry_count(index_data: &IndexPageData, page_size: usize) -> usize {
    let page_capacity = (page_size / 4).saturating_sub(7) / 4;
    let decoded_entries = index_data.refno_locs.len().min(page_capacity);
    let declared_entries = index_data.unknowns[1] as usize;
    if declared_entries == 0 {
        decoded_entries
    } else {
        declared_entries.min(decoded_entries)
    }
}

async fn parse_visible_sample(
    io: &mut PdmsIO,
    visible_offsets: &BTreeMap<RefU64, u64>,
    parse_sample_limit: usize,
) -> (
    usize,
    usize,
    BTreeMap<String, usize>,
    Vec<HistoryBaselineSampleRefno>,
    Vec<HistoryBaselineParseError>,
) {
    let mut parsed_sample_count = 0usize;
    let mut parse_error_count = 0usize;
    let mut noun_counts = BTreeMap::<String, usize>::new();
    let mut samples = Vec::new();
    let mut errors = Vec::new();

    for (refno, offset) in visible_offsets.iter().take(parse_sample_limit) {
        let page_no = (*offset as usize / io.page_size) as u32;
        let element_sesno = io.get_sesno(page_no).unwrap_or_default() as i32;
        match io.parse_element(*offset).await {
            Ok(ele) => {
                parsed_sample_count += 1;
                let noun = ele.att_map().get_type();
                *noun_counts.entry(noun.clone()).or_default() += 1;
                samples.push(HistoryBaselineSampleRefno {
                    refno_u64: refno.0,
                    refno: refno.to_string(),
                    offset: *offset,
                    noun,
                    owner_u64: ele.owner.0,
                    sesno: element_sesno,
                    child_count: ele.children.len(),
                });
            }
            Err(e) => {
                parse_error_count += 1;
                if errors.len() < 20 {
                    errors.push(HistoryBaselineParseError {
                        refno_u64: refno.0,
                        refno: refno.to_string(),
                        offset: *offset,
                        error: e.to_string(),
                    });
                }
            }
        }
    }

    (
        parsed_sample_count,
        parse_error_count,
        noun_counts,
        samples,
        errors,
    )
}

#[allow(dead_code)]
fn _assert_path_is_file(path: &Path) -> anyhow::Result<()> {
    if !path.is_file() {
        anyhow::bail!(
            "source DB file is missing or not a file: {}",
            path.display()
        );
    }
    Ok(())
}
