use std::str::FromStr;

use aios_core::{RefnoEnum, project_primary_db};
use anyhow::{Context, bail};
use serde::{Deserialize, Serialize};
use surrealdb::types::SurrealValue;

use crate::data_interface::increment_record::IncrGeoUpdateLog;

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
    #[serde(default)]
    pub created_at: Option<String>,
    #[serde(default)]
    pub consumed_at: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelGenDebtWriteOutcome {
    pub dbnum: u32,
    pub from_sesno: u32,
    pub to_sesno: u32,
    pub idempotent: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelGenDebtRange {
    pub from_sesno: u32,
    pub to_sesno: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelGenDebtCoverage {
    pub dbnum: u32,
    pub data_watermark: u32,
    pub model_generation_watermark: u32,
    pub debt_ranges: Vec<ModelGenDebtRange>,
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
        created_at: None,
        consumed_at: None,
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
         loop_owner_refnos, bran_hanger_refnos, basic_cata_refnos, delete_refnos, \
         created_at, consumed_at FROM model_gen_debt:[{dbnum}, {to_sesno}];"
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
        bail!(
            "immutable model_gen_debt conflict: dbnum={dbnum} to_sesno={to_sesno}"
        );
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

pub async fn model_generation_watermark(dbnum: u32) -> anyhow::Result<u32> {
    super::database::ensure_sesno_version_anchor_schema().await?;
    let sql = format!(
        "math::max(array::flatten([SELECT VALUE sesno FROM sesno_version_anchor \
         WHERE dbnum = {dbnum} AND source = 'model_gen']));"
    );
    let mut response = project_primary_db().query(sql).await?.check()?;
    Ok(response.take::<Option<u32>>(0)?.unwrap_or_default())
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

pub async fn analyze_model_gen_debt(dbnum: u32) -> anyhow::Result<ModelGenDebtCoverage> {
    ensure_model_gen_debt_schema().await?;
    let data_watermark = super::version_commit::committed_watermark(dbnum).await?;
    let model_generation_watermark = model_generation_watermark(dbnum).await?;
    let sql = format!(
        "SELECT dbnum, from_sesno, to_sesno, commit_fingerprint, prim_refnos, \
         loop_owner_refnos, bran_hanger_refnos, basic_cata_refnos, delete_refnos, \
         created_at, consumed_at FROM model_gen_debt \
         WHERE dbnum = {dbnum} AND consumed_at = NONE AND to_sesno <= {data_watermark} \
         ORDER BY to_sesno ASC;"
    );
    let mut response = project_primary_db().query(sql).await?.check()?;
    let debts: Vec<ModelGenDebtRecord> = response.take(0)?;

    let mut cursor = model_generation_watermark;
    let mut debt_ranges = Vec::new();
    let mut merged_update_log = IncrGeoUpdateLog::default();
    for debt in debts {
        if debt.to_sesno <= cursor {
            continue;
        }
        if debt.from_sesno > cursor.saturating_add(1) {
            break;
        }
        extend_bucket(&mut merged_update_log.prim_refnos, &debt.prim_refnos)?;
        extend_bucket(
            &mut merged_update_log.loop_owner_refnos,
            &debt.loop_owner_refnos,
        )?;
        extend_bucket(
            &mut merged_update_log.bran_hanger_refnos,
            &debt.bran_hanger_refnos,
        )?;
        extend_bucket(
            &mut merged_update_log.basic_cata_refnos,
            &debt.basic_cata_refnos,
        )?;
        extend_bucket(&mut merged_update_log.delete_refnos, &debt.delete_refnos)?;
        debt_ranges.push(ModelGenDebtRange {
            from_sesno: debt.from_sesno,
            to_sesno: debt.to_sesno,
        });
        cursor = cursor.max(debt.to_sesno);
    }
    let coverage_complete = model_generation_watermark >= data_watermark
        || (data_watermark > 0 && cursor >= data_watermark);
    Ok(ModelGenDebtCoverage {
        dbnum,
        data_watermark,
        model_generation_watermark,
        debt_ranges,
        coverage_complete,
        needs_full_regen: data_watermark > model_generation_watermark && !coverage_complete,
        merged_update_log,
    })
}

pub async fn finalize_model_generation(
    dbnum: u32,
    target_sesno: u32,
) -> anyhow::Result<super::version_commit::ModelGenAnchor> {
    ensure_model_gen_debt_schema().await?;
    super::database::ensure_sesno_version_anchor_schema().await?;
    let sql = format!(
        r#"
BEGIN TRANSACTION;
UPSERT sesno_version_anchor:[{dbnum}, {target_sesno}, 'model_gen'] SET
    dbnum = {dbnum}, sesno = {target_sesno}, source = 'model_gen',
    anchored_at = time::now(), note = 'model generation completed';
UPDATE model_gen_debt SET consumed_at = time::now()
    WHERE dbnum = {dbnum} AND to_sesno <= {target_sesno} AND consumed_at = NONE;
COMMIT TRANSACTION;
"#
    );
    project_primary_db().query(sql).await?.check()?;
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
    })
}
