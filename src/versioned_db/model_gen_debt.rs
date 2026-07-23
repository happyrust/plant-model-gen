use std::str::FromStr;

use aios_core::{RefnoEnum, project_primary_db};
use anyhow::{Context, bail};
use serde::{Deserialize, Serialize};
use surrealdb::types::SurrealValue;

use crate::data_interface::increment_record::IncrGeoUpdateLog;

pub const MODEL_GEN_DEBT_RANGE_SEMANTICS: &str = "[from_sesno,to_sesno]";

#[derive(Debug, Clone, Serialize, Deserialize, SurrealValue)]
pub struct ModelGenDebtRecord {
    pub dbnum: u32,
    pub from_sesno: u32,
    pub to_sesno: u32,
    pub commit_fingerprint: String,
    #[serde(default)]
    pub prim_refnos: Vec<String>,
    #[serde(default)]
    pub loop_owner_refnos: Vec<String>,
    #[serde(default)]
    pub bran_hanger_refnos: Vec<String>,
    #[serde(default)]
    pub basic_cata_refnos: Vec<String>,
    #[serde(default)]
    pub delete_refnos: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelGenDebtWriteOutcome {
    pub dbnum: u32,
    pub from_sesno: u32,
    pub to_sesno: u32,
    pub idempotent: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModelGenDebtBucketCounts {
    pub prim: usize,
    pub loop_owner: usize,
    pub bran_hanger: usize,
    pub basic_cata: usize,
    pub delete: usize,
    pub total: usize,
}

impl ModelGenDebtBucketCounts {
    fn from_record(record: &ModelGenDebtRecord) -> Self {
        let prim = record.prim_refnos.len();
        let loop_owner = record.loop_owner_refnos.len();
        let bran_hanger = record.bran_hanger_refnos.len();
        let basic_cata = record.basic_cata_refnos.len();
        let delete = record.delete_refnos.len();
        Self {
            prim,
            loop_owner,
            bran_hanger,
            basic_cata,
            delete,
            total: prim + loop_owner + bran_hanger + basic_cata + delete,
        }
    }

    fn from_log(log: &IncrGeoUpdateLog) -> Self {
        let prim = log.prim_refnos.len();
        let loop_owner = log.loop_owner_refnos.len();
        let bran_hanger = log.bran_hanger_refnos.len();
        let basic_cata = log.basic_cata_refnos.len();
        let delete = log.delete_refnos.len();
        Self {
            prim,
            loop_owner,
            bran_hanger,
            basic_cata,
            delete,
            total: prim + loop_owner + bran_hanger + basic_cata + delete,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelGenDebtRange {
    pub from_sesno: u32,
    pub to_sesno: u32,
    pub commit_fingerprint: String,
    pub bucket_counts: ModelGenDebtBucketCounts,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelGenDebtGap {
    pub missing_from_sesno: u32,
    pub missing_to_sesno: u32,
    pub next_debt_from_sesno: Option<u32>,
    pub next_debt_to_sesno: Option<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelGenDebtCoverage {
    pub dbnum: u32,
    pub data_watermark: u32,
    pub model_generation_watermark: u32,
    pub range_semantics: String,
    /// 全部尚未消费且不高于 data watermark 的 debt 行，包括已被模型水位覆盖的遗留行。
    pub debt_ranges: Vec<ModelGenDebtRange>,
    /// 从 model watermark 开始连续可消费的前缀。
    pub consumable_debt_ranges: Vec<ModelGenDebtRange>,
    /// 尚未标记 consumed、但已被 model watermark 覆盖的遗留行。
    pub stale_debt_ranges: Vec<ModelGenDebtRange>,
    /// 所有未覆盖区间；不会因首个 gap 而隐藏后续 debt。
    pub gap_ranges: Vec<ModelGenDebtGap>,
    /// 全部存活 debt（含尚未整理的 stale 行）的五桶去重规模。
    pub debt_bucket_counts: ModelGenDebtBucketCounts,
    /// 连续可消费前缀合并后的五桶去重规模。
    pub consumable_bucket_counts: ModelGenDebtBucketCounts,
    pub coverage_complete: bool,
    pub needs_full_regen: bool,
    pub merged_update_log: IncrGeoUpdateLog,
}

fn normalized_refnos(values: impl IntoIterator<Item = RefnoEnum>) -> Vec<String> {
    let mut values = values
        .into_iter()
        .filter(RefnoEnum::is_valid)
        .map(|value| value.to_string())
        .collect::<Vec<_>>();
    values.sort();
    values.dedup();
    values
}

fn record_from_log(
    dbnum: u32,
    from_sesno: u32,
    to_sesno: u32,
    commit_fingerprint: &str,
    log: &IncrGeoUpdateLog,
) -> ModelGenDebtRecord {
    ModelGenDebtRecord {
        dbnum,
        from_sesno,
        to_sesno,
        commit_fingerprint: commit_fingerprint.to_string(),
        prim_refnos: normalized_refnos(log.prim_refnos.iter().copied()),
        loop_owner_refnos: normalized_refnos(log.loop_owner_refnos.iter().copied()),
        bran_hanger_refnos: normalized_refnos(log.bran_hanger_refnos.iter().copied()),
        basic_cata_refnos: normalized_refnos(log.basic_cata_refnos.iter().copied()),
        delete_refnos: normalized_refnos(log.delete_refnos.iter().copied()),
    }
}

fn range_from_record(record: &ModelGenDebtRecord) -> ModelGenDebtRange {
    ModelGenDebtRange {
        from_sesno: record.from_sesno,
        to_sesno: record.to_sesno,
        commit_fingerprint: record.commit_fingerprint.clone(),
        bucket_counts: ModelGenDebtBucketCounts::from_record(record),
    }
}

fn same_payload(left: &ModelGenDebtRecord, right: &ModelGenDebtRecord) -> bool {
    left.dbnum == right.dbnum
        && left.from_sesno == right.from_sesno
        && left.to_sesno == right.to_sesno
        && left.commit_fingerprint == right.commit_fingerprint
        && left.prim_refnos == right.prim_refnos
        && left.loop_owner_refnos == right.loop_owner_refnos
        && left.bran_hanger_refnos == right.bran_hanger_refnos
        && left.basic_cata_refnos == right.basic_cata_refnos
        && left.delete_refnos == right.delete_refnos
}

pub async fn ensure_model_gen_debt_schema() -> anyhow::Result<()> {
    let sql = r#"
DEFINE TABLE IF NOT EXISTS model_gen_debt SCHEMAFULL;
DEFINE FIELD IF NOT EXISTS dbnum ON TABLE model_gen_debt TYPE int;
DEFINE FIELD IF NOT EXISTS from_sesno ON TABLE model_gen_debt TYPE int;
DEFINE FIELD IF NOT EXISTS to_sesno ON TABLE model_gen_debt TYPE int;
DEFINE FIELD IF NOT EXISTS commit_fingerprint ON TABLE model_gen_debt TYPE string;
DEFINE FIELD IF NOT EXISTS prim_refnos ON TABLE model_gen_debt TYPE array<string> DEFAULT [];
DEFINE FIELD IF NOT EXISTS loop_owner_refnos ON TABLE model_gen_debt TYPE array<string> DEFAULT [];
DEFINE FIELD IF NOT EXISTS bran_hanger_refnos ON TABLE model_gen_debt TYPE array<string> DEFAULT [];
DEFINE FIELD IF NOT EXISTS basic_cata_refnos ON TABLE model_gen_debt TYPE array<string> DEFAULT [];
DEFINE FIELD IF NOT EXISTS delete_refnos ON TABLE model_gen_debt TYPE array<string> DEFAULT [];
DEFINE FIELD IF NOT EXISTS created_at ON TABLE model_gen_debt TYPE datetime DEFAULT time::now();
DEFINE FIELD IF NOT EXISTS consumed_at ON TABLE model_gen_debt TYPE option<datetime>;
DEFINE INDEX IF NOT EXISTS idx_model_gen_debt_lookup ON TABLE model_gen_debt FIELDS dbnum, consumed_at, to_sesno;
"#;
    project_primary_db()
        .query(sql)
        .await
        .context("define model_gen_debt schema")?
        .check()
        .context("check model_gen_debt schema statements")?;
    Ok(())
}

pub async fn write_model_gen_debt(
    dbnum: u32,
    from_sesno: u32,
    to_sesno: u32,
    commit_fingerprint: &str,
    log: &IncrGeoUpdateLog,
) -> anyhow::Result<ModelGenDebtWriteOutcome> {
    ensure_model_gen_debt_schema().await?;
    let requested = record_from_log(dbnum, from_sesno, to_sesno, commit_fingerprint, log);
    let select = format!(
        "SELECT dbnum, from_sesno, to_sesno, commit_fingerprint, prim_refnos, \
         loop_owner_refnos, bran_hanger_refnos, basic_cata_refnos, delete_refnos \
         FROM model_gen_debt:[{dbnum}, {to_sesno}];"
    );
    let mut response = project_primary_db().query(select).await?.check()?;
    let existing: Vec<ModelGenDebtRecord> = response.take(0)?;
    if let Some(existing) = existing.first() {
        if same_payload(existing, &requested) {
            return Ok(ModelGenDebtWriteOutcome {
                dbnum,
                from_sesno,
                to_sesno,
                idempotent: true,
            });
        }
        bail!("immutable model_gen_debt conflict: dbnum={dbnum} to_sesno={to_sesno}");
    }

    let sql = format!(
        "CREATE ONLY model_gen_debt:[{dbnum}, {to_sesno}] SET \
         dbnum = {dbnum}, from_sesno = {from_sesno}, to_sesno = {to_sesno}, \
         commit_fingerprint = $commit_fingerprint, prim_refnos = $prim_refnos, \
         loop_owner_refnos = $loop_owner_refnos, bran_hanger_refnos = $bran_hanger_refnos, \
         basic_cata_refnos = $basic_cata_refnos, delete_refnos = $delete_refnos, \
         created_at = time::now(), consumed_at = NONE;"
    );
    project_primary_db()
        .query(sql)
        .bind(("commit_fingerprint", requested.commit_fingerprint))
        .bind(("prim_refnos", requested.prim_refnos))
        .bind(("loop_owner_refnos", requested.loop_owner_refnos))
        .bind(("bran_hanger_refnos", requested.bran_hanger_refnos))
        .bind(("basic_cata_refnos", requested.basic_cata_refnos))
        .bind(("delete_refnos", requested.delete_refnos))
        .await?
        .check()?;
    Ok(ModelGenDebtWriteOutcome {
        dbnum,
        from_sesno,
        to_sesno,
        idempotent: false,
    })
}

/// 生成幂等的 `model_gen_debt` UPSERT 语句，供增量 `commit_version` 的 apply
/// 闭包内**与数据写入同一提交保护域**执行（消除"数据 commit 成功、debt 未写"
/// 的崩溃窗口）。
///
/// 与 `write_model_gen_debt` 的 CREATE-once + payload 冲突检查不同：这里用
/// UPSERT 且**不触碰 `created_at`/`consumed_at`**——新建行靠 schema 默认（
/// `created_at` DEFAULT、`consumed_at` 为 NONE），recover 重放已存在行时保留原值，
/// 从而在 `preparing → apply → committed` 状态机内可安全重放。apply 闭包用拼接 SQL
/// 无 bind，故 refnos/fingerprint 内联为字面量（refno 为安全字符，fingerprint 为 hex）。
pub fn debt_upsert_sql(
    dbnum: u32,
    from_sesno: u32,
    to_sesno: u32,
    commit_fingerprint: &str,
    log: &IncrGeoUpdateLog,
) -> String {
    let record = record_from_log(dbnum, from_sesno, to_sesno, commit_fingerprint, log);
    let fp = record.commit_fingerprint.replace('\'', "\\'");
    let fmt = |values: &[String]| -> String {
        let inner = values
            .iter()
            .map(|value| format!("'{}'", value.replace('\'', "\\'")))
            .collect::<Vec<_>>()
            .join(", ");
        format!("[{inner}]")
    };
    format!(
        "UPSERT model_gen_debt:[{dbnum}, {to_sesno}] SET \
         dbnum = {dbnum}, from_sesno = {from_sesno}, to_sesno = {to_sesno}, \
         commit_fingerprint = '{fp}', prim_refnos = {}, loop_owner_refnos = {}, \
         bran_hanger_refnos = {}, basic_cata_refnos = {}, delete_refnos = {};",
        fmt(&record.prim_refnos),
        fmt(&record.loop_owner_refnos),
        fmt(&record.bran_hanger_refnos),
        fmt(&record.basic_cata_refnos),
        fmt(&record.delete_refnos),
    )
}

pub async fn model_generation_watermark(dbnum: u32) -> anyhow::Result<u32> {
    super::database::ensure_sesno_version_anchor_schema().await?;
    let sql = format!(
        "math::max(array::flatten([SELECT VALUE sesno FROM sesno_version_anchor \
         WHERE dbnum = {dbnum} AND source = 'model_gen']));"
    );
    let mut response = project_primary_db().query(sql).await?.check()?;
    let value = response.take::<surrealdb::types::Value>(0)?;
    Ok(
        super::version_commit::optional_u32_from_value(value, "model generation watermark")?
            .unwrap_or_default(),
    )
}

pub async fn list_model_gen_candidate_dbnums() -> anyhow::Result<Vec<u32>> {
    ensure_model_gen_debt_schema().await?;
    super::database::ensure_sesno_version_anchor_schema().await?;
    let sql = r#"
SELECT VALUE dbnum FROM sesno_version_anchor
    WHERE source IN ['full', 'incremental_baseline', 'incremental'];
SELECT VALUE dbnum FROM model_gen_debt;
"#;
    let mut response = project_primary_db().query(sql).await?.check()?;
    let mut dbnums = std::collections::BTreeSet::new();
    for idx in 0..2 {
        let values: Vec<i64> = response.take(idx)?;
        dbnums.extend(
            values
                .into_iter()
                .filter_map(|value| u32::try_from(value).ok())
                .filter(|value| *value > 0),
        );
    }
    Ok(dbnums.into_iter().collect())
}

pub async fn reconcile_model_gen_debt_covered_by_watermark(
    dbnum: u32,
    model_generation_watermark: u32,
) -> anyhow::Result<()> {
    ensure_model_gen_debt_schema().await?;
    if model_generation_watermark == 0 {
        return Ok(());
    }
    let sql = format!(
        "UPDATE model_gen_debt SET consumed_at = time::now() \
         WHERE dbnum = {dbnum} AND consumed_at = NONE \
         AND to_sesno <= {model_generation_watermark};"
    );
    project_primary_db().query(sql).await?.check()?;
    Ok(())
}

fn extend_bucket(
    target: &mut std::collections::HashSet<RefnoEnum>,
    values: &[String],
) -> anyhow::Result<()> {
    for value in values {
        target.insert(
            RefnoEnum::from_str(value)
                .map_err(|_| anyhow::anyhow!("invalid refno in model_gen_debt: {value}"))?,
        );
    }
    Ok(())
}

fn extend_log(target: &mut IncrGeoUpdateLog, debt: &ModelGenDebtRecord) -> anyhow::Result<()> {
    extend_bucket(&mut target.prim_refnos, &debt.prim_refnos)?;
    extend_bucket(&mut target.loop_owner_refnos, &debt.loop_owner_refnos)?;
    extend_bucket(&mut target.bran_hanger_refnos, &debt.bran_hanger_refnos)?;
    extend_bucket(&mut target.basic_cata_refnos, &debt.basic_cata_refnos)?;
    extend_bucket(&mut target.delete_refnos, &debt.delete_refnos)?;
    Ok(())
}

pub async fn analyze_model_gen_debt(dbnum: u32) -> anyhow::Result<ModelGenDebtCoverage> {
    ensure_model_gen_debt_schema().await?;
    let data_watermark = super::version_commit::committed_watermark(dbnum).await?;
    let model_generation_watermark = model_generation_watermark(dbnum).await?;
    let sql = format!(
        "SELECT dbnum, from_sesno, to_sesno, commit_fingerprint, prim_refnos, \
         loop_owner_refnos, bran_hanger_refnos, basic_cata_refnos, delete_refnos \
         FROM model_gen_debt \
         WHERE dbnum = {dbnum} AND consumed_at = NONE AND to_sesno <= {data_watermark} \
         ORDER BY from_sesno ASC, to_sesno ASC;"
    );
    let mut response = project_primary_db().query(sql).await?.check()?;
    let mut debts: Vec<ModelGenDebtRecord> = response.take(0)?;
    debts.sort_by_key(|debt| (debt.from_sesno, debt.to_sesno));

    let debt_ranges = debts.iter().map(range_from_record).collect::<Vec<_>>();
    let stale_debt_ranges = debts
        .iter()
        .filter(|debt| debt.to_sesno <= model_generation_watermark)
        .map(range_from_record)
        .collect::<Vec<_>>();
    let mut all_live_log = IncrGeoUpdateLog::default();
    for debt in &debts {
        extend_log(&mut all_live_log, debt)?;
    }
    let pending_debts = debts
        .iter()
        .filter(|debt| debt.to_sesno > model_generation_watermark)
        .collect::<Vec<_>>();

    let mut observed_cursor = model_generation_watermark;
    let mut gap_ranges = Vec::new();
    for debt in &pending_debts {
        if debt.from_sesno > observed_cursor.saturating_add(1) {
            gap_ranges.push(ModelGenDebtGap {
                missing_from_sesno: observed_cursor.saturating_add(1),
                missing_to_sesno: debt.from_sesno.saturating_sub(1),
                next_debt_from_sesno: Some(debt.from_sesno),
                next_debt_to_sesno: Some(debt.to_sesno),
            });
        }
        observed_cursor = observed_cursor.max(debt.to_sesno);
    }
    if observed_cursor < data_watermark {
        gap_ranges.push(ModelGenDebtGap {
            missing_from_sesno: observed_cursor.saturating_add(1),
            missing_to_sesno: data_watermark,
            next_debt_from_sesno: None,
            next_debt_to_sesno: None,
        });
    }

    let mut consumable_cursor = model_generation_watermark;
    let mut consumable_debt_ranges = Vec::new();
    let mut merged_update_log = IncrGeoUpdateLog::default();
    for debt in pending_debts {
        if debt.from_sesno > consumable_cursor.saturating_add(1) {
            break;
        }
        extend_log(&mut merged_update_log, debt)?;
        consumable_debt_ranges.push(range_from_record(debt));
        consumable_cursor = consumable_cursor.max(debt.to_sesno);
    }
    let coverage_complete = model_generation_watermark >= data_watermark
        || (data_watermark > 0 && consumable_cursor >= data_watermark);
    Ok(ModelGenDebtCoverage {
        dbnum,
        data_watermark,
        model_generation_watermark,
        range_semantics: MODEL_GEN_DEBT_RANGE_SEMANTICS.to_string(),
        debt_ranges,
        consumable_debt_ranges,
        stale_debt_ranges,
        gap_ranges,
        debt_bucket_counts: ModelGenDebtBucketCounts::from_log(&all_live_log),
        consumable_bucket_counts: ModelGenDebtBucketCounts::from_log(&merged_update_log),
        coverage_complete,
        needs_full_regen: data_watermark > model_generation_watermark && !coverage_complete,
        merged_update_log,
    })
}

pub async fn finalize_model_generation(
    dbnum: u32,
    target_sesno: u32,
    note: &str,
) -> anyhow::Result<super::version_commit::ModelGenAnchor> {
    ensure_model_gen_debt_schema().await?;
    super::database::ensure_sesno_version_anchor_schema().await?;
    let sql = format!(
        r#"
BEGIN TRANSACTION;
UPSERT sesno_version_anchor:[{dbnum}, {target_sesno}, 'model_gen'] SET
    dbnum = {dbnum}, sesno = {target_sesno}, source = 'model_gen',
    anchored_at = time::now(), note = $note;
UPDATE model_gen_debt SET consumed_at = time::now()
    WHERE dbnum = {dbnum} AND to_sesno <= {target_sesno} AND consumed_at = NONE;
COMMIT TRANSACTION;
"#
    );
    project_primary_db()
        .query(sql)
        .bind(("note", note.to_string()))
        .await?
        .check()?;
    let mut response = project_primary_db()
        .query(format!(
            "SELECT VALUE anchored_at FROM ONLY sesno_version_anchor:[{dbnum}, {target_sesno}, 'model_gen'];"
        ))
        .await?
        .check()?;
    let anchored_at: Option<surrealdb::types::Datetime> = response.take(0)?;
    let anchored_at = anchored_at
        .map(|value| value.to_string())
        .ok_or_else(|| anyhow::anyhow!("model_gen anchor returned no anchored_at"))?;
    Ok(super::version_commit::ModelGenAnchor {
        dbnum,
        sesno: target_sesno,
        source: "model_gen".to_string(),
        anchored_at,
        note: note.to_string(),
    })
}
