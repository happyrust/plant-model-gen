//! MQTT 源文件同步台账（SQLite）。
//!
//! 取代原先发布端 / 订阅端两处 `INSERT INTO e3d_sync`（SurrealDB）的写入，落到与异地协同
//! 控制面同一个 `deployment_sites.sqlite`，让中继站点在没有 SurrealDB 时也能记账、可查。
//!
//! 约定：
//! - 本模块所有写入 **只 `warn!` 不上抛**——台账丢一行不该让文件同步失败。
//! - DDL 幂等（`CREATE TABLE IF NOT EXISTS`），首次写入前自动建表；`web_server` 启动期也会经
//!   `collab_migrations::ensure_collab_schema` 建一次，保证没收过消息的站点也能直接查表。
//! - SQLite 路径与 `web_server/collab_migrations.rs`、`remote_sync_handlers.rs` 同一约定：
//!   `DB_OPTION_FILE` 指向的 toml 里的 `deployment_sites_sqlite_path`，缺省 `deployment_sites.sqlite`。
//!
//! 表：
//! - `e3d_sync_ledger`      每条 MQTT 消息里的每个文件一行（outbound / inbound），带校验结果与 diff 计数
//! - `e3d_sync_changes`     变更清单（RefNo 级），由中继轮询的 e3d-io diff 填
//! - `relay_sync_watermark` 中继水位（每 dbnum 一行）：收包端校验通过后写入以防回声
//!
//! 中继本身 2026-09-16 搬去了 `../plant-web-server`（`src/relay/`），这个后端只剩完整站点
//! 那条路径：仍然收发 MQTT 源文件并记这本账，但不判会话 diff（没有 e3d-io），也不做中继广播。
//! 水位表因此只由收包端写，`load_watermarks_*` / `set_watermark_*` 这几个读写口留着给中继用。
//!
//! 设计见 `plant-collab-monitor/docs/plans/2026-09-15-sqlite-only-remote-collab-plan.md` P1 / P3。

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use blake2::{Blake2b512, Digest};
use rusqlite::{Connection, params};

use crate::mqtt_service::SyncE3dFileMsg;

/// 台账 + 变更清单 + 中继水位的 DDL（幂等）。
pub const SCHEMA_SQL: &str = r#"
-- 每个文件一行；同一条 MQTT 消息里的多个文件共用 msg_id
CREATE TABLE IF NOT EXISTS e3d_sync_ledger (
    id             TEXT PRIMARY KEY,          -- uuid v4
    msg_id         TEXT NOT NULL,             -- 消息分组键：payload 字节的 Blake2b512 前 16 字节 hex
    direction      TEXT NOT NULL,             -- outbound | inbound
    location       TEXT NOT NULL,             -- 消息来源站点（outbound 为本站）
    file_name      TEXT NOT NULL,
    file_hash      TEXT,                      -- 消息携带的源文件 Blake2b512（小写 hex）
    sesno_from     INTEGER,                   -- outbound: 广播前水位
    sesno_to       INTEGER,                   -- outbound: 本次 latest；inbound: 消息声明的 file_sesnos[i]
    sesno_seen     INTEGER,                   -- e3d-io 实际读到的 latest sesno
    diff_inserted  INTEGER,
    diff_deleted   INTEGER,
    diff_modified  INTEGER,
    diff_status    TEXT,                      -- ok | empty | unavailable | skipped
    verify_status  TEXT NOT NULL,             -- ok | hash_mismatch | open_failed | sesno_mismatch | sesno_disagree | clone_failed | skipped
    verify_detail  TEXT,
    msg_timestamp  TEXT,                      -- 消息自带时间戳 RFC3339
    created_at     TEXT NOT NULL              -- 本行写入时间 RFC3339
);
CREATE INDEX IF NOT EXISTS idx_e3d_sync_ledger_created ON e3d_sync_ledger(created_at);
CREATE INDEX IF NOT EXISTS idx_e3d_sync_ledger_file    ON e3d_sync_ledger(file_name, created_at);

-- 变更清单：这一轮广播改了哪些 RefNo
CREATE TABLE IF NOT EXISTS e3d_sync_changes (
    ledger_id  TEXT NOT NULL,
    refno      TEXT NOT NULL,                 -- "dbno/seq"
    kind       TEXT NOT NULL,                 -- inserted | deleted | modified | relocated
    PRIMARY KEY (ledger_id, refno)
);

-- 中继水位：每个 dbnum 已广播（或已接收 / 基线）的 latest sesno
CREATE TABLE IF NOT EXISTS relay_sync_watermark (
    dbnum                  INTEGER PRIMARY KEY,
    file_name              TEXT NOT NULL,
    sesno                  INTEGER NOT NULL,
    last_seen_fingerprint  TEXT,              -- 去抖：上一轮看到的 "{mtime_nanos}:{size}"
    updated_at             TEXT NOT NULL
);
"#;

/// `e3d_sync_changes` 单次写入上限；超出只写前 N 行并在 `verify_detail` 标 `changes_truncated:<总数>`。
pub const MAX_CHANGES_PER_ROW: usize = 20_000;

static SCHEMA_READY: AtomicBool = AtomicBool::new(false);

/// 台账所在 SQLite 路径（与控制面同库）。
pub fn resolve_db_path() -> String {
    use config as cfg;
    let cfg_name =
        std::env::var("DB_OPTION_FILE").unwrap_or_else(|_| "db_options/DbOption".to_string());
    let cfg_file = format!("{}.toml", cfg_name);
    if Path::new(&cfg_file).exists()
        && let Ok(builder) = cfg::Config::builder()
            .add_source(cfg::File::with_name(&cfg_name))
            .build()
        && let Ok(path) = builder.get_string("deployment_sites_sqlite_path")
    {
        return path;
    }
    "deployment_sites.sqlite".to_string()
}

/// 幂等建表。成功后本进程内不再重复执行。
pub fn ensure_schema(conn: &Connection) -> rusqlite::Result<()> {
    conn.execute_batch(SCHEMA_SQL)?;
    SCHEMA_READY.store(true, Ordering::Release);
    Ok(())
}

fn open() -> rusqlite::Result<Connection> {
    let path = resolve_db_path();
    if let Some(parent) = Path::new(&path).parent()
        && !parent.as_os_str().is_empty()
    {
        let _ = std::fs::create_dir_all(parent);
    }
    let conn = Connection::open(&path)?;
    conn.busy_timeout(Duration::from_secs(5))?;
    if !SCHEMA_READY.load(Ordering::Acquire) {
        ensure_schema(&conn)?;
    }
    Ok(conn)
}

// ---------------------------------------------------------------------------
// 行模型
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Direction {
    Outbound,
    Inbound,
}

impl Direction {
    pub fn as_str(self) -> &'static str {
        match self {
            Direction::Outbound => "outbound",
            Direction::Inbound => "inbound",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VerifyStatus {
    /// 所有能做的校验都过了（inbound 要求 hash、e3d-io 打开、sesno 三项齐；outbound 表示已发布）。
    Ok,
    /// clone 结果的 Blake2b512 与消息 `file_hashes[i]` 不一致。
    HashMismatch,
    /// e3d-io 打不开 / 读不出会话链（或源文件本身读不了）。
    OpenFailed,
    /// e3d-io 读到的 latest sesno 与消息声明的 `file_sesnos[i]` 不一致。
    SesnoMismatch,
    /// 发布端 pdms_io 与 e3d-io 对同一文件读到的 latest sesno 不一致（P3 用）。
    SesnoDisagree,
    /// CBA clone 本身失败。
    CloneFailed,
    /// 校验没有做全：消息缺 `file_sesnos`（旧发送端）、本构建没有 e3d-io、文件被跳过等，详情见 `verify_detail`。
    Skipped,
}

impl VerifyStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            VerifyStatus::Ok => "ok",
            VerifyStatus::HashMismatch => "hash_mismatch",
            VerifyStatus::OpenFailed => "open_failed",
            VerifyStatus::SesnoMismatch => "sesno_mismatch",
            VerifyStatus::SesnoDisagree => "sesno_disagree",
            VerifyStatus::CloneFailed => "clone_failed",
            VerifyStatus::Skipped => "skipped",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DiffStatus {
    Ok,
    Empty,
    Unavailable,
    Skipped,
}

impl DiffStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            DiffStatus::Ok => "ok",
            DiffStatus::Empty => "empty",
            DiffStatus::Unavailable => "unavailable",
            DiffStatus::Skipped => "skipped",
        }
    }
}

/// e3d-io `SessionDiff::tally()` 的三个计数。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct DiffTally {
    pub inserted: u64,
    pub deleted: u64,
    pub modified: u64,
}

/// 变更清单里的一行。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChangeEntry {
    /// `"dbno/seq"`，与 e3d-io `RefNo` 的 Display 一致。
    pub refno: String,
    /// inserted | deleted | modified | relocated
    pub kind: &'static str,
}

/// `e3d_sync_ledger` 的一行（连带它的 `e3d_sync_changes`）。
#[derive(Debug, Clone)]
pub struct LedgerRow {
    pub msg_id: String,
    pub direction: Direction,
    pub location: String,
    pub file_name: String,
    pub file_hash: Option<String>,
    pub sesno_from: Option<u32>,
    pub sesno_to: Option<u32>,
    pub sesno_seen: Option<u32>,
    pub diff: Option<DiffTally>,
    pub diff_status: Option<DiffStatus>,
    pub verify_status: VerifyStatus,
    pub verify_detail: Option<String>,
    pub msg_timestamp: Option<String>,
    pub changes: Vec<ChangeEntry>,
}

impl LedgerRow {
    /// 只填必填项，其余留空。
    pub fn new(
        msg_id: impl Into<String>,
        direction: Direction,
        location: impl Into<String>,
        file_name: impl Into<String>,
        verify_status: VerifyStatus,
    ) -> Self {
        Self {
            msg_id: msg_id.into(),
            direction,
            location: location.into(),
            file_name: file_name.into(),
            file_hash: None,
            sesno_from: None,
            sesno_to: None,
            sesno_seen: None,
            diff: None,
            diff_status: None,
            verify_status,
            verify_detail: None,
            msg_timestamp: None,
            changes: Vec::new(),
        }
    }
}

// ---------------------------------------------------------------------------
// 写入
// ---------------------------------------------------------------------------

/// 写一批台账行（通常是同一条消息里的全部文件）。失败只 warn，永不上抛。
pub async fn record(rows: Vec<LedgerRow>) {
    if rows.is_empty() {
        return;
    }
    let count = rows.len();
    match tokio::task::spawn_blocking(move || record_blocking(&rows)).await {
        Ok(Ok(())) => log::debug!("[sync-ledger] 已写入 {count} 行"),
        Ok(Err(error)) => log::warn!("[sync-ledger] 写台账失败（{count} 行丢弃，不影响同步）: {error}"),
        Err(error) => log::warn!("[sync-ledger] 写台账任务异常: {error}"),
    }
}

/// 同步版本：一个事务写完所有行。
pub fn record_blocking(rows: &[LedgerRow]) -> rusqlite::Result<()> {
    let mut conn = open()?;
    let tx = conn.transaction()?;
    for row in rows {
        insert_row(&tx, row)?;
    }
    tx.commit()
}

fn insert_row(conn: &Connection, row: &LedgerRow) -> rusqlite::Result<()> {
    let id = uuid::Uuid::new_v4().to_string();
    let created_at = chrono::Utc::now().to_rfc3339();

    let (changes, truncated_total) = if row.changes.len() > MAX_CHANGES_PER_ROW {
        (&row.changes[..MAX_CHANGES_PER_ROW], Some(row.changes.len()))
    } else {
        (&row.changes[..], None)
    };
    let verify_detail = match truncated_total {
        Some(total) => Some(match &row.verify_detail {
            Some(detail) => format!("{detail}; changes_truncated:{total}"),
            None => format!("changes_truncated:{total}"),
        }),
        None => row.verify_detail.clone(),
    };

    conn.execute(
        "INSERT INTO e3d_sync_ledger (
            id, msg_id, direction, location, file_name, file_hash,
            sesno_from, sesno_to, sesno_seen,
            diff_inserted, diff_deleted, diff_modified, diff_status,
            verify_status, verify_detail, msg_timestamp, created_at
        ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17)",
        params![
            id,
            row.msg_id,
            row.direction.as_str(),
            row.location,
            row.file_name,
            row.file_hash,
            row.sesno_from,
            row.sesno_to,
            row.sesno_seen,
            row.diff.map(|d| d.inserted as i64),
            row.diff.map(|d| d.deleted as i64),
            row.diff.map(|d| d.modified as i64),
            row.diff_status.map(DiffStatus::as_str),
            row.verify_status.as_str(),
            verify_detail,
            row.msg_timestamp,
            created_at,
        ],
    )?;

    if !changes.is_empty() {
        let mut stmt = conn.prepare_cached(
            "INSERT OR IGNORE INTO e3d_sync_changes (ledger_id, refno, kind) VALUES (?1, ?2, ?3)",
        )?;
        for change in changes {
            stmt.execute(params![id, change.refno, change.kind])?;
        }
    }
    Ok(())
}

/// 把某 dbnum 的中继水位设为 `sesno`（收包端校验通过后调用，防止本站点中继轮询把刚收到的文件再广播）。
/// 文件刚被改写，旧指纹已失效，一并清空。失败只 warn。
pub async fn upsert_watermark(dbnum: u32, file_name: String, sesno: u32) {
    match tokio::task::spawn_blocking(move || upsert_watermark_blocking(dbnum, &file_name, sesno))
        .await
    {
        Ok(Ok(())) => log::debug!("[sync-ledger] relay_sync_watermark dbnum={dbnum} -> sesno {sesno}"),
        Ok(Err(error)) => {
            log::warn!("[sync-ledger] 写 relay_sync_watermark 失败 dbnum={dbnum}: {error}")
        }
        Err(error) => log::warn!("[sync-ledger] 写 relay_sync_watermark 任务异常: {error}"),
    }
}

/// 同步版本。
pub fn upsert_watermark_blocking(dbnum: u32, file_name: &str, sesno: u32) -> rusqlite::Result<()> {
    let conn = open()?;
    set_watermark_in(&conn, dbnum, file_name, sesno, None)
}

// ---------------------------------------------------------------------------
// 中继水位（relay_sync 轮询读写；P3）
// ---------------------------------------------------------------------------

/// `relay_sync_watermark` 的一行。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Watermark {
    pub dbnum: u32,
    pub file_name: String,
    /// 已广播 / 已接收 / 基线的 latest sesno。
    pub sesno: u32,
    /// 上一轮看到的文件指纹 `"{mtime_nanos}:{size}"`（去抖用）；收包端写入后为 None。
    pub last_seen_fingerprint: Option<String>,
}

/// 读出全部中继水位（中继轮询每轮一次）。
pub fn load_watermarks_blocking() -> rusqlite::Result<std::collections::BTreeMap<u32, Watermark>> {
    let conn = open()?;
    load_watermarks_in(&conn)
}

pub fn load_watermarks_in(
    conn: &Connection,
) -> rusqlite::Result<std::collections::BTreeMap<u32, Watermark>> {
    let mut stmt = conn.prepare(
        "SELECT dbnum, file_name, sesno, last_seen_fingerprint FROM relay_sync_watermark",
    )?;
    let rows = stmt.query_map([], |row| {
        Ok(Watermark {
            dbnum: row.get(0)?,
            file_name: row.get(1)?,
            sesno: row.get(2)?,
            last_seen_fingerprint: row.get(3)?,
        })
    })?;
    let mut out = std::collections::BTreeMap::new();
    for row in rows {
        let watermark = row?;
        out.insert(watermark.dbnum, watermark);
    }
    Ok(out)
}

/// 写 / 推进某 dbnum 的中继水位，并记下本轮指纹（基线、空 diff、广播成功三处都走这里）。
pub fn set_watermark_blocking(
    dbnum: u32,
    file_name: &str,
    sesno: u32,
    fingerprint: Option<&str>,
) -> rusqlite::Result<()> {
    let conn = open()?;
    set_watermark_in(&conn, dbnum, file_name, sesno, fingerprint)
}

pub fn set_watermark_in(
    conn: &Connection,
    dbnum: u32,
    file_name: &str,
    sesno: u32,
    fingerprint: Option<&str>,
) -> rusqlite::Result<()> {
    conn.execute(
        "INSERT INTO relay_sync_watermark (dbnum, file_name, sesno, last_seen_fingerprint, updated_at)
         VALUES (?1, ?2, ?3, ?4, ?5)
         ON CONFLICT(dbnum) DO UPDATE SET
            file_name = excluded.file_name,
            sesno = excluded.sesno,
            last_seen_fingerprint = excluded.last_seen_fingerprint,
            updated_at = excluded.updated_at",
        params![dbnum, file_name, sesno, fingerprint, chrono::Utc::now().to_rfc3339()],
    )?;
    Ok(())
}

/// 只更新去抖指纹，水位不动（文件在两轮之间还在变，等它停下来）。没有该 dbnum 的行时什么都不做。
pub fn touch_watermark_fingerprint_blocking(dbnum: u32, fingerprint: &str) -> rusqlite::Result<()> {
    let conn = open()?;
    touch_watermark_fingerprint_in(&conn, dbnum, fingerprint)
}

pub fn touch_watermark_fingerprint_in(
    conn: &Connection,
    dbnum: u32,
    fingerprint: &str,
) -> rusqlite::Result<()> {
    conn.execute(
        "UPDATE relay_sync_watermark SET last_seen_fingerprint = ?2, updated_at = ?3 WHERE dbnum = ?1",
        params![dbnum, fingerprint, chrono::Utc::now().to_rfc3339()],
    )?;
    Ok(())
}

// ---------------------------------------------------------------------------
// hash / msg_id
// ---------------------------------------------------------------------------

/// 一段字节的 Blake2b512，小写 hex——与 `pdms_io::sync::compress` 产出的 `file_hashes` 同算法同编码。
pub fn blake2b512_hex(bytes: &[u8]) -> String {
    let mut hasher = Blake2b512::new();
    hasher.update(bytes);
    hex::encode(hasher.finalize())
}

/// 整个文件的 Blake2b512，小写 hex（4 MiB 缓冲流式读，源 db 文件可能有几百 MB）。
pub fn blake2b512_file_hex(path: &Path) -> std::io::Result<String> {
    use std::io::Read;
    let mut file = std::fs::File::open(path)?;
    let mut hasher = Blake2b512::new();
    let mut buffer = vec![0u8; 4 * 1024 * 1024];
    loop {
        let read = file.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    Ok(hex::encode(hasher.finalize()))
}

/// 消息分组键：payload 字节的 Blake2b512 前 16 字节 hex（32 个字符）。
/// 发布端对发出的字节、订阅端对收到的字节各算一次，同一条消息两端得到同一个 id。
pub fn msg_id_from_bytes(payload: &[u8]) -> String {
    let mut hasher = Blake2b512::new();
    hasher.update(payload);
    hex::encode(&hasher.finalize()[..16])
}

/// 没有原始字节时（如手工触发的 clone）从结构体重新序列化得到；与线格式一致时等于 [`msg_id_from_bytes`]。
pub fn msg_id_for(msg: &SyncE3dFileMsg) -> String {
    msg_id_from_bytes(&serde_json::to_vec(msg).unwrap_or_default())
}

// ---------------------------------------------------------------------------
// 收包端校验
// ---------------------------------------------------------------------------

/// 对单个 clone 后文件的校验结论。
#[derive(Debug, Clone)]
pub struct InboundVerdict {
    pub verify_status: VerifyStatus,
    pub verify_detail: Option<String>,
    /// e3d-io 实际读到的 latest sesno（文件能打开时才有）。
    pub sesno_seen: Option<u32>,
    /// 实际算出的 Blake2b512（消息带了 hash 且文件可读时才有）。
    pub actual_hash: Option<String>,
}

impl InboundVerdict {
    /// 文件在盘上是一个能打开的 E3D 库、且没有硬性不一致——可以拿 `sesno_seen` 当水位。
    pub fn file_is_trustworthy(&self) -> bool {
        self.sesno_seen.is_some()
            && matches!(self.verify_status, VerifyStatus::Ok | VerifyStatus::Skipped)
    }
}

enum E3dProbe {
    /// 本构建没有 e3d-io。
    Unavailable(&'static str),
    #[allow(dead_code)]
    Opened { latest_sesno: Option<u32> },
    #[allow(dead_code)]
    Failed(String),
}

/// 这个后端不带 e3d-io：会话链校验只有中继站点做，而中继已经搬到 `../plant-web-server`
/// （2026-09-16）。收包端因此只比 hash，sesno 一项记 `skipped`。
fn probe_e3d(_path: &Path) -> E3dProbe {
    E3dProbe::Unavailable("e3d-io 未编入本构建（会话链校验在 plant-web-server 的中继里做）")
}

fn short_hash(hash: &str) -> &str {
    &hash[..hash.len().min(16)]
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

/// 收包端：clone 之后对单个文件做校验（阻塞版）。
///
/// 顺序：消息带了 hash 就先比 Blake2b512（不等 → `hash_mismatch`，但仍尽力读一次 sesno 供排障）；
/// 再用 e3d-io 打开并读会话链（失败 → `open_failed`）；消息声明了 sesno 就比对（不等 → `sesno_mismatch`）。
/// 消息没声明 sesno（旧发送端）或本构建没有 e3d-io 时给 `skipped`，`verify_detail` 记下哪些项过了。
pub fn verify_cloned_file_blocking(
    path: &Path,
    expected_hash: Option<&str>,
    declared_sesno: Option<u32>,
) -> InboundVerdict {
    let mut actual_hash = None;
    let hash_note: &str = match expected_hash {
        None => "hash=not_declared",
        Some(expected) => match blake2b512_file_hex(path) {
            Err(error) => {
                return InboundVerdict {
                    verify_status: VerifyStatus::OpenFailed,
                    verify_detail: Some(truncate_detail(format!(
                        "read_failed: {error} ({})",
                        path.display()
                    ))),
                    sesno_seen: None,
                    actual_hash: None,
                };
            }
            Ok(actual) => {
                let matched = actual.eq_ignore_ascii_case(expected.trim());
                let detail = format!(
                    "hash_mismatch: expected={}… actual={}…",
                    short_hash(expected.trim()),
                    short_hash(&actual)
                );
                actual_hash = Some(actual);
                if !matched {
                    // 源文件已被 clone 覆盖，本期只记录不回滚；顺手读一次 sesno 供排障。
                    let sesno_seen = match probe_e3d(path) {
                        E3dProbe::Opened { latest_sesno } => latest_sesno,
                        _ => None,
                    };
                    return InboundVerdict {
                        verify_status: VerifyStatus::HashMismatch,
                        verify_detail: Some(detail),
                        sesno_seen,
                        actual_hash,
                    };
                }
                "hash=ok"
            }
        },
    };

    match probe_e3d(path) {
        E3dProbe::Failed(error) => InboundVerdict {
            verify_status: VerifyStatus::OpenFailed,
            verify_detail: Some(truncate_detail(format!("e3d-io {error}; {hash_note}"))),
            sesno_seen: None,
            actual_hash,
        },
        E3dProbe::Unavailable(why) => InboundVerdict {
            verify_status: VerifyStatus::Skipped,
            verify_detail: Some(format!("{why}; {hash_note}")),
            sesno_seen: None,
            actual_hash,
        },
        E3dProbe::Opened { latest_sesno: None } => InboundVerdict {
            verify_status: VerifyStatus::OpenFailed,
            verify_detail: Some(format!("e3d-io 会话链为空; {hash_note}")),
            sesno_seen: None,
            actual_hash,
        },
        E3dProbe::Opened {
            latest_sesno: Some(seen),
        } => match declared_sesno {
            Some(declared) if declared != seen => InboundVerdict {
                verify_status: VerifyStatus::SesnoMismatch,
                verify_detail: Some(format!(
                    "sesno_mismatch: declared={declared} seen={seen}; {hash_note}"
                )),
                sesno_seen: Some(seen),
                actual_hash,
            },
            Some(_) => InboundVerdict {
                verify_status: VerifyStatus::Ok,
                verify_detail: (expected_hash.is_none()).then(|| hash_note.to_string()),
                sesno_seen: Some(seen),
                actual_hash,
            },
            None => InboundVerdict {
                verify_status: VerifyStatus::Skipped,
                verify_detail: Some(format!(
                    "file_sesnos 缺失（旧发送端）; {hash_note}; open=ok sesno_seen={seen}"
                )),
                sesno_seen: Some(seen),
                actual_hash,
            },
        },
    }
}

/// [`verify_cloned_file_blocking`] 的异步包装（hash 与 e3d-io 都是同步 IO，放 `spawn_blocking`）。
pub async fn verify_cloned_file(
    path: PathBuf,
    expected_hash: Option<String>,
    declared_sesno: Option<u32>,
) -> InboundVerdict {
    match tokio::task::spawn_blocking(move || {
        verify_cloned_file_blocking(&path, expected_hash.as_deref(), declared_sesno)
    })
    .await
    {
        Ok(verdict) => verdict,
        Err(error) => InboundVerdict {
            verify_status: VerifyStatus::Skipped,
            verify_detail: Some(format!("verify task panicked: {error}")),
            sesno_seen: None,
            actual_hash: None,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn blake2b512_matches_known_vectors() {
        // RFC 7693 / 官方测试向量
        assert_eq!(
            blake2b512_hex(b""),
            "786a02f742015903c6c6fd852552d272912f4740e15847618a86e217f71f5419d25e1031afee585313896444934eb04b903a685b1448b755d56f701afe9be2ce"
        );
        assert_eq!(
            blake2b512_hex(b"abc"),
            "ba80a53f981c4d0d6a2797b69f12f6e94c212f14685ac4b74b12bb6fdbffa2d17d87c5392aab792dc252d5de4533cc9518d38aa8dbf1925ab92386edd4009923"
        );
        assert_eq!(msg_id_from_bytes(b"abc"), "ba80a53f981c4d0d6a2797b69f12f6e9");
        assert_eq!(msg_id_from_bytes(b"abc").len(), 32);
    }

    #[test]
    fn file_hash_equals_bytes_hash() {
        let dir = std::env::temp_dir().join(format!("sync-ledger-test-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("sample.bin");
        let payload: Vec<u8> = (0..(5 * 1024 * 1024)).map(|i| (i % 251) as u8).collect();
        std::fs::write(&file, &payload).unwrap();
        assert_eq!(blake2b512_file_hex(&file).unwrap(), blake2b512_hex(&payload));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn schema_and_rows_round_trip_in_memory() {
        let mut conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(SCHEMA_SQL).unwrap();
        // 幂等
        conn.execute_batch(SCHEMA_SQL).unwrap();

        let mut row = LedgerRow::new("m1", Direction::Outbound, "local-a", "ams8000_0001", VerifyStatus::Ok);
        row.file_hash = Some("ab".repeat(64));
        row.sesno_from = Some(10);
        row.sesno_to = Some(12);
        row.sesno_seen = Some(12);
        row.diff = Some(DiffTally { inserted: 1, deleted: 2, modified: 3 });
        row.diff_status = Some(DiffStatus::Ok);
        row.msg_timestamp = Some("2026-09-15T00:00:00+00:00".to_string());
        row.changes = (0..(MAX_CHANGES_PER_ROW + 5))
            .map(|i| ChangeEntry { refno: format!("8000/{i}"), kind: "modified" })
            .collect();

        let mut skipped = LedgerRow::new("m1", Direction::Inbound, "local-b", "ams8000_0001", VerifyStatus::Skipped);
        skipped.verify_detail = Some("file_sesnos 缺失（旧发送端）".to_string());

        let tx = conn.transaction().unwrap();
        insert_row(&tx, &row).unwrap();
        insert_row(&tx, &skipped).unwrap();
        tx.commit().unwrap();

        let (count, truncated): (i64, String) = conn
            .query_row(
                "SELECT (SELECT COUNT(*) FROM e3d_sync_changes), verify_detail FROM e3d_sync_ledger WHERE direction='outbound'",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        assert_eq!(count as usize, MAX_CHANGES_PER_ROW);
        assert_eq!(truncated, format!("changes_truncated:{}", MAX_CHANGES_PER_ROW + 5));

        let statuses: Vec<(String, String, Option<i64>)> = conn
            .prepare("SELECT direction, verify_status, sesno_to FROM e3d_sync_ledger ORDER BY direction")
            .unwrap()
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap();
        assert_eq!(
            statuses,
            vec![
                ("inbound".to_string(), "skipped".to_string(), None),
                ("outbound".to_string(), "ok".to_string(), Some(12)),
            ]
        );

        // 水位 upsert：同 dbnum 第二次写覆盖 sesno 并清空指纹
        conn.execute(
            "INSERT INTO relay_sync_watermark (dbnum, file_name, sesno, last_seen_fingerprint, updated_at) VALUES (8000, 'ams8000_0001', 5, 'fp', 't0')",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO relay_sync_watermark (dbnum, file_name, sesno, last_seen_fingerprint, updated_at)
             VALUES (?1, ?2, ?3, NULL, ?4)
             ON CONFLICT(dbnum) DO UPDATE SET file_name = excluded.file_name, sesno = excluded.sesno,
                last_seen_fingerprint = NULL, updated_at = excluded.updated_at",
            params![8000u32, "ams8000_0001", 12u32, "t1"],
        )
        .unwrap();
        let (sesno, fp): (i64, Option<String>) = conn
            .query_row("SELECT sesno, last_seen_fingerprint FROM relay_sync_watermark WHERE dbnum=8000", [], |r| Ok((r.get(0)?, r.get(1)?)))
            .unwrap();
        assert_eq!((sesno, fp), (12, None));
    }

    #[test]
    fn relay_watermark_helpers_in_memory() {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(SCHEMA_SQL).unwrap();
        assert!(load_watermarks_in(&conn).unwrap().is_empty());

        // 基线：带指纹
        set_watermark_in(&conn, 7001, "acp7001_0001", 260, Some("1:100")).unwrap();
        // 收包端写法：指纹清空
        set_watermark_in(&conn, 8000, "ams8000_0001", 12, None).unwrap();
        let all = load_watermarks_in(&conn).unwrap();
        assert_eq!(all.len(), 2);
        assert_eq!(
            all[&7001],
            Watermark {
                dbnum: 7001,
                file_name: "acp7001_0001".into(),
                sesno: 260,
                last_seen_fingerprint: Some("1:100".into()),
            }
        );
        assert_eq!(all[&8000].last_seen_fingerprint, None);

        // 去抖：只动指纹
        touch_watermark_fingerprint_in(&conn, 7001, "2:200").unwrap();
        let wm = &load_watermarks_in(&conn).unwrap()[&7001];
        assert_eq!((wm.sesno, wm.last_seen_fingerprint.as_deref()), (260, Some("2:200")));
        // 不存在的 dbnum：no-op
        touch_watermark_fingerprint_in(&conn, 9999, "x").unwrap();
        assert_eq!(load_watermarks_in(&conn).unwrap().len(), 2);

        // 推进：覆盖 sesno 与指纹
        set_watermark_in(&conn, 7001, "acp7001_0001", 270, Some("3:300")).unwrap();
        let wm = &load_watermarks_in(&conn).unwrap()[&7001];
        assert_eq!((wm.sesno, wm.last_seen_fingerprint.as_deref()), (270, Some("3:300")));
    }

    #[test]
    fn verdict_for_non_e3d_file() {
        let dir = std::env::temp_dir().join(format!("sync-ledger-verdict-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("not_a_db_0001");
        std::fs::write(&file, b"definitely not an e3d database").unwrap();
        let good_hash = blake2b512_hex(b"definitely not an e3d database");

        // hash 不对 → hash_mismatch，优先于打不开
        let v = verify_cloned_file_blocking(&file, Some("00ff"), Some(3));
        assert_eq!(v.verify_status, VerifyStatus::HashMismatch);
        assert_eq!(v.actual_hash.as_deref(), Some(good_hash.as_str()));

        // hash 对、不是 E3D 库 → 本构建没有 e3d-io，sesno 一项记 skipped
        let v = verify_cloned_file_blocking(&file, Some(&good_hash.to_uppercase()), Some(3));
        assert_eq!(v.verify_status, VerifyStatus::Skipped);
        assert!(v.verify_detail.as_deref().unwrap().contains("hash=ok"));

        // 文件不存在 → open_failed(read_failed)
        let v = verify_cloned_file_blocking(&dir.join("missing"), Some(&good_hash), None);
        assert_eq!(v.verify_status, VerifyStatus::OpenFailed);
        assert!(v.verify_detail.as_deref().unwrap().starts_with("read_failed"));

        let _ = std::fs::remove_dir_all(&dir);
    }
}
