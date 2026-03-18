use std::collections::{BTreeMap, BTreeSet, HashSet};

use aios_core::{RefnoEnum, SurrealQueryExt, model_primary_db};
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

fn split_refno_parts(refno: RefnoEnum) -> Option<(u32, u32)> {
    let raw = refno.to_string();
    let (ref0, ref1) = raw.split_once('/').or_else(|| raw.split_once('_'))?;
    let ref0 = ref0.parse::<u32>().ok()?;
    let ref1 = ref1.parse::<u32>().ok()?;
    Some((ref0, ref1))
}

fn assoc_record_id(refno: RefnoEnum) -> String {
    let (ref0, ref1) = split_refno_parts(refno).unwrap_or((0, 0));
    format!("refno_assoc_index:[{ref0},{ref1}]")
}

fn assoc_record_id_from_parts(ref0: u32, ref1: u32) -> String {
    format!("refno_assoc_index:[{ref0},{ref1}]")
}

fn assoc_record_id_from_refno_key(refno_key: &str) -> Option<String> {
    let (ref0, ref1) = refno_key
        .split_once('/')
        .or_else(|| refno_key.split_once('_'))?;
    let ref0 = ref0.parse::<u32>().ok()?;
    let ref1 = ref1.parse::<u32>().ok()?;
    Some(assoc_record_id_from_parts(ref0, ref1))
}

fn refno_key(refno: RefnoEnum) -> String {
    refno.to_string()
}

fn assoc_refno_key_from_value(value: &Value) -> Option<String> {
    let id_value = value.get("id")?;
    match id_value {
        Value::String(raw) => {
            let prefix = "refno_assoc_index:[";
            if let Some(inner) = raw.strip_prefix(prefix).and_then(|s| s.strip_suffix(']')) {
                let (ref0, ref1) = inner.split_once(',')?;
                return Some(format!("{}/{}", ref0.trim(), ref1.trim()));
            }
            None
        }
        Value::Array(arr) if arr.len() == 2 => {
            let ref0 = arr.first()?.as_u64()?;
            let ref1 = arr.get(1)?.as_u64()?;
            Some(format!("{ref0}/{ref1}"))
        }
        _ => value.get("refno").and_then(|v| v.as_str()).and_then(|s| {
            let raw = s
                .strip_prefix("pe:⟨")
                .and_then(|v| v.strip_suffix('⟩'))
                .or_else(|| {
                    s.strip_prefix("pe:")
                        .map(|v| v.trim_matches('⟨').trim_matches('⟩'))
                })?;
            let (ref0, ref1) = raw.split_once('_')?;
            Some(format!("{ref0}/{ref1}"))
        }),
    }
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
DEFINE FIELD IF NOT EXISTS geo_hashes ON TABLE refno_assoc_index TYPE array<string>;
DEFINE FIELD IF NOT EXISTS neg_relate_ids ON TABLE refno_assoc_index TYPE array<string>;
DEFINE FIELD IF NOT EXISTS ngmr_relate_ids ON TABLE refno_assoc_index TYPE array<string>;
DEFINE FIELD IF NOT EXISTS inst_relate_bool_ids ON TABLE refno_assoc_index TYPE array<string>;
DEFINE FIELD IF NOT EXISTS inst_relate_cata_bool_ids ON TABLE refno_assoc_index TYPE array<string>;
DEFINE FIELD IF NOT EXISTS inst_relate_aabb_ids ON TABLE refno_assoc_index TYPE array<string>;
DEFINE FIELD IF NOT EXISTS inst_relate_booled_aabb_ids ON TABLE refno_assoc_index TYPE array<string>;
DEFINE FIELD IF NOT EXISTS tubi_branch_keys ON TABLE refno_assoc_index TYPE array<string>;
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
    pub geo_hashes: BTreeSet<String>,
    pub neg_relate_ids: BTreeSet<String>,
    pub ngmr_relate_ids: BTreeSet<String>,
    pub inst_relate_bool_ids: BTreeSet<String>,
    pub inst_relate_cata_bool_ids: BTreeSet<String>,
    pub inst_relate_aabb_ids: BTreeSet<String>,
    pub inst_relate_booled_aabb_ids: BTreeSet<String>,
    pub tubi_branch_keys: BTreeSet<String>,
}

impl RefnoAssocIndexEntry {
    fn upsert_sql(&self) -> String {
        format!(
            "UPSERT {} CONTENT {{ \
                refno: {}, \
                inst_relate_ids: {}, \
                inst_info_ids: {}, \
                geo_relate_ids: {}, \
                geo_hashes: {}, \
                neg_relate_ids: {}, \
                ngmr_relate_ids: {}, \
                inst_relate_bool_ids: {}, \
                inst_relate_cata_bool_ids: {}, \
                inst_relate_aabb_ids: {}, \
                inst_relate_booled_aabb_ids: {}, \
                tubi_branch_keys: {}, \
                updated_at: time::now(), \
                version: 1 \
            }};",
            assoc_record_id(self.refno),
            self.refno.to_pe_key(),
            array_to_sql_string(&self.inst_relate_ids),
            array_to_sql_string(&self.inst_info_ids),
            array_to_sql_string(&self.geo_relate_ids),
            array_to_sql_string(&self.geo_hashes),
            array_to_sql_string(&self.neg_relate_ids),
            array_to_sql_string(&self.ngmr_relate_ids),
            array_to_sql_string(&self.inst_relate_bool_ids),
            array_to_sql_string(&self.inst_relate_cata_bool_ids),
            array_to_sql_string(&self.inst_relate_aabb_ids),
            array_to_sql_string(&self.inst_relate_booled_aabb_ids),
            array_to_sql_string(&self.tubi_branch_keys),
        )
    }
}

#[derive(Debug, Default, Clone)]
pub struct RefnoAssocIndexBatch {
    entries: std::collections::HashMap<RefnoEnum, RefnoAssocIndexEntry>,
}

impl RefnoAssocIndexBatch {
    fn entry_mut(&mut self, refno: RefnoEnum) -> &mut RefnoAssocIndexEntry {
        self.entries
            .entry(refno)
            .or_insert_with(|| RefnoAssocIndexEntry {
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

    pub fn add_geo_hash(&mut self, refno: RefnoEnum, hash: String) {
        self.entry_mut(refno).geo_hashes.insert(hash);
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

    pub fn add_tubi_branch_key(&mut self, refno: RefnoEnum, branch_key: String) {
        self.entry_mut(refno).tubi_branch_keys.insert(branch_key);
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

#[derive(Debug, Default, Clone)]
struct DeleteSqlStats {
    inst_relate_ids: usize,
    inst_info_ids: usize,
    geo_relate_ids: usize,
    geo_hashes: usize,
    neg_relate_ids: usize,
    ngmr_relate_ids: usize,
    inst_relate_bool_ids: usize,
    inst_relate_cata_bool_ids: usize,
    inst_relate_aabb_ids: usize,
    tubi_branch_keys: usize,
}

fn build_delete_sql_from_rows(
    rows: &[Value],
    index_record_ids: &[String],
    chunk_size: usize,
) -> (Vec<String>, DeleteSqlStats) {
    let mut inst_relate_ids: HashSet<String> = HashSet::new();
    let mut inst_info_ids: HashSet<String> = HashSet::new();
    let mut geo_relate_ids: HashSet<String> = HashSet::new();
    let mut geo_hashes: HashSet<String> = HashSet::new();
    let mut neg_relate_ids: HashSet<String> = HashSet::new();
    let mut ngmr_relate_ids: HashSet<String> = HashSet::new();
    let mut inst_relate_bool_ids: HashSet<String> = HashSet::new();
    let mut inst_relate_cata_bool_ids: HashSet<String> = HashSet::new();
    let mut inst_relate_aabb_ids: HashSet<String> = HashSet::new();
    let mut tubi_branch_keys: HashSet<String> = HashSet::new();

    for row in rows {
        for id in get_string_array(row, "inst_relate_ids") {
            inst_relate_ids.insert(id);
        }
        for id in get_string_array(row, "inst_info_ids") {
            inst_info_ids.insert(id);
        }
        for id in get_string_array(row, "geo_relate_ids") {
            geo_relate_ids.insert(id);
        }
        for id in get_string_array(row, "geo_hashes") {
            geo_hashes.insert(id);
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
        for id in get_string_array(row, "tubi_branch_keys") {
            tubi_branch_keys.insert(id);
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

    fn append_inst_geo_delete_sql(
        hashes: &HashSet<String>,
        chunk_size: usize,
        sqls: &mut Vec<String>,
    ) {
        let mut ids = hashes
            .iter()
            .filter_map(|hash| hash.parse::<u64>().ok())
            .filter(|hash| *hash >= 10)
            .map(|hash| format!("inst_geo:{hash}"))
            .collect::<Vec<_>>();
        if ids.is_empty() {
            return;
        }
        ids.sort_unstable();
        for chunk in ids.chunks(chunk_size.max(1)) {
            sqls.push(format!("DELETE [{}];", chunk.join(",")));
        }
    }

    fn append_tubi_delete_sql(
        branch_keys: &HashSet<String>,
        chunk_size: usize,
        sqls: &mut Vec<String>,
    ) {
        if branch_keys.is_empty() {
            return;
        }
        let mut keys = branch_keys.iter().cloned().collect::<Vec<_>>();
        keys.sort_unstable();
        for chunk in keys.chunks(chunk_size.max(1)) {
            let statements = chunk
                .iter()
                .map(|branch_key| {
                    format!(
                        "LET $ids = SELECT VALUE id FROM tubi_relate:[{branch_key}, 0]..[{branch_key}, ..]; DELETE $ids;"
                    )
                })
                .collect::<Vec<_>>()
                .join("\n");
            sqls.push(statements);
        }
    }

    let stats = DeleteSqlStats {
        inst_relate_ids: inst_relate_ids.len(),
        inst_info_ids: inst_info_ids.len(),
        geo_relate_ids: geo_relate_ids.len(),
        geo_hashes: geo_hashes.len(),
        neg_relate_ids: neg_relate_ids.len(),
        ngmr_relate_ids: ngmr_relate_ids.len(),
        inst_relate_bool_ids: inst_relate_bool_ids.len(),
        inst_relate_cata_bool_ids: inst_relate_cata_bool_ids.len(),
        inst_relate_aabb_ids: inst_relate_aabb_ids.len(),
        tubi_branch_keys: tubi_branch_keys.len(),
    };

    let mut sqls = Vec::new();
    append_delete_sql(&neg_relate_ids, chunk_size, &mut sqls);
    append_delete_sql(&ngmr_relate_ids, chunk_size, &mut sqls);
    append_delete_sql(&inst_relate_bool_ids, chunk_size, &mut sqls);
    append_delete_sql(&inst_relate_cata_bool_ids, chunk_size, &mut sqls);
    append_delete_sql(&inst_relate_aabb_ids, chunk_size, &mut sqls);
    append_delete_sql(&inst_relate_ids, chunk_size, &mut sqls);
    append_delete_sql(&inst_info_ids, chunk_size, &mut sqls);
    append_delete_sql(&geo_relate_ids, chunk_size, &mut sqls);
    append_inst_geo_delete_sql(&geo_hashes, chunk_size, &mut sqls);
    append_tubi_delete_sql(&tubi_branch_keys, chunk_size, &mut sqls);

    for chunk in index_record_ids.chunks(chunk_size.max(1)) {
        sqls.push(format!("DELETE [{}];", chunk.join(",")));
    }

    (sqls, stats)
}

#[derive(Debug, Default, Clone)]
struct LoadedAssocRows {
    rows: Vec<Value>,
    index_record_ids: Vec<String>,
    exact_hits: usize,
    prefetched_ref0_groups: usize,
    overfetched_rows: usize,
}

async fn load_assoc_rows(
    refnos: &[RefnoEnum],
    chunk_size: usize,
) -> anyhow::Result<LoadedAssocRows> {
    ensure_refno_assoc_index_schema().await;

    let mut rows: Vec<Value> = Vec::new();
    let mut index_ids: Vec<String> = Vec::new();
    let mut loaded_refno_keys: HashSet<String> = HashSet::new();
    let mut refnos_by_ref0: BTreeMap<u32, Vec<RefnoEnum>> = BTreeMap::new();
    for &refno in refnos {
        if let Some((ref0, _)) = split_refno_parts(refno) {
            refnos_by_ref0.entry(ref0).or_default().push(refno);
        }
    }

    let mut prefetched_ref0_groups = 0usize;
    let mut overfetched_rows = 0usize;
    const PREFETCH_MIN_GROUP_SIZE: usize = 8;

    for (&ref0, group) in &refnos_by_ref0 {
        if group.len() < PREFETCH_MIN_GROUP_SIZE {
            continue;
        }
        let sql = format!("SELECT * FROM refno_assoc_index:[{ref0},0]..[{ref0},..];");
        let mut resp = model_primary_db().query_response(&sql).await?;
        let prefetched: Vec<Value> = resp.take(0).unwrap_or_default();
        let prefetched_total = prefetched.len();
        let target_keys = group.iter().map(|r| refno_key(*r)).collect::<HashSet<_>>();
        let mut matched = 0usize;
        for row in prefetched {
            let Some(row_refno_key) = assoc_refno_key_from_value(&row) else {
                continue;
            };
            if !target_keys.contains(&row_refno_key)
                || !loaded_refno_keys.insert(row_refno_key.clone())
            {
                continue;
            }
            if let Some(record_id) = assoc_record_id_from_refno_key(&row_refno_key) {
                index_ids.push(record_id);
            }
            rows.push(row);
            matched += 1;
        }
        prefetched_ref0_groups += 1;
        overfetched_rows += prefetched_total.saturating_sub(matched);
    }

    let remaining = refnos
        .iter()
        .copied()
        .filter(|refno| !loaded_refno_keys.contains(&refno_key(*refno)))
        .collect::<Vec<_>>();
    for chunk in remaining.chunks(chunk_size.max(1)) {
        let ids = chunk
            .iter()
            .map(|r| assoc_record_id(*r))
            .collect::<Vec<_>>();
        let sql = format!("SELECT * FROM [{}];", ids.join(","));
        let mut resp = model_primary_db().query_response(&sql).await?;
        let part: Vec<Value> = resp.take(0).unwrap_or_default();
        for row in part {
            let Some(row_refno_key) = assoc_refno_key_from_value(&row) else {
                continue;
            };
            if !loaded_refno_keys.insert(row_refno_key.clone()) {
                continue;
            }
            if let Some(record_id) = assoc_record_id_from_refno_key(&row_refno_key) {
                index_ids.push(record_id);
            }
            rows.push(row);
        }
    }

    Ok(LoadedAssocRows {
        rows,
        index_record_ids: index_ids,
        exact_hits: loaded_refno_keys.len(),
        prefetched_ref0_groups,
        overfetched_rows,
    })
}

pub async fn build_delete_sql_by_refnos(
    refnos: &[RefnoEnum],
    chunk_size: usize,
) -> anyhow::Result<Option<Vec<String>>> {
    if refnos.is_empty() {
        return Ok(Some(Vec::new()));
    }

    let loaded = load_assoc_rows(refnos, chunk_size).await?;
    if loaded.rows.len() != refnos.len() {
        return Ok(None);
    }
    Ok(Some(
        build_delete_sql_from_rows(&loaded.rows, &loaded.index_record_ids, chunk_size).0,
    ))
}

#[derive(Debug, Default, Clone)]
pub struct RefnoAssocDeleteSummary {
    pub used_index: bool,
    pub deleted_statement_count: usize,
    pub indexed_refnos: usize,
    pub requested_refnos: usize,
    pub cache_miss_refnos: usize,
    pub prefetched_ref0_groups: usize,
    pub overfetched_rows: usize,
    pub inst_relate_ids: usize,
    pub inst_info_ids: usize,
    pub geo_relate_ids: usize,
    pub geo_hashes: usize,
    pub neg_relate_ids: usize,
    pub ngmr_relate_ids: usize,
    pub inst_relate_bool_ids: usize,
    pub inst_relate_cata_bool_ids: usize,
    pub inst_relate_aabb_ids: usize,
    pub tubi_branch_keys: usize,
}

pub async fn delete_by_refnos(
    refnos: &[RefnoEnum],
    chunk_size: usize,
) -> anyhow::Result<RefnoAssocDeleteSummary> {
    if refnos.is_empty() {
        return Ok(RefnoAssocDeleteSummary::default());
    }

    let loaded = load_assoc_rows(refnos, chunk_size).await?;
    if loaded.rows.len() != refnos.len() {
        return Ok(RefnoAssocDeleteSummary {
            used_index: false,
            deleted_statement_count: 0,
            indexed_refnos: loaded.rows.len(),
            requested_refnos: refnos.len(),
            cache_miss_refnos: refnos.len().saturating_sub(loaded.rows.len()),
            prefetched_ref0_groups: loaded.prefetched_ref0_groups,
            overfetched_rows: loaded.overfetched_rows,
            inst_relate_ids: 0,
            inst_info_ids: 0,
            geo_relate_ids: 0,
            geo_hashes: 0,
            neg_relate_ids: 0,
            ngmr_relate_ids: 0,
            inst_relate_bool_ids: 0,
            inst_relate_cata_bool_ids: 0,
            inst_relate_aabb_ids: 0,
            tubi_branch_keys: 0,
        });
    }

    let (sqls, stats) =
        build_delete_sql_from_rows(&loaded.rows, &loaded.index_record_ids, chunk_size);
    for sql in &sqls {
        model_primary_db().query_response(sql).await?;
    }

    Ok(RefnoAssocDeleteSummary {
        used_index: true,
        deleted_statement_count: sqls.len(),
        indexed_refnos: loaded.exact_hits,
        requested_refnos: refnos.len(),
        cache_miss_refnos: 0,
        prefetched_ref0_groups: loaded.prefetched_ref0_groups,
        overfetched_rows: loaded.overfetched_rows,
        inst_relate_ids: stats.inst_relate_ids,
        inst_info_ids: stats.inst_info_ids,
        geo_relate_ids: stats.geo_relate_ids,
        geo_hashes: stats.geo_hashes,
        neg_relate_ids: stats.neg_relate_ids,
        ngmr_relate_ids: stats.ngmr_relate_ids,
        inst_relate_bool_ids: stats.inst_relate_bool_ids,
        inst_relate_cata_bool_ids: stats.inst_relate_cata_bool_ids,
        inst_relate_aabb_ids: stats.inst_relate_aabb_ids,
        tubi_branch_keys: stats.tubi_branch_keys,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::str::FromStr;

    #[test]
    fn assoc_record_id_should_use_array_id_shape() {
        let refno = RefnoEnum::from_str("24381/145569").unwrap();
        assert_eq!(assoc_record_id(refno), "refno_assoc_index:[24381,145569]");
    }

    #[test]
    fn build_delete_sql_from_rows_should_cover_inst_info_inst_geo_and_tubi_range() {
        let rows = vec![json!({
            "inst_relate_ids": ["inst_relate:⟨24381/145569⟩"],
            "inst_info_ids": ["inst_info:⟨inst-key-1⟩"],
            "geo_relate_ids": ["geo_relate:⟨geo-rel-1⟩"],
            "geo_hashes": ["124", "9"],
            "tubi_branch_keys": ["pe:⟨24381_145569⟩"],
        })];
        let (sqls, _) =
            build_delete_sql_from_rows(&rows, &["refno_assoc_index:[24381,145569]".into()], 100);
        let joined = sqls.join("\n");

        assert!(joined.contains("DELETE [inst_info:⟨inst-key-1⟩];"));
        assert!(joined.contains("DELETE [inst_geo:124];"));
        assert!(!joined.contains("inst_geo:9"));
        assert!(joined.contains("tubi_relate:[pe:⟨24381_145569⟩, 0]..[pe:⟨24381_145569⟩, ..]"));
    }
}
