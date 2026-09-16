//! 中继模式（`sync_relay_mode = true`）的广播轮询——`watch_incremental` 的 SQLite-only 替身。
//!
//! 完整站点靠 SurrealDB 里的 Committed Watermark 决定「哪些文件要广播」（`watch_incremental.rs:154`），
//! 中继站点没有模型库，这里改为：
//! - 水位存 SQLite（`relay_sync_watermark`，P1 建表，收包端校验通过也写它防回声）；
//! - 变更由 e3d-io 直接读源 db 文件的会话链判定（`diff_sessions(AtOrBefore(水位), Latest)`）；
//! - 有变更才走 `MqttFilePublisher::publish_source_files` 广播，tally / RefNo 清单随发布进
//!   `e3d_sync_ledger` / `e3d_sync_changes`。
//!
//! 每轮（周期 = `remote_sync_env_config.detect_interval`，读不到用 30 s）：
//! 1. `db_index::rebuild_from_config(false)`：pdms_io 读 latest_sesno + 指纹，落 SQLite db_index；
//! 2. 过滤：`manual_db_nums` 非空只看它们；`location_dbs` 非空只广播自有库（今天完整站点会把收到的
//!    别家文件再发一遍，中继在源头掐掉这个回声）；
//! 3. 每个文件按 [`decide`]：无水位 → 写基线不广播；`latest_sesno <= 水位` → 跳过；指纹与上一轮不同 →
//!    记指纹、本轮跳过（一拍去抖，文件还在变就等它停下来）；否则进 e3d-io：打不开 → `open_failed`；
//!    pdms_io 与 e3d-io 读到的 latest sesno 不一致 → `sesno_disagree`；diff 为空 → 只推水位；
//!    diff 判不了 → `unavailable` 仍广播（判不了变更不等于没变更）；有变更 → 广播；
//! 4. 广播成功才推水位；失败记台账、水位不动、下轮重试（CBA 已写到 `assets/archives/`，无害）。
//!
//! 任何一步失败都不让循环退出，与 `watch_incremental` 的常驻语义一致。全程不碰 SurrealDB。
//! 同一个文件连续几轮同一种失败只记一行台账（内存去重），避免 30 s 一行把表撑爆。
//!
//! 设计见 `plant-collab-monitor/docs/plans/2026-09-15-sqlite-only-remote-collab-plan.md` P3。

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::Duration;

use crate::data_interface::db_index::{self, DbFileRecord, DbIndexStore};
use crate::data_interface::mqtt_file_sync::{MqttFilePublisher, PublishSourceFile};
use crate::data_interface::sync_ledger::{
    self, ChangeEntry, DiffStatus, DiffTally, Direction, LedgerRow, MAX_CHANGES_PER_ROW,
    VerifyStatus, Watermark,
};

/// `remote_sync_env_config.detect_interval` 读不到（未配置 / 表不存在）时的轮询周期。
pub const DEFAULT_INTERVAL_SECS: u64 = 30;

#[derive(Debug, Clone)]
pub struct RelaySyncOptions {
    /// 当前激活的 env；`remote_sync_env_config` 按它取 `detect_interval`。
    pub env_id: String,
    /// `manual_db_nums`：非空则只看这些 dbnum。
    pub requested_dbnums: Vec<u32>,
    /// 覆盖 `detect_interval`（CLI / 测试用）；None 每轮从配置表读。
    pub interval_secs: Option<u64>,
    /// 只跑一轮就返回（测试 / 手工触发）。
    pub once: bool,
}

/// 一轮的统计（日志用）。
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct CycleReport {
    /// 过滤后进入判定的文件数。
    pub candidates: usize,
    pub baseline: usize,
    pub unchanged: usize,
    pub debounced: usize,
    pub broadcast: usize,
    pub empty_diff: usize,
    /// open_failed / sesno_disagree / publish_failed。
    pub problems: usize,
}

impl CycleReport {
    fn is_quiet(&self) -> bool {
        self.baseline + self.debounced + self.broadcast + self.empty_diff + self.problems == 0
    }
}

/// 一个文件在本轮该走哪一步（纯判定，便于单测）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Step {
    /// 水位表里没有这个 dbnum：写基线，不广播。
    Baseline,
    /// `latest_sesno <= 水位`：没有新会话。
    Unchanged,
    /// sesno 前进了，但指纹与上一轮不同：文件可能还在写，记指纹、等下一轮。
    Debounce,
    /// sesno 前进且指纹稳定：进 e3d-io。
    Inspect { watermark: u32 },
}

pub fn decide(latest_sesno: u32, fingerprint: &str, watermark: Option<&Watermark>) -> Step {
    let Some(watermark) = watermark else {
        return Step::Baseline;
    };
    if latest_sesno <= watermark.sesno {
        return Step::Unchanged;
    }
    if watermark.last_seen_fingerprint.as_deref() != Some(fingerprint) {
        return Step::Debounce;
    }
    Step::Inspect {
        watermark: watermark.sesno,
    }
}

#[derive(Default)]
struct RelayState {
    /// dbnum → 上一次记进台账的问题签名；相同签名不再重复落行。
    last_problem: HashMap<u32, String>,
    index_bootstrapped: bool,
}

/// 常驻中继轮询。`once = true` 时跑一轮返回该轮结果；否则永不返回（错误只记日志）。
pub async fn run_relay_watch(mut options: RelaySyncOptions) -> anyhow::Result<()> {
    options.requested_dbnums.sort_unstable();
    options.requested_dbnums.dedup();

    let publisher = MqttFilePublisher::start();
    let mut state = RelayState::default();
    log::info!(
        "relay-sync 启动: env_id={} requested_dbnums={:?} interval={}",
        options.env_id,
        options.requested_dbnums,
        match options.interval_secs {
            Some(secs) => format!("{secs}s (override)"),
            None => "remote_sync_env_config.detect_interval".to_string(),
        }
    );

    loop {
        let interval = poll_interval_secs(&options).await;
        let outcome = run_cycle(&options, &publisher, &mut state).await;
        match &outcome {
            Ok(report) if report.is_quiet() => log::debug!(
                "relay-sync 本轮无事: candidates={} unchanged={}",
                report.candidates,
                report.unchanged
            ),
            Ok(report) => log::info!(
                "relay-sync 本轮: candidates={} baseline={} unchanged={} debounced={} broadcast={} empty_diff={} problems={}",
                report.candidates,
                report.baseline,
                report.unchanged,
                report.debounced,
                report.broadcast,
                report.empty_diff,
                report.problems
            ),
            Err(error) => log::warn!("relay-sync 本轮失败，{interval}s 后重试: {error:#}"),
        }
        if options.once {
            return outcome.map(|_| ());
        }
        tokio::time::sleep(Duration::from_secs(interval)).await;
    }
}

/// 跑一轮：刷 db_index → 读水位 → 逐文件判定 / 广播。
///
/// 配置每轮从 `DB_OPTION_FILE` 重新读：`activate` 会改写 `location` / `location_dbs`，
/// 而 `aios_core::get_db_option()` 是进程启动时的快照。
async fn run_cycle(
    options: &RelaySyncOptions,
    publisher: &MqttFilePublisher,
    state: &mut RelayState,
) -> anyhow::Result<CycleReport> {
    let db_option = db_index::load_db_option_from_env()?;
    let index_path = db_index::default_index_path(&db_option.project_name);
    let force = !state.index_bootstrapped && !index_path.exists();
    let scan = db_index::rebuild_from_config(force).await?;
    state.index_bootstrapped = true;
    log::debug!(
        "relay-sync db_index: scanned={} skipped={} db_files={}",
        scan.scanned,
        scan.skipped,
        scan.db_files
    );

    let records = DbIndexStore::open(&index_path)?.all_db_files();
    let watermarks = tokio::task::spawn_blocking(sync_ledger::load_watermarks_blocking).await??;
    let own_dbs = db_option
        .location_dbs
        .clone()
        .filter(|dbnums| !dbnums.is_empty());
    let location = db_option.location.clone();

    let mut report = CycleReport::default();
    for record in records {
        if !options.requested_dbnums.is_empty() && !options.requested_dbnums.contains(&record.dbnum)
        {
            continue;
        }
        if let Some(own) = &own_dbs
            && !own.contains(&record.dbnum)
        {
            continue;
        }
        report.candidates += 1;
        process_record(
            &record,
            watermarks.get(&record.dbnum),
            &location,
            publisher,
            state,
            &mut report,
        )
        .await;
    }
    Ok(report)
}

async fn process_record(
    record: &DbFileRecord,
    watermark: Option<&Watermark>,
    location: &str,
    publisher: &MqttFilePublisher,
    state: &mut RelayState,
    report: &mut CycleReport,
) {
    let dbnum = record.dbnum;
    let file_name = record.file_name.as_str();
    let fingerprint = record.fingerprint.as_str();
    let latest = record.latest_sesno;

    let watermark_sesno = match decide(latest, fingerprint, watermark) {
        Step::Baseline => {
            set_watermark(dbnum, file_name, latest, Some(fingerprint)).await;
            log::info!(
                "relay-sync 基线 dbnum={dbnum} {file_name}: sesno={latest}（首次见到，只记水位不广播）"
            );
            report.baseline += 1;
            return;
        }
        Step::Unchanged => {
            report.unchanged += 1;
            state.last_problem.remove(&dbnum);
            return;
        }
        Step::Debounce => {
            let previous = watermark.map(|w| w.sesno).unwrap_or_default();
            log::info!(
                "relay-sync 去抖 dbnum={dbnum} {file_name}: sesno {previous} -> {latest}，指纹刚变，等下一轮"
            );
            touch_fingerprint(dbnum, fingerprint).await;
            report.debounced += 1;
            return;
        }
        Step::Inspect { watermark } => watermark,
    };

    let path = PathBuf::from(&record.file_path);
    let inspected = match tokio::task::spawn_blocking({
        let path = path.clone();
        move || inspect_blocking(&path, watermark_sesno, latest)
    })
    .await
    {
        Ok(inspected) => inspected,
        Err(error) => Inspected::OpenFailed(format!("inspect task panicked: {error}")),
    };
    let msg_id = format!("relay:{dbnum}:{watermark_sesno}->{latest}");
    let mut row = LedgerRow::new(
        msg_id,
        Direction::Outbound,
        location,
        file_name,
        VerifyStatus::Skipped,
    );
    row.sesno_from = Some(watermark_sesno);
    row.sesno_to = Some(latest);

    match inspected {
        Inspected::OpenFailed(detail) => {
            row.verify_status = VerifyStatus::OpenFailed;
            row.verify_detail = Some(detail);
            report.problems += 1;
            record_problem(state, dbnum, format!("open_failed:{latest}"), row).await;
        }
        Inspected::Disagree { seen } => {
            row.verify_status = VerifyStatus::SesnoDisagree;
            row.sesno_seen = Some(seen);
            row.verify_detail = Some(format!(
                "sesno_disagree: pdms_io latest_sesno={latest} e3d-io sessions().max={seen}"
            ));
            report.problems += 1;
            record_problem(state, dbnum, format!("sesno_disagree:{latest}:{seen}"), row).await;
        }
        Inspected::Ready(inspection) if inspection.diff_status == DiffStatus::Empty => {
            set_watermark(dbnum, file_name, inspection.seen, Some(fingerprint)).await;
            row.sesno_seen = Some(inspection.seen);
            row.diff = inspection.tally;
            row.diff_status = Some(DiffStatus::Empty);
            row.verify_detail = Some(join_notes(
                format!(
                    "empty_diff: 水位 {watermark_sesno} -> {} 只推水位，未广播",
                    inspection.seen
                ),
                inspection.detail,
            ));
            log::info!(
                "relay-sync dbnum={dbnum} {file_name}: sesno {watermark_sesno} -> {} 无元素变更，只推水位",
                inspection.seen
            );
            report.empty_diff += 1;
            state.last_problem.remove(&dbnum);
            sync_ledger::record(vec![row]).await;
        }
        Inspected::Ready(inspection) => {
            let file = PublishSourceFile {
                path,
                sesno: Some(inspection.seen),
                sesno_from: Some(watermark_sesno),
                sesno_seen: Some(inspection.seen),
                diff: inspection.tally,
                diff_status: Some(inspection.diff_status),
                changes: inspection.changes,
                verify_detail: inspection.detail,
            };
            let tally = inspection.tally.unwrap_or_default();
            match publisher.publish_source_files(std::slice::from_ref(&file)).await {
                Ok(()) => {
                    set_watermark(dbnum, file_name, inspection.seen, Some(fingerprint)).await;
                    log::info!(
                        "relay-sync 已广播 dbnum={dbnum} {file_name}: sesno {watermark_sesno} -> {} diff={} (+{} -{} ~{}) changes={}",
                        inspection.seen,
                        inspection.diff_status.as_str(),
                        tally.inserted,
                        tally.deleted,
                        tally.modified,
                        file.changes.len()
                    );
                    report.broadcast += 1;
                    state.last_problem.remove(&dbnum);
                }
                Err(error) => {
                    // 水位不动，下轮重试；判定结果一起进台账，排障时能看到「检测到了但没发出去」。
                    row.sesno_seen = Some(inspection.seen);
                    row.diff = file.diff;
                    row.diff_status = Some(inspection.diff_status);
                    row.changes = file.changes;
                    row.verify_detail = Some(join_notes(
                        format!("publish_failed: {error:#}"),
                        file.verify_detail,
                    ));
                    report.problems += 1;
                    record_problem(
                        state,
                        dbnum,
                        format!("publish_failed:{}", inspection.seen),
                        row,
                    )
                    .await;
                }
            }
        }
    }
}

/// 同一 dbnum 连续几轮同一个问题只落一行台账；签名变了（如 sesno 又前进）才再记。
async fn record_problem(state: &mut RelayState, dbnum: u32, signature: String, row: LedgerRow) {
    let detail = row.verify_detail.clone().unwrap_or_default();
    if state.last_problem.get(&dbnum) == Some(&signature) {
        log::debug!(
            "relay-sync dbnum={dbnum} {}: {} 仍未恢复（{detail}）",
            row.file_name,
            row.verify_status.as_str()
        );
        return;
    }
    log::warn!(
        "relay-sync dbnum={dbnum} {}: {} {detail}",
        row.file_name,
        row.verify_status.as_str()
    );
    state.last_problem.insert(dbnum, signature);
    sync_ledger::record(vec![row]).await;
}

async fn set_watermark(dbnum: u32, file_name: &str, sesno: u32, fingerprint: Option<&str>) {
    let file_name = file_name.to_string();
    let fingerprint = fingerprint.map(str::to_string);
    let result = tokio::task::spawn_blocking(move || {
        sync_ledger::set_watermark_blocking(dbnum, &file_name, sesno, fingerprint.as_deref())
    })
    .await;
    match result {
        Ok(Ok(())) => {}
        Ok(Err(error)) => log::warn!("relay-sync 写水位失败 dbnum={dbnum} sesno={sesno}: {error}"),
        Err(error) => log::warn!("relay-sync 写水位任务异常 dbnum={dbnum}: {error}"),
    }
}

async fn touch_fingerprint(dbnum: u32, fingerprint: &str) {
    let fingerprint = fingerprint.to_string();
    let result = tokio::task::spawn_blocking(move || {
        sync_ledger::touch_watermark_fingerprint_blocking(dbnum, &fingerprint)
    })
    .await;
    match result {
        Ok(Ok(())) => {}
        Ok(Err(error)) => log::warn!("relay-sync 记去抖指纹失败 dbnum={dbnum}: {error}"),
        Err(error) => log::warn!("relay-sync 记去抖指纹任务异常 dbnum={dbnum}: {error}"),
    }
}

fn join_notes(head: String, tail: Option<String>) -> String {
    match tail {
        Some(tail) if !tail.is_empty() => format!("{head}; {tail}"),
        _ => head,
    }
}

// ---------------------------------------------------------------------------
// 轮询周期
// ---------------------------------------------------------------------------

async fn poll_interval_secs(options: &RelaySyncOptions) -> u64 {
    if let Some(secs) = options.interval_secs {
        return secs.max(1);
    }
    let env_id = options.env_id.clone();
    let configured = tokio::task::spawn_blocking(move || detect_interval_blocking(&env_id))
        .await
        .ok()
        .flatten();
    configured.unwrap_or(DEFAULT_INTERVAL_SECS).max(1)
}

/// `remote_sync_env_config.detect_interval`（控制面表，与台账同库）。表不存在 / 无该 env / 非正数 → None。
fn detect_interval_blocking(env_id: &str) -> Option<u64> {
    let conn = rusqlite::Connection::open(sync_ledger::resolve_db_path()).ok()?;
    conn.busy_timeout(Duration::from_secs(5)).ok()?;
    let value: i64 = conn
        .query_row(
            "SELECT detect_interval FROM remote_sync_env_config WHERE env_id = ?1",
            rusqlite::params![env_id],
            |row| row.get(0),
        )
        .ok()?;
    (value > 0).then_some(value as u64)
}

// ---------------------------------------------------------------------------
// e3d-io 判定（同步 IO，调用方放 spawn_blocking）
// ---------------------------------------------------------------------------

#[derive(Debug)]
pub enum Inspected {
    /// e3d-io 打不开 / 读不出会话链。
    OpenFailed(String),
    /// pdms_io（db_index）与 e3d-io 读到的 latest sesno 不一致。
    Disagree { seen: u32 },
    Ready(Inspection),
}

#[derive(Debug)]
pub struct Inspection {
    /// e3d-io 读到的 latest sesno（== db_index 的 latest_sesno）。
    pub seen: u32,
    /// Ok（有变更）| Empty | Unavailable（diff 报错，仍广播）。
    pub diff_status: DiffStatus,
    pub tally: Option<DiffTally>,
    /// RefNo 清单，最多 [`MAX_CHANGES_PER_ROW`] 条。
    pub changes: Vec<ChangeEntry>,
    /// 进台账 `verify_detail` 的备注：diff 报错原文、`changes_truncated:<总数>`、水位落在被 MERGE 掉的会话上等。
    pub detail: Option<String>,
}

/// 打开源 db 文件，交叉校验 sesno，再对 `AtOrBefore(watermark) → Latest` 做会话 diff。
pub fn inspect_blocking(path: &Path, watermark: u32, latest_sesno: u32) -> Inspected {
    use e3d_io::session::{ChangeKind, SessionSelector};

    let mut engine = match e3d_io::ReadOnlyEngine::open(path) {
        Ok(engine) => engine,
        Err(error) => {
            return Inspected::OpenFailed(truncate_detail(format!(
                "e3d-io open: {error} ({})",
                path.display()
            )));
        }
    };
    let seen = match engine.sessions() {
        Ok(sessions) => match sessions.iter().map(|session| session.session_id).max() {
            Some(seen) => seen,
            None => return Inspected::OpenFailed("e3d-io sessions: 会话链为空".to_string()),
        },
        Err(error) => {
            return Inspected::OpenFailed(truncate_detail(format!("e3d-io sessions: {error}")));
        }
    };
    if seen != latest_sesno {
        return Inspected::Disagree { seen };
    }

    let diff = match engine.diff_sessions(SessionSelector::AtOrBefore(watermark), SessionSelector::Latest)
    {
        Ok(diff) => diff,
        Err(error) => {
            return Inspected::Ready(Inspection {
                seen,
                diff_status: DiffStatus::Unavailable,
                tally: None,
                changes: Vec::new(),
                detail: Some(truncate_detail(format!("diff_unavailable: {error}"))),
            });
        }
    };

    let tally = diff.tally();
    let tally = DiffTally {
        inserted: tally.inserted,
        deleted: tally.deleted,
        modified: tally.modified,
    };
    let mut notes: Vec<String> = Vec::new();
    if diff.from.sesno != watermark {
        // MERGE CHANGES 把水位那一节会话折掉了：从不晚于水位的最新保留会话起算，中间的变更会被重报而不是漏掉。
        notes.push(format!(
            "diff_from_resolved={} (watermark={watermark} 已不在会话链上)",
            diff.from.sesno
        ));
    }
    if diff.is_empty() {
        return Inspected::Ready(Inspection {
            seen,
            diff_status: DiffStatus::Empty,
            tally: Some(tally),
            changes: Vec::new(),
            detail: (!notes.is_empty()).then(|| notes.join("; ")),
        });
    }

    let total = diff.changes().len();
    let mut changes = Vec::with_capacity(total.min(MAX_CHANGES_PER_ROW));
    let mut unreadable = 0usize;
    for item in engine.changed_elements(&diff).take(MAX_CHANGES_PER_ROW) {
        match item {
            Ok(element) => changes.push(ChangeEntry {
                refno: element.refno.to_string(),
                kind: match element.kind {
                    ChangeKind::Inserted => "inserted",
                    ChangeKind::Deleted => "deleted",
                    ChangeKind::Modified {
                        relocated_only: true,
                    } => "relocated",
                    ChangeKind::Modified { .. } => "modified",
                },
            }),
            Err(_) => unreadable += 1,
        }
    }
    if total > MAX_CHANGES_PER_ROW {
        notes.push(format!("changes_truncated:{total}"));
    }
    if unreadable > 0 {
        notes.push(format!("changes_unreadable:{unreadable}"));
    }
    Inspected::Ready(Inspection {
        seen,
        diff_status: DiffStatus::Ok,
        tally: Some(tally),
        changes,
        detail: (!notes.is_empty()).then(|| notes.join("; ")),
    })
}

fn truncate_detail(text: String) -> String {
    const MAX: usize = 400;
    if text.chars().count() <= MAX {
        text
    } else {
        let mut cut: String = text.chars().take(MAX).collect();
        cut.push('…');
        cut
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn watermark(sesno: u32, fingerprint: Option<&str>) -> Watermark {
        Watermark {
            dbnum: 7001,
            file_name: "acp7001_0001".into(),
            sesno,
            last_seen_fingerprint: fingerprint.map(str::to_string),
        }
    }

    #[test]
    fn decide_follows_plan_order() {
        // 无水位 → 基线，不管 sesno / 指纹
        assert_eq!(decide(270, "1:1", None), Step::Baseline);
        // sesno 没前进 → 跳过（指纹变了也不管）
        assert_eq!(decide(270, "2:2", Some(&watermark(270, Some("1:1")))), Step::Unchanged);
        assert_eq!(decide(269, "1:1", Some(&watermark(270, Some("1:1")))), Step::Unchanged);
        // sesno 前进但指纹刚变 → 去抖；收包端写的水位指纹为 None 也算「变了」
        assert_eq!(decide(271, "2:2", Some(&watermark(270, Some("1:1")))), Step::Debounce);
        assert_eq!(decide(271, "2:2", Some(&watermark(270, None))), Step::Debounce);
        // sesno 前进且指纹稳定 → 进 e3d-io，带上水位
        assert_eq!(
            decide(271, "2:2", Some(&watermark(270, Some("2:2")))),
            Step::Inspect { watermark: 270 }
        );
    }

    #[test]
    fn inspect_non_e3d_file_is_open_failed() {
        let dir = std::env::temp_dir().join(format!("relay-sync-test-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("not_a_db_0001");
        std::fs::write(&file, b"definitely not an e3d database").unwrap();
        match inspect_blocking(&file, 1, 2) {
            Inspected::OpenFailed(detail) => assert!(detail.starts_with("e3d-io open:"), "{detail}"),
            other => panic!("expected OpenFailed, got {other:?}"),
        }
        match inspect_blocking(&dir.join("missing_0001"), 1, 2) {
            Inspected::OpenFailed(_) => {}
            other => panic!("expected OpenFailed, got {other:?}"),
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn cycle_report_quietness_and_notes() {
        let mut report = CycleReport::default();
        report.candidates = 3;
        report.unchanged = 3;
        assert!(report.is_quiet());
        report.debounced = 1;
        assert!(!report.is_quiet());

        assert_eq!(join_notes("a".into(), None), "a");
        assert_eq!(join_notes("a".into(), Some(String::new())), "a");
        assert_eq!(join_notes("a".into(), Some("b".into())), "a; b");
        assert_eq!(truncate_detail("x".repeat(10)), "x".repeat(10));
        assert_eq!(truncate_detail("x".repeat(500)).chars().count(), 401);
    }
}
