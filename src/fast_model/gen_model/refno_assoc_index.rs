use std::collections::{BTreeSet, HashSet};

use aios_core::{RefnoEnum, model_primary_db, SurrealQueryExt};
use anyhow::Context;
use serde_json::Value;
use tokio::sync::OnceCell;

use super::sql_file_writer::SqlFileWriter;

static REFNO_ASSOC_INDEX_SCHEMA_INIT: OnceCell<()> = OnceCell::const_new();

fn quote_sql_string(input: &str) -> String {
    input.replace('\\', "\\\\").replace('\'', "\\'")
}

fn array_to_sql_string(values: &BTreeSet<String>) -> String {
    if values.is_empty() {
        "[]".to_string()
    } else {
        let quoted = values
            .iter()
            .map(|v| format!("'{}'", quote_sql_string(v)))
            .collect::<Vec<_>>()
            .join(",");
        format!("[{}]", quoted)
    }
}

fn assoc_record_id(refno: RefnoEnum) -> String {
    format!("refno_assoc_index:⟨{}⟩", refno)
}

async fn ensure_refno_assoc_index_schema() {
    REFNO_ASSOC_INDEX_SCHEMA_INIT
        .get_or_init(|| async {
            let sql = r#"
DEFINE TABLE IF NOT EXISTS refno_assoc_index SCHEMALESS;
DEFINE FIELD IF NOT EXISTS refno ON TABLE refno_assoc_index TYPE record<pe>;
DEFINE FIELD IF NOT EXISTS inst_relate_ids ON TABLE refno_assoc_index TYPE array<string>;
DEFINE FIELD IF NOT EXISTS inst_info_ids ON TABLE refno_assoc_index TYPE array<string>;
DEFINE FIELD IF NOT EXISTS geo_relate_ids ON TABLE refno_assoc_index TYPE array<string>;
DEFINE FIELD IF NOT EXISTS neg_relate_ids ON TABLE refno_assoc_index TYPE array<string>;
DEFINE FIELD IF NOT EXISTS ngmr_relate_ids ON TABLE refno_assoc_index TYPE array<string>;
DEFINE FIELD IF NOT EXISTS inst_relate_bool_ids ON TABLE refno_assoc_index TYPE array<string>;
DEFINE FIELD IF NOT EXISTS inst_relate_cata_bool_ids ON TABLE refno_assoc_index TYPE array<string>;
DEFINE FIELD IF NOT EXISTS inst_relate_aabb_ids ON TABLE refno_assoc_index TYPE array<string>;
DEFINE FIELD IF NOT EXISTS inst_relate_booled_aabb_ids ON TABLE refno_assoc_index TYPE array<string>;
DEFINE FIELD IF NOT EXISTS updated_at ON TABLE refno_assoc_index TYPE datetime;
DEFINE FIELD IF NOT EXISTS version ON TABLE refno_assoc_index TYPE int;
DEFINE INDEX IF NOT EXISTS idx_refno_assoc_refno ON TABLE refno_assoc_index FIELDS refno UNIQUE;
"#;
            let _ = model_primary_db().query(sql).await;
        })
        .await;
}

#[derive(Debug, Default, Clone)]
pub struct RefnoAssocIndexEntry {
    pub refno: RefnoEnum,
    pub inst_relate_ids: BTreeSet<String>,
    pub inst_info_ids: BTreeSet<String>,
    pub geo_relate_ids: BTreeSet<String>,
    pub neg_relate_ids: BTreeSet<String>,
    pub ngmr_relate_ids: BTreeSet<String>,
    pub inst_relate_bool_ids: BTreeSet<String>,
    pub inst_relate_cata_bool_ids: BTreeSet<String>,
    pub inst_relate_aabb_ids: BTreeSet<String>,
    pub inst_relate_booled_aabb_ids: BTreeSet<String>,
}

impl RefnoAssocIndexEntry {
    fn upsert_sql(&self) -> String {
        format!(
            "UPSERT {} CONTENT {{ \
                refno: {}, \
                inst_relate_ids: {}, \
                inst_info_ids: {}, \
                geo_relate_ids: {}, \
                neg_relate_ids: {}, \
                ngmr_relate_ids: {}, \
                inst_relate_bool_ids: {}, \
                inst_relate_cata_bool_ids: {}, \
                inst_relate_aabb_ids: {}, \
                inst_relate_booled_aabb_ids: {}, \
                updated_at: time::now(), \
                version: 1 \
            }};",
            assoc_record_id(self.refno),
            self.refno.to_pe_key(),
            array_to_sql_string(&self.inst_relate_ids),
            array_to_sql_string(&self.inst_info_ids),
            array_to_sql_string(&self.geo_relate_ids),
            array_to_sql_string(&self.neg_relate_ids),
            array_to_sql_string(&self.ngmr_relate_ids),
            array_to_sql_string(&self.inst_relate_bool_ids),
            array_to_sql_string(&self.inst_relate_cata_bool_ids),
            array_to_sql_string(&self.inst_relate_aabb_ids),
            array_to_sql_string(&self.inst_relate_booled_aabb_ids),
        )
    }
}

#[derive(Debug, Default, Clone)]
pub struct RefnoAssocIndexBatch {
    entries: std::collections::HashMap<RefnoEnum, RefnoAssocIndexEntry>,
}

impl RefnoAssocIndexBatch {
    fn entry_mut(&mut self, refno: RefnoEnum) -> &mut RefnoAssocIndexEntry {
        self.entries.entry(refno).or_insert_with(|| RefnoAssocIndexEntry {
            refno,
            ..Default::default()
        })
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub fn add_inst_relate_id(&mut self, refno: RefnoEnum, id: String) {
        self.entry_mut(refno).inst_relate_ids.insert(id);
    }

    pub fn add_inst_info_id(&mut self, refno: RefnoEnum, id: String) {
        self.entry_mut(refno).inst_info_ids.insert(id);
    }

    pub fn add_geo_relate_id(&mut self, refno: RefnoEnum, id: String) {
        self.entry_mut(refno).geo_relate_ids.insert(id);
    }

    pub fn add_neg_relate_id(&mut self, refno: RefnoEnum, id: String) {
        self.entry_mut(refno).neg_relate_ids.insert(id);
    }

    pub fn add_ngmr_relate_id(&mut self, refno: RefnoEnum, id: String) {
        self.entry_mut(refno).ngmr_relate_ids.insert(id);
    }

    pub fn add_inst_relate_bool_id(&mut self, refno: RefnoEnum, id: String) {
        self.entry_mut(refno).inst_relate_bool_ids.insert(id);
    }

    pub fn add_inst_relate_cata_bool_id(&mut self, refno: RefnoEnum, id: String) {
        self.entry_mut(refno).inst_relate_cata_bool_ids.insert(id);
    }

    pub fn add_inst_relate_aabb_id(&mut self, refno: RefnoEnum, id: String) {
        self.entry_mut(refno).inst_relate_aabb_ids.insert(id);
    }

    pub fn add_inst_relate_booled_aabb_id(&mut self, refno: RefnoEnum, id: String) {
        self.entry_mut(refno).inst_relate_booled_aabb_ids.insert(id);
    }

    fn to_upsert_sqls(&self) -> Vec<String> {
        let mut entries = self.entries.values().collect::<Vec<_>>();
        entries.sort_by_key(|e| e.refno.to_string());
        entries.into_iter().map(|e| e.upsert_sql()).collect()
    }

    pub async fn upsert_to_db(&self) -> anyhow::Result<()> {
        if self.entries.is_empty() {
            return Ok(());
        }
        ensure_refno_assoc_index_schema().await;

        for sql in self.to_upsert_sqls() {
            model_primary_db()
                .query_response(&sql)
                .await
                .with_context(|| format!("写入 refno_assoc_index 失败: {}", sql))?;
        }
        Ok(())
    }

    pub fn write_to_sql_file(&self, writer: &SqlFileWriter) -> anyhow::Result<()> {
        if self.entries.is_empty() {
            return Ok(());
        }
        for sql in self.to_upsert_sqls() {
            writer.write_statement(&sql)?;
        }
        Ok(())
    }
}

fn get_string_array(row: &Value, key: &str) -> Vec<String> {
    row.get(key)
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str().map(|s| s.to_string()))
                .collect::<Vec<_>>()
        })
        .unwrap_or_default()
}

fn build_delete_sql_from_rows(
    rows: &[Value],
    index_record_ids: &[String],
    chunk_size: usize,
) -> Vec<String> {
    let mut inst_relate_ids: HashSet<String> = HashSet::new();
    let mut geo_relate_ids: HashSet<String> = HashSet::new();
    let mut neg_relate_ids: HashSet<String> = HashSet::new();
    let mut ngmr_relate_ids: HashSet<String> = HashSet::new();
    let mut inst_relate_bool_ids: HashSet<String> = HashSet::new();
    let mut inst_relate_cata_bool_ids: HashSet<String> = HashSet::new();
    let mut inst_relate_aabb_ids: HashSet<String> = HashSet::new();

    for row in rows {
        for id in get_string_array(row, "inst_relate_ids") {
            inst_relate_ids.insert(id);
        }
        for id in get_string_array(row, "geo_relate_ids") {
            geo_relate_ids.insert(id);
        }
        for id in get_string_array(row, "neg_relate_ids") {
            neg_relate_ids.insert(id);
        }
        for id in get_string_array(row, "ngmr_relate_ids") {
            ngmr_relate_ids.insert(id);
        }
        for id in get_string_array(row, "inst_relate_bool_ids") {
            inst_relate_bool_ids.insert(id);
        }
        for id in get_string_array(row, "inst_relate_cata_bool_ids") {
            inst_relate_cata_bool_ids.insert(id);
        }
        for id in get_string_array(row, "inst_relate_aabb_ids") {
            inst_relate_aabb_ids.insert(id);
        }
    }

    fn append_delete_sql(records: &HashSet<String>, chunk_size: usize, sqls: &mut Vec<String>) {
        if records.is_empty() {
            return;
        }
        let mut ids = records.iter().cloned().collect::<Vec<_>>();
        ids.sort_unstable();
        for chunk in ids.chunks(chunk_size.max(1)) {
            sqls.push(format!("DELETE [{}];", chunk.join(",")));
        }
    }

    let mut sqls = Vec::new();
    append_delete_sql(&inst_relate_ids, chunk_size, &mut sqls);
    append_delete_sql(&geo_relate_ids, chunk_size, &mut sqls);
    append_delete_sql(&neg_relate_ids, chunk_size, &mut sqls);
    append_delete_sql(&ngmr_relate_ids, chunk_size, &mut sqls);
    append_delete_sql(&inst_relate_bool_ids, chunk_size, &mut sqls);
    append_delete_sql(&inst_relate_cata_bool_ids, chunk_size, &mut sqls);
    append_delete_sql(&inst_relate_aabb_ids, chunk_size, &mut sqls);

    for chunk in index_record_ids.chunks(chunk_size.max(1)) {
        sqls.push(format!("DELETE [{}];", chunk.join(",")));
    }

    sqls
}

async fn load_assoc_rows(
    refnos: &[RefnoEnum],
    chunk_size: usize,
) -> anyhow::Result<(Vec<Value>, Vec<String>)> {
    ensure_refno_assoc_index_schema().await;

    let mut rows: Vec<Value> = Vec::new();
    let mut index_ids: Vec<String> = Vec::new();
    for chunk in refnos.chunks(chunk_size.max(1)) {
        let ids = chunk
            .iter()
            .map(|r| assoc_record_id(*r))
            .collect::<Vec<_>>();
        let sql = format!("SELECT * FROM [{}];", ids.join(","));
        let mut resp = model_primary_db().query_response(&sql).await?;
        let mut part: Vec<Value> = resp.take(0).unwrap_or_default();
        rows.append(&mut part);
        index_ids.extend(ids);
    }
    Ok((rows, index_ids))
}

pub async fn build_delete_sql_by_refnos(
    refnos: &[RefnoEnum],
    chunk_size: usize,
) -> anyhow::Result<Option<Vec<String>>> {
    if refnos.is_empty() {
        return Ok(Some(Vec::new()));
    }

    let (rows, index_ids) = load_assoc_rows(refnos, chunk_size).await?;
    if rows.len() != refnos.len() {
        return Ok(None);
    }
    Ok(Some(build_delete_sql_from_rows(&rows, &index_ids, chunk_size)))
}

#[derive(Debug, Default, Clone)]
pub struct RefnoAssocDeleteSummary {
    pub used_index: bool,
    pub deleted_statement_count: usize,
    pub indexed_refnos: usize,
    pub requested_refnos: usize,
}

pub async fn delete_by_refnos(
    refnos: &[RefnoEnum],
    chunk_size: usize,
) -> anyhow::Result<RefnoAssocDeleteSummary> {
    if refnos.is_empty() {
        return Ok(RefnoAssocDeleteSummary::default());
    }

    let (rows, index_ids) = load_assoc_rows(refnos, chunk_size).await?;
    if rows.len() != refnos.len() {
        return Ok(RefnoAssocDeleteSummary {
            used_index: false,
            deleted_statement_count: 0,
            indexed_refnos: rows.len(),
            requested_refnos: refnos.len(),
        });
    }

    let sqls = build_delete_sql_from_rows(&rows, &index_ids, chunk_size);
    for sql in &sqls {
        model_primary_db().query_response(sql).await?;
    }

    Ok(RefnoAssocDeleteSummary {
        used_index: true,
        deleted_statement_count: sqls.len(),
        indexed_refnos: rows.len(),
        requested_refnos: refnos.len(),
    })
}

