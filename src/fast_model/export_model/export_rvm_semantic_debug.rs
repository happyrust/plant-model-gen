use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet, VecDeque};
use std::fs;
use std::path::Path;

use aios_core::pdms_types::{RefU64, RefnoEnum};
use anyhow::{Context, Result, anyhow};
use chrono::{SecondsFormat, Utc};
use serde::Serialize;

use crate::model_relation_store::{InstGeoRecord, ModelRelationStore};

#[derive(Debug)]
pub struct SemanticDebugExportStats {
    pub component_count: usize,
    pub geometry_reference_count: usize,
    pub output_filename: String,
}

#[derive(Debug, Serialize)]
struct SemanticDebugArtifact {
    version: u32,
    format: &'static str,
    generated_at: String,
    dbnum: u32,
    scope_refno: String,
    source: &'static str,
    normalization_rules: SemanticNormalizationRules,
    summary: SemanticDebugSummary,
    components: Vec<SemanticDebugComponent>,
}

#[derive(Debug, Serialize)]
struct SemanticNormalizationRules {
    component_identity: &'static str,
    geometry_identity: &'static str,
    naming: &'static str,
    owner_linkage: &'static str,
}

#[derive(Debug, Serialize)]
struct SemanticDebugSummary {
    component_count: usize,
    duplicate_component_ids: usize,
    component_ids_stable: bool,
    geometry_reference_count: usize,
    unique_geometry_hash_count: usize,
}

#[derive(Debug, Serialize)]
struct SemanticDebugComponent {
    stable_id: String,
    refno: String,
    identity_source: &'static str,
    name: Option<String>,
    noun: Option<String>,
    owner_refno: Option<String>,
    owner_noun: Option<String>,
    owner_linkage_status: &'static str,
    geometry_refs: Vec<SemanticGeometryRef>,
}

#[derive(Debug, Serialize)]
struct SemanticGeometryRef {
    geo_hash: String,
    stable_key: String,
    group_path: Option<String>,
    geometry_index: Option<u64>,
    kind: Option<String>,
    geo_type: Option<String>,
    bbox_world: Option<serde_json::Value>,
}

#[derive(Debug, serde::Deserialize)]
struct GeometryBlob {
    group_path: Option<String>,
    geometry_index: Option<u64>,
    kind: Option<String>,
    geo_type: Option<String>,
    bbox_world: Option<serde_json::Value>,
}

pub fn export_rvm_semantic_debug(
    dbnum: u32,
    relation_store_root: &Path,
    output_dir: &Path,
    root_refno: RefnoEnum,
    verbose: bool,
) -> Result<SemanticDebugExportStats> {
    let relation_store = ModelRelationStore::new(relation_store_root);
    let component_rows = relation_store
        .read_component_rows(dbnum)
        .with_context(|| format!("读取 relations.db 失败 (dbnum={dbnum})"))?;
    let geo_relates = relation_store
        .read_geo_relates(dbnum)
        .with_context(|| format!("读取 geo_relate 失败 (dbnum={dbnum})"))?;
    let inst_geos = relation_store
        .read_inst_geos(dbnum)
        .with_context(|| format!("读取 inst_geo 失败 (dbnum={dbnum})"))?;

    let component_by_refno: HashMap<RefnoEnum, ComponentRow> = component_rows
        .into_iter()
        .map(|row| (row.refno, row))
        .collect();

    let root_ref = root_refno.refno();
    let scoped_refnos = collect_scoped_refnos(root_ref, &component_by_refno);
    if scoped_refnos.is_empty() {
        return Err(anyhow!(
            "未在关系库中找到 root_refno={} 的 RVM 语义记录",
            root_refno
        ));
    }

    let inst_geo_map = build_inst_geo_map(inst_geos);
    let geo_relate_map = build_geo_relate_map(geo_relates);
    let root_refno_string = RefnoEnum::Refno(root_ref).to_string();

    let mut components = Vec::new();
    let mut seen_component_ids = HashSet::new();
    let mut duplicate_component_ids = 0usize;
    let mut unique_geo_hashes = BTreeSet::new();
    let mut geometry_reference_count = 0usize;

    let mut scoped_refnos_sorted: Vec<RefU64> = scoped_refnos.into_iter().collect();
    scoped_refnos_sorted.sort();

    for refno in scoped_refnos_sorted {
        let Some(row) = component_by_refno.get(&RefnoEnum::Refno(refno)) else {
            continue;
        };

        let stable_id = RefnoEnum::Refno(refno).to_string();
        if !seen_component_ids.insert(stable_id.clone()) {
            duplicate_component_ids += 1;
        }

        let owner_refno = row
            .parent_refno
            .map(|owner| RefnoEnum::Refno(owner).to_string());
        let owner_noun = row
            .parent_refno
            .and_then(|owner| component_by_refno.get(&RefnoEnum::Refno(owner)))
            .and_then(|owner| owner.noun.clone());

        let owner_linkage_status = match (owner_refno.as_ref(), owner_noun.as_ref()) {
            (Some(_), Some(_)) => "consistent",
            (None, None) => "absent",
            _ => "partial",
        };

        let geometry_refs = geo_relate_map
            .get(&row.inst_id)
            .into_iter()
            .flatten()
            .filter_map(|geo_hash| {
                let blob = inst_geo_map.get(geo_hash)?;
                let stable_key = format!("{dbnum}:{geo_hash}");
                unique_geo_hashes.insert(*geo_hash);
                geometry_reference_count += 1;
                Some(SemanticGeometryRef {
                    geo_hash: geo_hash.to_string(),
                    stable_key,
                    group_path: blob.group_path.clone(),
                    geometry_index: blob.geometry_index,
                    kind: blob.kind.clone(),
                    geo_type: blob.geo_type.clone(),
                    bbox_world: blob.bbox_world.clone(),
                })
            })
            .collect();

        components.push(SemanticDebugComponent {
            stable_id,
            refno: RefnoEnum::Refno(refno).to_string(),
            identity_source: "stable_refno(dbnum, group_path, kind)",
            name: row.name.clone(),
            noun: row.noun.clone(),
            owner_refno,
            owner_noun,
            owner_linkage_status,
            geometry_refs,
        });
    }

    let artifact = SemanticDebugArtifact {
        version: 1,
        format: "rvm-semantic-debug",
        generated_at: Utc::now().to_rfc3339_opts(SecondsFormat::Millis, true),
        dbnum,
        scope_refno: root_refno_string.clone(),
        source: "relation_store_sqlite",
        normalization_rules: SemanticNormalizationRules {
            component_identity: "component.stable_id equals the deterministic refno generated by stable_refno(dbnum, group_path, kind) during RVM import",
            geometry_identity: "geometry_refs[*].stable_key equals dbnum + geo_hash where geo_hash comes from stable_geo_hash(dbnum, group_path, geometry_index, geometry)",
            naming: "name preserves imported ATT/RVM labels when non-empty; null means no resolved semantic name was available",
            owner_linkage: "owner_refno/owner_noun come from the normalized parent component within the same scoped relation store subtree; missing values are explicit",
        },
        summary: SemanticDebugSummary {
            component_count: components.len(),
            duplicate_component_ids,
            component_ids_stable: duplicate_component_ids == 0,
            geometry_reference_count,
            unique_geometry_hash_count: unique_geo_hashes.len(),
        },
        components,
    };

    fs::create_dir_all(output_dir)
        .with_context(|| format!("创建输出目录失败: {}", output_dir.display()))?;
    let slug = root_refno_string
        .replace(['/', '\\'], "_")
        .replace(' ', "_");
    let output_filename = format!("rvm_semantic_debug_root_{slug}.json");
    let output_path = output_dir.join(&output_filename);
    let payload =
        serde_json::to_string_pretty(&artifact).context("序列化 semantic debug JSON 失败")?;
    fs::write(&output_path, payload)
        .with_context(|| format!("写入 semantic debug 文件失败: {}", output_path.display()))?;

    if verbose {
        println!("✅ 写入 semantic debug: {}", output_path.display());
        println!(
            "   - scoped components: {}",
            artifact.summary.component_count
        );
        println!(
            "   - unique geo hashes: {}",
            artifact.summary.unique_geometry_hash_count
        );
    }

    Ok(SemanticDebugExportStats {
        component_count: artifact.summary.component_count,
        geometry_reference_count: artifact.summary.geometry_reference_count,
        output_filename,
    })
}

#[derive(Debug)]
struct ComponentRow {
    refno: RefnoEnum,
    inst_id: u64,
    parent_refno: Option<RefU64>,
    noun: Option<String>,
    name: Option<String>,
}

fn collect_scoped_refnos(
    root_refno: RefU64,
    components: &HashMap<RefnoEnum, ComponentRow>,
) -> BTreeSet<RefU64> {
    let mut children: BTreeMap<RefU64, Vec<RefU64>> = BTreeMap::new();
    let mut has_root = false;

    for row in components.values() {
        let current = row.refno.refno();
        if current == root_refno {
            has_root = true;
        }
        if let Some(parent) = row.parent_refno {
            children.entry(parent).or_default().push(current);
        }
    }

    if !has_root {
        return BTreeSet::new();
    }

    let mut scoped = BTreeSet::new();
    let mut queue = VecDeque::from([root_refno]);
    while let Some(current) = queue.pop_front() {
        if !scoped.insert(current) {
            continue;
        }
        if let Some(next) = children.get(&current) {
            for child in next {
                queue.push_back(*child);
            }
        }
    }
    scoped
}

fn build_geo_relate_map(geo_relates: Vec<(u64, u64)>) -> HashMap<u64, Vec<u64>> {
    let mut map: HashMap<u64, Vec<u64>> = HashMap::new();
    for (inst_id, geo_hash) in geo_relates {
        map.entry(inst_id).or_default().push(geo_hash);
    }
    for values in map.values_mut() {
        values.sort_unstable();
    }
    map
}

fn build_inst_geo_map(inst_geos: Vec<InstGeoRecord>) -> HashMap<u64, GeometryBlob> {
    let mut map = HashMap::new();
    for row in inst_geos {
        let parsed =
            serde_json::from_slice::<GeometryBlob>(&row.geometry).unwrap_or(GeometryBlob {
                group_path: None,
                geometry_index: None,
                kind: None,
                geo_type: None,
                bbox_world: None,
            });
        map.insert(row.hash, parsed);
    }
    map
}

trait RelationStoreReader {
    fn read_component_rows(&self, dbnum: u32) -> Result<Vec<ComponentRow>>;
    fn read_geo_relates(&self, dbnum: u32) -> Result<Vec<(u64, u64)>>;
    fn read_inst_geos(&self, dbnum: u32) -> Result<Vec<InstGeoRecord>>;
}

impl RelationStoreReader for ModelRelationStore {
    fn read_component_rows(&self, dbnum: u32) -> Result<Vec<ComponentRow>> {
        let db_path = relation_db_path(self, dbnum);
        let conn = rusqlite::Connection::open(&db_path)
            .with_context(|| format!("打开关系库失败: {}", db_path.display()))?;

        let mut stmt = conn.prepare(
            "SELECT refno, inst_id, parent_refno, noun, name FROM inst_relate ORDER BY refno",
        )?;
        let rows = stmt.query_map([], |row| {
            let refno_raw: String = row.get(0)?;
            let parent_raw: Option<String> = row.get(2)?;
            Ok(ComponentRow {
                refno: RefnoEnum::from(refno_raw.as_str()),
                inst_id: row.get(1)?,
                parent_refno: parent_raw.as_deref().and_then(parse_refu64),
                noun: row.get(3)?,
                name: row
                    .get::<_, Option<String>>(4)?
                    .and_then(normalize_optional_string),
            })
        })?;

        rows.collect::<rusqlite::Result<Vec<_>>>()
            .map_err(Into::into)
    }

    fn read_geo_relates(&self, dbnum: u32) -> Result<Vec<(u64, u64)>> {
        let db_path = relation_db_path(self, dbnum);
        let conn = rusqlite::Connection::open(&db_path)
            .with_context(|| format!("打开关系库失败: {}", db_path.display()))?;
        let mut stmt =
            conn.prepare("SELECT inst_id, geo_hash FROM geo_relate ORDER BY inst_id, geo_hash")?;
        let rows = stmt.query_map([], |row| Ok((row.get(0)?, row.get(1)?)))?;
        rows.collect::<rusqlite::Result<Vec<_>>>()
            .map_err(Into::into)
    }

    fn read_inst_geos(&self, dbnum: u32) -> Result<Vec<InstGeoRecord>> {
        let db_path = relation_db_path(self, dbnum);
        let conn = rusqlite::Connection::open(&db_path)
            .with_context(|| format!("打开关系库失败: {}", db_path.display()))?;
        let mut stmt = conn.prepare(
            "SELECT hash, geometry, aabb_min_x, aabb_min_y, aabb_min_z, aabb_max_x, aabb_max_y, aabb_max_z, meshed FROM inst_geo",
        )?;
        let rows = stmt.query_map([], |row| {
            Ok(InstGeoRecord {
                hash: row.get(0)?,
                geometry: row.get(1)?,
                aabb_min_x: row.get(2)?,
                aabb_min_y: row.get(3)?,
                aabb_min_z: row.get(4)?,
                aabb_max_x: row.get(5)?,
                aabb_max_y: row.get(6)?,
                aabb_max_z: row.get(7)?,
                meshed: row.get::<_, i64>(8)? != 0,
            })
        })?;
        rows.collect::<rusqlite::Result<Vec<_>>>()
            .map_err(Into::into)
    }
}

fn relation_db_path(store: &ModelRelationStore, dbnum: u32) -> std::path::PathBuf {
    let _ = store;
    store.db_dir(dbnum).join("relations.db")
}

fn parse_refu64(raw: &str) -> Option<RefU64> {
    let trimmed = raw.trim();
    let parts: Vec<&str> = trimmed.split('/').collect();
    if parts.len() != 2 {
        return None;
    }
    let high = parts[0].parse::<u32>().ok()?;
    let low = parts[1].parse::<u32>().ok()?;
    Some(RefU64::from(((high as u64) << 32) | low as u64))
}

fn normalize_optional_string(value: String) -> Option<String> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_string())
    }
}
