use crate::model_relation_store::{InstGeoRecord, InstRelateRecord, ModelRelationStore};
use aios_core::pdms_types::{RefU64, RefnoEnum};
use anyhow::{Context, Result, anyhow};
use bincode::serialize;
use glam::{Affine3A, Mat3A, Vec3, Vec3A};
use rvm_rs::store::Store;
use rvm_rs::store::geometry::{Geometry, GeometryKind, GeometryType};
use rvm_rs::store::node::{NodeId, NodeKind};
use rvm_rs::{parse_att, parse_rvm};
use serde_json::json;
use std::collections::{HashMap, HashSet, VecDeque};
use std::fs;
use std::hash::Hasher;
use std::path::{Path, PathBuf};
use surrealdb::types::SurrealValue;
use twox_hash::XxHash64;

#[derive(Debug, Clone)]
pub struct RvmImportOptions {
    pub dbnum: u32,
    pub relation_store_root: PathBuf,
    pub rvm_path: PathBuf,
    pub att_paths: Vec<PathBuf>,
    pub verbose: bool,
    /// spec 009:是否在导入期把 RVM 组名解析为真实 PDMS refno
    /// (需要可用的 SurrealDB 连接,失败自动退化为 stable_hash)。
    pub resolve_identity: bool,
}

#[derive(Debug, Default, Clone)]
pub struct RvmImportStats {
    pub file_nodes: usize,
    pub model_nodes: usize,
    pub group_nodes: usize,
    pub geometry_records: usize,
    pub cleaned_records: usize,
    pub att_sections: usize,
    pub att_attr_sections: usize,
    pub att_duplicate_sections: usize,
    pub att_matched_sections: usize,
    pub att_unmatched_sections: usize,
    /// spec 009:身份解析成功(真实 refno)的 inst_relate 数。
    pub resolved_records: usize,
    /// spec 009:仍使用 stable_hash 伪 refno 的 inst_relate 数。
    pub unresolved_records: usize,
}

pub async fn import_rvm_to_sqlite(options: &RvmImportOptions) -> Result<RvmImportStats> {
    let mut store = Store::new();
    let rvm_bytes = fs::read(&options.rvm_path)
        .with_context(|| format!("读取 RVM 文件失败: {}", options.rvm_path.display()))?;
    parse_rvm(&rvm_bytes, &mut store)
        .with_context(|| format!("解析 RVM 文件失败: {}", options.rvm_path.display()))?;

    for att_path in &options.att_paths {
        let att_text = fs::read_to_string(att_path)
            .with_context(|| format!("读取 ATT 文件失败: {}", att_path.display()))?;
        parse_att(&att_text, &mut store)
            .with_context(|| format!("解析 ATT 文件失败: {}", att_path.display()))?;
    }

    let att_index = load_att_attr_index(&options.att_paths)?;
    let root_name_hint = infer_root_name_from_att_index(&att_index);
    let mut builder = RelationBuilder::new(options.dbnum, options.verbose, att_index);
    builder.build(&store)?;
    if options.verbose {
        builder.print_att_summary();
    }
    let att_stats = builder.att_stats();

    let (resolved_records, unresolved_records) = if options.resolve_identity {
        let root_refno_hint = infer_root_refno_from_rvm_path(&options.rvm_path);
        resolve_identities(
            &mut builder,
            options.dbnum,
            options.verbose,
            root_refno_hint,
            root_name_hint.as_deref(),
        )
        .await?
    } else {
        (0, builder.inst_relates.len())
    };

    let relation_store = ModelRelationStore::new(&options.relation_store_root);
    let refnos: Vec<RefnoEnum> = builder.inst_relates.iter().map(|r| r.refno).collect();
    let names: Vec<String> = builder
        .inst_relates
        .iter()
        .filter_map(|r| r.name.clone())
        .collect();
    let cleaned_records_by_name = relation_store
        .cleanup_inst_relates_by_names(options.dbnum, &names)
        .unwrap_or(0);
    let cleaned_records_by_refno = relation_store
        .cleanup_by_refnos(options.dbnum, &refnos)
        .unwrap_or(0);
    let cleaned_records = cleaned_records_by_name + cleaned_records_by_refno;
    relation_store.insert_inst_geos(options.dbnum, &builder.inst_geos)?;
    relation_store.insert_inst_relates(options.dbnum, &builder.inst_relates)?;
    relation_store.insert_geo_relates(options.dbnum, &builder.geo_relates)?;

    Ok(RvmImportStats {
        file_nodes: builder.stats.file_nodes,
        model_nodes: builder.stats.model_nodes,
        group_nodes: builder.stats.group_nodes,
        geometry_records: builder.stats.geometry_records,
        cleaned_records,
        att_sections: att_stats.sections,
        att_attr_sections: att_stats.attr_sections,
        att_duplicate_sections: att_stats.duplicate_sections,
        att_matched_sections: att_stats.matched_sections,
        att_unmatched_sections: att_stats.unmatched_sections,
        resolved_records,
        unresolved_records,
    })
}

// ──────────────────────── spec 009:导入期身份解析 ────────────────────────

/// E3D 默认命名解析结果:`<NOUN_FULL> <n> of <OWNER_DESC>`。
/// `owner_desc` 可能是 `BRANCH /x/B1`(命名 owner)或
/// `TMPLATE 1 of EQUIPMENT /x`(owner 自身也是默认命名,需递归解析)。
#[derive(Debug)]
struct DefaultNameParts<'a> {
    noun_full: &'a str,
    ordinal: usize,
    owner_desc: &'a str,
}

/// 解析 E3D 默认命名的最外层,如:
/// `FLANGE 1 of BRANCH /03SKID1-PIPE-SUCTION/B1`
/// `SNOUT 1 of TMPLATE 1 of EQUIPMENT /03SKID3-EQUIP1`(嵌套)。
fn parse_default_name(name: &str) -> Option<DefaultNameParts<'_>> {
    let (left, right) = name.split_once(" of ")?;
    let (noun_full, ordinal_str) = left.rsplit_once(' ')?;
    let ordinal: usize = ordinal_str.parse().ok()?;
    if ordinal == 0 {
        return None;
    }
    Some(DefaultNameParts {
        noun_full: noun_full.trim(),
        ordinal,
        owner_desc: right.trim(),
    })
}

/// 从 owner 描述中提取命名 owner:`<OWNER_NOUN_FULL> /name` → `/name`。
/// 仅当 `/` 前正好是单个名词(无序号)时成立;否则说明是嵌套默认命名。
fn named_owner_from_desc(desc: &str) -> Option<&str> {
    let slash = desc.find('/')?;
    let prefix = desc[..slash].trim();
    // 单层:前缀是一个纯名词(如 BRANCH / EQUIPMENT);嵌套:前缀含 " of " 或序号。
    if !prefix.is_empty() && !prefix.contains(' ') {
        Some(desc[slash..].trim())
    } else {
        None
    }
}

/// RVM 默认命名里的名词全称 → PDMS 四字短名词(站点库 `pe.noun` 形态)。
fn full_noun_to_short(full: &str) -> String {
    match full {
        "FLANGE" => "FLAN",
        "ELBOW" => "ELBO",
        "REDUCER" => "REDU",
        "GASKET" => "GASK",
        "VALVE" => "VALV",
        "BRANCH" => "BRAN",
        "TUBING" | "TUBE" => "TUBI",
        "EQUIPMENT" => "EQUI",
        "NOZZLE" => "NOZZ",
        "COUPLING" => "COUP",
        "INSTRUMENT" => "INST",
        "ATTACHMENT" => "ATTA",
        "STRUCTURE" => "STRU",
        "FITTING" => "FITT",
        other => {
            let upper = other.to_ascii_uppercase();
            let take = upper.chars().take(4).collect::<String>();
            return take;
        }
    }
    .to_string()
}

/// 把站点库 `record::id(pe)` 字符串 key(`ref0_ref1`)解析为 RefnoEnum。
fn refno_from_pe_key(key: &str) -> Option<RefnoEnum> {
    use std::str::FromStr;
    RefnoEnum::from_str(key)
        .ok()
        .or_else(|| RefnoEnum::from_str(&key.replace('_', "/")).ok())
}

fn refno_from_att_value(value: &str) -> Option<RefnoEnum> {
    let value = value.trim();
    if !value.starts_with('=') {
        return None;
    }
    let trimmed = value.trim_start_matches('=');
    let refno = refno_from_pe_key(trimmed)?;
    let r = refno.refno();
    if r.get_0() == 0 || r.get_1() == 0 {
        return None;
    }
    Some(refno)
}

fn direct_refno_from_attrs(
    attrs: Option<&serde_json::Map<String, serde_json::Value>>,
) -> Option<RefnoEnum> {
    let value = attrs?.get("NAME")?.as_str()?;
    refno_from_att_value(value)
}

fn noun_from_attrs(attrs: Option<&serde_json::Map<String, serde_json::Value>>) -> Option<String> {
    let noun = attrs?.get("TYPE")?.as_str()?.trim();
    (!noun.is_empty()).then(|| noun.to_ascii_uppercase())
}

fn infer_root_refno_from_rvm_path(path: &Path) -> Option<RefnoEnum> {
    let stem = path.file_stem()?.to_str()?;
    refno_from_pe_key(stem)
}

fn infer_root_name_from_att_index(att_index: &AttAttrIndex) -> Option<String> {
    let value = att_index
        .get("Header Information")?
        .get("Element")?
        .as_str()?;
    let name = value.trim();
    (!name.is_empty()).then(|| name.to_string())
}

#[derive(Debug, Default, Clone, Copy)]
struct AttImportStats {
    sections: usize,
    attr_sections: usize,
    duplicate_sections: usize,
    matched_sections: usize,
    unmatched_sections: usize,
}

#[derive(Debug, Default, Clone)]
struct AttAttrIndex {
    sections: HashMap<String, serde_json::Map<String, serde_json::Value>>,
    section_count: usize,
    duplicate_sections: usize,
    matched_sections: HashSet<String>,
}

impl AttAttrIndex {
    fn get(&self, name: &str) -> Option<&serde_json::Map<String, serde_json::Value>> {
        self.sections.get(name)
    }

    fn entry(&mut self, name: &str) -> &mut serde_json::Map<String, serde_json::Value> {
        self.sections.entry(name.to_string()).or_default()
    }

    fn start_section(&mut self, name: &str) {
        self.section_count += 1;
        if self.sections.contains_key(name) {
            self.duplicate_sections += 1;
        }
        self.sections.entry(name.to_string()).or_default();
    }

    fn attrs_for_group(&mut self, name: &str) -> serde_json::Map<String, serde_json::Value> {
        let Some(attrs) = self.sections.get(name) else {
            return serde_json::Map::new();
        };
        self.matched_sections.insert(name.to_string());
        attrs.clone()
    }

    fn retain_non_empty(&mut self) {
        self.sections.retain(|_, attrs| !attrs.is_empty());
    }

    fn stats(&self) -> AttImportStats {
        let matchable_names = self
            .sections
            .keys()
            .filter(|name| is_matchable_att_section(name));
        let matched_sections = self
            .matched_sections
            .iter()
            .filter(|name| is_matchable_att_section(name))
            .count();
        let unmatched_sections = matchable_names
            .filter(|name| !self.matched_sections.contains(*name))
            .count();
        AttImportStats {
            sections: self.section_count,
            attr_sections: self
                .sections
                .keys()
                .filter(|name| is_matchable_att_section(name))
                .count(),
            duplicate_sections: self.duplicate_sections,
            matched_sections,
            unmatched_sections,
        }
    }

    fn unmatched_samples(&self, limit: usize) -> Vec<String> {
        self.sections
            .keys()
            .filter(|name| is_matchable_att_section(name))
            .filter(|name| !self.matched_sections.contains(*name))
            .take(limit)
            .cloned()
            .collect()
    }
}

fn is_matchable_att_section(name: &str) -> bool {
    name != "Header Information"
}

fn previous_refno(refno: RefnoEnum) -> Option<RefnoEnum> {
    use std::str::FromStr;
    let r = refno.refno();
    let low = r.get_1().checked_sub(1)?;
    RefnoEnum::from_str(&format!("{}/{}", r.get_0(), low)).ok()
}

fn min_refno(values: impl IntoIterator<Item = RefnoEnum>) -> Option<RefnoEnum> {
    values.into_iter().min_by_key(|r| r.refno().0)
}

/// SurrealDB 字符串字面量转义(单引号)。
fn escape_surreal_str(s: &str) -> String {
    s.replace('\\', "\\\\").replace('\'', "\\'")
}

#[derive(Debug, serde::Deserialize, SurrealValue)]
struct PeIdentRow {
    #[serde(default)]
    key: Option<String>,
    #[serde(default)]
    noun: Option<String>,
}

/// 按元素全名精确查 pe。返回 (refno, noun);多解/无解返回 None。
async fn query_pe_by_name(dbnum: u32, name: &str) -> Result<Option<(RefnoEnum, Option<String>)>> {
    use aios_core::{SurrealQueryExt, project_primary_db};
    let sql = format!(
        "SELECT record::id(id) AS key, noun FROM pe WHERE dbnum = {dbnum} AND name = '{}' LIMIT 2;",
        escape_surreal_str(name)
    );
    let rows: Vec<PeIdentRow> = match project_primary_db().query_take(&sql, 0).await {
        Ok(rows) => rows,
        Err(e) => {
            eprintln!("[rvm-import] PE name lookup unavailable, fallback to ATT refs: {e}");
            return Ok(None);
        }
    };
    if rows.len() != 1 {
        return Ok(None);
    }
    let row = &rows[0];
    let Some(key) = row.key.as_deref() else {
        return Ok(None);
    };
    Ok(refno_from_pe_key(key).map(|refno| (refno, row.noun.clone())))
}

/// 查 owner 的有序子元素(图遍历保持成员序),返回 (key, noun) 列表。
async fn query_owner_children(owner: RefnoEnum) -> Result<Vec<PeIdentRow>> {
    use aios_core::{SurrealQueryExt, project_primary_db};
    let owner_key = owner.to_pe_key();
    let sql = format!(
        "SELECT VALUE (<-pe_owner.in).{{key: record::id(id), noun: noun}} FROM ONLY {owner_key} LIMIT 1;"
    );
    match project_primary_db().query_take(&sql, 0).await {
        Ok(rows) => Ok(rows),
        Err(e) => {
            eprintln!("[rvm-import] PE owner-child lookup unavailable, fallback to ATT refs: {e}");
            Ok(Vec::new())
        }
    }
}

/// 两段式身份解析:pass1 按遍历序(父先子)解析每个组的真实 refno,
/// pass2 重写 inst_relates(refno/inst_id/parent_refno)与 geo_relates(inst_id)。
async fn resolve_identities(
    builder: &mut RelationBuilder,
    dbnum: u32,
    verbose: bool,
    root_refno_hint: Option<RefnoEnum>,
    root_name_hint: Option<&str>,
) -> Result<(usize, usize)> {
    use std::collections::HashMap;

    // old(stable_hash) refno -> (真实 refno, noun, identity_source)
    let mut mapping: HashMap<RefnoEnum, (RefnoEnum, Option<String>, &'static str)> = HashMap::new();
    // 名称 -> 真实 refno(供默认命名 owner 查找;含本批已解析的命名元素)
    let mut by_name: HashMap<String, RefnoEnum> = HashMap::new();
    // owner refno -> 有序子元素缓存
    let mut children_cache: HashMap<RefnoEnum, Vec<PeIdentRow>> = HashMap::new();

    let records_snapshot: Vec<(
        RefnoEnum,
        Option<RefnoEnum>,
        Option<String>,
        Option<serde_json::Map<String, serde_json::Value>>,
    )> = builder
        .inst_relates
        .iter()
        .map(|r| {
            let attrs = r
                .attrs_json
                .as_deref()
                .and_then(|s| serde_json::from_str::<serde_json::Value>(s).ok())
                .and_then(|v| v.as_object().cloned());
            (r.refno, r.parent_refno, r.name.clone(), attrs)
        })
        .collect();

    let needs_surreal_lookup = records_snapshot.iter().any(|(_, _, name, attrs)| {
        let Some(name) = name.as_deref() else {
            return false;
        };
        direct_refno_from_attrs(attrs.as_ref()).is_none()
            && (name.starts_with('/') || parse_default_name(name).is_some())
    });
    let use_surreal_lookup = std::env::var("AIOS_RVM_IMPORT_USE_SURREAL_LOOKUP")
        .map(|v| matches!(v.as_str(), "1" | "true" | "TRUE" | "yes" | "YES"))
        .unwrap_or(false);
    let surreal_lookup_enabled = if needs_surreal_lookup && use_surreal_lookup {
        match crate::fast_model::utils::ensure_surreal_init().await {
            Ok(()) => true,
            Err(e) => {
                eprintln!(
                    "[rvm-import] SurrealDB 身份辅助查询不可用，将仅使用 ATT 直接 refno/序列回退: {e}"
                );
                false
            }
        }
    } else {
        if verbose && needs_surreal_lookup {
            println!(
                "[rvm-import] 跳过 SurrealDB 身份辅助查询；如需启用请设置 AIOS_RVM_IMPORT_USE_SURREAL_LOOKUP=1"
            );
        }
        false
    };

    for (old_refno, _old_parent, name, attrs) in &records_snapshot {
        let Some(name) = name.as_deref() else {
            continue;
        };

        let resolved: Option<(RefnoEnum, Option<String>, &'static str)> = if let Some(refno) =
            direct_refno_from_attrs(attrs.as_ref())
        {
            Some((refno, noun_from_attrs(attrs.as_ref()), "att_name_refno"))
        } else if name.starts_with('/') {
            if surreal_lookup_enabled {
                query_pe_by_name(dbnum, name)
                    .await?
                    .map(|(refno, noun)| (refno, noun, "surreal_name"))
            } else {
                None
            }
            .or_else(|| {
                if root_name_hint == Some(name) {
                    root_refno_hint
                        .map(|refno| (refno, noun_from_attrs(attrs.as_ref()), "rvm_filename_root"))
                } else {
                    None
                }
            })
        } else if let Some(parts) = parse_default_name(name) {
            // owner 查找:① 本批缓存按 owner 完整描述命中(覆盖嵌套默认命名,
            // 如 `TMPLATE 1 of EQUIPMENT /x`——遍历序父先子,owner 已解析过);
            // ② owner 是命名元素(`BRANCH /x`)→ 提取 `/x` 查库。
            let owner = match by_name.get(parts.owner_desc) {
                Some(r) => Some(*r),
                None => match named_owner_from_desc(parts.owner_desc) {
                    Some(owner_name) => match by_name.get(owner_name) {
                        Some(r) => Some(*r),
                        None if surreal_lookup_enabled => query_pe_by_name(dbnum, owner_name)
                            .await?
                            .map(|(r, _)| r)
                            .inspect(|r| {
                                by_name.insert(owner_name.to_string(), *r);
                            }),
                        None => None,
                    },
                    None => None,
                },
            };
            if let Some(owner) = owner {
                if !surreal_lookup_enabled {
                    None
                } else {
                    if !children_cache.contains_key(&owner) {
                        children_cache.insert(owner, query_owner_children(owner).await?);
                    }
                    // 站点库 noun 是 PDMS 截断名,长度不定:TMPL/SUBE(4)、
                    // GENSEC/JLDATU(6)、SPINE(5,全名)——按 {映射短名, 全名,
                    // 截6, 截4} 多候选匹配(实测 851/1036 子序列)。
                    let short = full_noun_to_short(parts.noun_full);
                    let full_upper = parts.noun_full.to_ascii_uppercase();
                    let take6: String = full_upper.chars().take(6).collect();
                    let take4: String = full_upper.chars().take(4).collect();
                    let children = &children_cache[&owner];
                    let matched_child = children
                        .iter()
                        .filter(|c| {
                            let Some(n) = c.noun.as_deref() else {
                                return false;
                            };
                            n == short || n == full_upper || n == take6 || n == take4
                        })
                        .nth(parts.ordinal - 1);
                    matched_child
                        .and_then(|c| {
                            let noun = c.noun.clone();
                            c.key
                                .as_deref()
                                .and_then(refno_from_pe_key)
                                .map(|refno| (refno, noun))
                        })
                        .map(|(refno, noun)| (refno, noun, "default_name_rule"))
                }
            } else {
                None
            }
        } else {
            None
        };

        match resolved {
            Some((new_refno, noun, src)) => {
                // 全名入缓存:命名元素与默认命名成员都可能是后续成员的 owner。
                by_name.insert(name.to_string(), new_refno);
                if verbose {
                    println!("[rvm-import] resolve {name} -> {new_refno} (source={src})");
                }
                mapping.insert(*old_refno, (new_refno, noun, src));
            }
            None => {
                if verbose {
                    println!("[rvm-import] resolve {name} -> <unresolved>");
                }
            }
        }
    }

    // If the PE table is unavailable, recover named container refnos from the ATT
    // tree: a named owner immediately precedes its child members in the same ref0
    // sequence, so it is one less than the minimum resolved child refno.
    let mut changed = true;
    while changed {
        changed = false;
        for (old_refno, _old_parent, name, attrs) in &records_snapshot {
            if mapping.contains_key(old_refno) {
                continue;
            }
            let Some(name) = name.as_deref() else {
                continue;
            };
            if !name.starts_with('/') {
                continue;
            }

            let direct_children = records_snapshot
                .iter()
                .filter(|(_, parent, _, _)| parent == &Some(*old_refno))
                .filter_map(|(child_old, _, _, _)| mapping.get(child_old).map(|(r, _, _)| *r));
            let Some(candidate) = min_refno(direct_children).and_then(previous_refno) else {
                continue;
            };

            if verbose {
                println!("[rvm-import] resolve {name} -> {candidate} (source=att_owner_sequence)");
            }
            by_name.insert(name.to_string(), candidate);
            mapping.insert(
                *old_refno,
                (
                    candidate,
                    noun_from_attrs(attrs.as_ref()),
                    "att_owner_sequence",
                ),
            );
            changed = true;
        }
    }

    // pass2:重写记录。
    let mut resolved_cnt = 0usize;
    for rec in &mut builder.inst_relates {
        if let Some((new_refno, noun, src)) = mapping.get(&rec.refno) {
            let new_inst_id = new_refno.refno().0;
            // geo_relates 的 inst_id 同步重映射。
            let old_inst_id = rec.inst_id;
            for (inst_id, _) in builder.geo_relates.iter_mut() {
                if *inst_id == old_inst_id {
                    *inst_id = new_inst_id;
                }
            }
            rec.refno = *new_refno;
            rec.inst_id = new_inst_id;
            rec.noun = noun.clone();
            rec.identity_source = Some((*src).to_string());
            rec.resolved = true;
            resolved_cnt += 1;
        }
        if let Some(parent) = rec.parent_refno {
            if let Some((new_parent, _, _)) = mapping.get(&parent) {
                rec.parent_refno = Some(*new_parent);
            }
        }
    }

    let unresolved_cnt = builder.inst_relates.len() - resolved_cnt;
    println!("[rvm-import] 身份解析完成: resolved={resolved_cnt} unresolved={unresolved_cnt}");
    Ok((resolved_cnt, unresolved_cnt))
}

struct RelationBuilder {
    dbnum: u32,
    verbose: bool,
    inst_relates: Vec<InstRelateRecord>,
    inst_geos: Vec<InstGeoRecord>,
    geo_relates: Vec<(u64, u64)>,
    att_index: AttAttrIndex,
    stats: RvmImportStats,
}

impl RelationBuilder {
    fn new(dbnum: u32, verbose: bool, att_index: AttAttrIndex) -> Self {
        Self {
            dbnum,
            verbose,
            inst_relates: Vec::new(),
            inst_geos: Vec::new(),
            geo_relates: Vec::new(),
            att_index,
            stats: RvmImportStats::default(),
        }
    }

    fn build(&mut self, store: &Store) -> Result<()> {
        for &root in store.roots() {
            self.walk_node(store, root, &mut VecDeque::new(), None, Vec3::ZERO)?;
        }
        Ok(())
    }

    fn att_stats(&self) -> AttImportStats {
        self.att_index.stats()
    }

    fn print_att_summary(&self) {
        let stats = self.att_stats();
        if stats.sections == 0 {
            println!("[rvm-import] ATT 属性索引: 未加载 ATT 文件");
            return;
        }
        println!(
            "[rvm-import] ATT 属性索引: sections={} attr_sections={} duplicates={} matched={} unmatched={}",
            stats.sections,
            stats.attr_sections,
            stats.duplicate_sections,
            stats.matched_sections,
            stats.unmatched_sections
        );
        let samples = self.att_index.unmatched_samples(5);
        if !samples.is_empty() {
            println!(
                "[rvm-import] ATT 未匹配 section 示例: {}",
                samples.join(", ")
            );
        }
    }

    fn walk_node(
        &mut self,
        store: &Store,
        node_id: NodeId,
        path: &mut VecDeque<String>,
        parent_refno: Option<RefnoEnum>,
        parent_translation: Vec3,
    ) -> Result<()> {
        let node = store
            .get_node(node_id)
            .ok_or_else(|| anyhow!("无效的节点 ID: {}", node_id.0))?;

        match &node.kind {
            NodeKind::File(file) => {
                self.stats.file_nodes += 1;
                let name =
                    sanitize_name(store.get_string(file.info), format!("file_{}", node_id.0));
                path.push_back(name);
                self.walk_children(store, node, path, parent_refno, parent_translation)?;
                path.pop_back();
            }
            NodeKind::Model(model) => {
                self.stats.model_nodes += 1;
                let name =
                    sanitize_name(store.get_string(model.name), format!("model_{}", node_id.0));
                path.push_back(name);
                self.walk_children(store, node, path, parent_refno, parent_translation)?;
                path.pop_back();
            }
            NodeKind::Group(group) => {
                self.stats.group_nodes += 1;
                let name =
                    sanitize_name(store.get_string(group.name), format!("group_{}", node_id.0));
                path.push_back(name.clone());
                let current_path = join_path(path);
                let refno = stable_refno(self.dbnum, &current_path, "group");
                let inst_id = refno.0;
                // CNTB translations in observed E3D RVM exports are absolute world
                // positions in millimetres. They must not be accumulated through the
                // hierarchy; doing so multiplies coordinates by the nesting depth.
                let world_translation = group.translation;
                let world_affine = affine_from_translation(world_translation);

                if self.verbose {
                    println!(
                        "[rvm-import] group path={} refno={} geos={} attrs={}",
                        current_path,
                        refno.0,
                        count_group_geometries(group.first_geometry, store),
                        group.attributes.len()
                    );
                }

                let attrs_json = self.group_attrs_json(store, group, &name)?;
                self.inst_relates.push(InstRelateRecord {
                    refno: RefnoEnum::from(refno),
                    inst_id,
                    parent_refno,
                    world_matrix: Some(encode_affine_blob(&world_affine)?),
                    // spec 009:保留组名(末段)供身份解析与对拍报告;
                    // T003 解析器接入前 identity 仍为 stable_hash。
                    name: Some(name.clone()),
                    noun: None,
                    identity_source: Some("stable_hash".to_string()),
                    resolved: false,
                    attrs_json,
                });

                let mut geometry_link = group.first_geometry;
                let mut geometry_index = 0usize;
                while let Some(geometry_id) = geometry_link {
                    let geometry = store
                        .get_geometry(geometry_id)
                        .ok_or_else(|| anyhow!("无效的几何 ID: {}", geometry_id.0))?;
                    geometry_index += 1;
                    self.push_geometry(inst_id, &current_path, geometry_index, geometry)?;
                    geometry_link = geometry.next;
                }

                self.walk_children(
                    store,
                    node,
                    path,
                    Some(RefnoEnum::from(refno)),
                    world_translation,
                )?;
                path.pop_back();
            }
        }

        Ok(())
    }

    fn walk_children(
        &mut self,
        store: &Store,
        node: &rvm_rs::store::node::Node,
        path: &mut VecDeque<String>,
        parent_refno: Option<RefnoEnum>,
        parent_translation: Vec3,
    ) -> Result<()> {
        let mut child = node.first_child;
        while let Some(child_id) = child {
            let child_node = store
                .get_node(child_id)
                .ok_or_else(|| anyhow!("无效的子节点 ID: {}", child_id.0))?;
            self.walk_node(store, child_id, path, parent_refno, parent_translation)?;
            child = child_node.next;
        }
        Ok(())
    }

    fn push_geometry(
        &mut self,
        inst_id: u64,
        group_path: &str,
        geometry_index: usize,
        geometry: &Geometry,
    ) -> Result<()> {
        self.stats.geometry_records += 1;
        let geo_hash = stable_geo_hash(self.dbnum, group_path, geometry_index, geometry);
        let final_bbox = scale_bbox(geometry.bbox_world, 1000.0);
        let geometry_blob = encode_geometry_blob(geometry, group_path, geometry_index, final_bbox)?;

        // spec 009:rvm-rs 的 bbox_world 对部分带 transform 的原语退化为点
        // (VALVE 12 原语实测全为零尺寸盒)。优先用原语参数 mesh 化后的精确
        // 包围盒,mesh 不可用时回退 rvm-rs bbox。
        let computed = compute_payload_aabb(&geometry_blob);
        let (min_v, max_v) = match computed {
            Some(b) => (Some([b[0], b[1], b[2]]), Some([b[3], b[4], b[5]])),
            // mesh 化失败或退化(零矩阵坍缩)时落 NULL——
            // rvm-rs 的 bbox_world 在同样的零矩阵下也是点,不可回退采信。
            None => (None, None),
        };

        self.inst_geos.push(InstGeoRecord {
            hash: geo_hash,
            geometry: geometry_blob,
            aabb_min_x: min_v.map(|v| v[0]),
            aabb_min_y: min_v.map(|v| v[1]),
            aabb_min_z: min_v.map(|v| v[2]),
            aabb_max_x: max_v.map(|v| v[0]),
            aabb_max_y: max_v.map(|v| v[1]),
            aabb_max_z: max_v.map(|v| v[2]),
            meshed: false,
        });
        self.geo_relates.push((inst_id, geo_hash));
        Ok(())
    }

    fn group_attrs_json(
        &mut self,
        store: &Store,
        group: &rvm_rs::store::node::GroupNode,
        name: &str,
    ) -> Result<Option<String>> {
        let mut map = self.att_index.attrs_for_group(name);

        for attr in &group.attributes {
            let key = store.get_string(attr.key).trim();
            if key.is_empty() {
                continue;
            }
            let value = store.get_string(attr.value).trim();
            map.insert(
                key.to_string(),
                serde_json::Value::String(value.to_string()),
            );
        }

        if map.is_empty() {
            return Ok(None);
        }

        serde_json::to_string(&serde_json::Value::Object(map))
            .map(Some)
            .context("序列化 RVM ATT 属性失败")
    }
}

/// 用原语参数 mesh 化(已应用 transform)后取顶点精确 AABB。
///
/// 退化检测:rvm-rs 对部分原语解析出的 transform.matrix3 为全零矩阵
/// (VALVE 2013286704_483 的 12 个原语实测),顶点会坍缩成一个点——
/// 这种退化盒不可用于对拍,返回 None(调用方落 NULL,compare 跳过并可识别)。
fn compute_payload_aabb(geometry_blob: &[u8]) -> Option<[f64; 6]> {
    let payload: serde_json::Value = serde_json::from_slice(geometry_blob).ok()?;
    let mesh = crate::rvm_obj_export::mesh_from_payload(&payload).ok()??;
    if mesh.vertices.is_empty() {
        return None;
    }
    let mut bbox = [
        f64::INFINITY,
        f64::INFINITY,
        f64::INFINITY,
        f64::NEG_INFINITY,
        f64::NEG_INFINITY,
        f64::NEG_INFINITY,
    ];
    for v in &mesh.vertices {
        bbox[0] = bbox[0].min(v.x as f64);
        bbox[1] = bbox[1].min(v.y as f64);
        bbox[2] = bbox[2].min(v.z as f64);
        bbox[3] = bbox[3].max(v.x as f64);
        bbox[4] = bbox[4].max(v.y as f64);
        bbox[5] = bbox[5].max(v.z as f64);
    }
    if !bbox.iter().all(|v| v.is_finite()) {
        return None;
    }
    // 三轴全部退化(< 0.01mm)视为零矩阵坍缩,弃用。
    let degenerate = (0..3).all(|i| (bbox[i + 3] - bbox[i]).abs() < 0.01);
    (!degenerate).then_some(bbox)
}

fn sanitize_name(raw: &str, fallback: String) -> String {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        fallback
    } else {
        trimmed.replace('\n', " ")
    }
}

fn join_path(path: &VecDeque<String>) -> String {
    path.iter()
        .map(String::as_str)
        .collect::<Vec<_>>()
        .join("/")
}

fn count_group_geometries(
    mut first: Option<rvm_rs::store::geometry::GeometryId>,
    store: &Store,
) -> usize {
    let mut count = 0usize;
    while let Some(id) = first {
        count += 1;
        first = store.get_geometry(id).and_then(|geo| geo.next);
    }
    count
}

fn load_att_attr_index(att_paths: &[PathBuf]) -> Result<AttAttrIndex> {
    let mut index = AttAttrIndex::default();
    for att_path in att_paths {
        let att_text = fs::read_to_string(att_path)
            .with_context(|| format!("读取 ATT 文件失败: {}", att_path.display()))?;
        merge_att_attr_index(&att_text, &mut index);
    }
    Ok(index)
}

fn merge_att_attr_index(att_text: &str, index: &mut AttAttrIndex) {
    let mut current_name: Option<String> = None;

    for line in att_text.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }

        if let Some(name) = trimmed.strip_prefix("NEW") {
            let name = name.trim().trim_matches(|c| c == '\'' || c == '"');
            current_name = (!name.is_empty()).then(|| name.to_string());
            if let Some(name) = current_name.as_ref() {
                index.start_section(name);
            }
            continue;
        }

        if trimmed == "END" {
            current_name = None;
            continue;
        }

        let Some(name) = current_name.as_ref() else {
            continue;
        };
        for part in trimmed.split("&end&") {
            let part = part.trim();
            let Some((key, value)) = part.split_once(":=") else {
                continue;
            };
            let key = key.trim();
            if key.is_empty() {
                continue;
            }
            let value = value.trim().trim_matches(|c| c == '\'' || c == '"');
            index.entry(name).insert(
                key.to_string(),
                serde_json::Value::String(value.to_string()),
            );
        }
    }

    index.retain_non_empty();
}

fn stable_refno(dbnum: u32, path: &str, kind: &str) -> RefU64 {
    let mut hasher = XxHash64::with_seed(0x52_56_4d);
    hasher.write_u32(dbnum);
    hasher.write(kind.as_bytes());
    hasher.write(path.as_bytes());
    let low = (hasher.finish() as u32).max(1);
    RefU64::from(((dbnum as u64) << 32) | low as u64)
}

fn stable_geo_hash(
    dbnum: u32,
    group_path: &str,
    geometry_index: usize,
    geometry: &Geometry,
) -> u64 {
    let mut hasher = XxHash64::with_seed(0x67_65_6f);
    hasher.write_u32(dbnum);
    hasher.write(group_path.as_bytes());
    hasher.write_usize(geometry_index);
    hasher.write(geometry_signature(geometry).as_bytes());
    let hash = hasher.finish() & (i64::MAX as u64);
    hash.max(10)
}

fn geometry_signature(geometry: &Geometry) -> String {
    format!(
        "kind={:?}|type={}|color={}|rgb={}|bbox={:?}:{:?}",
        geometry.kind,
        geometry_type_name(geometry.geo_type),
        geometry.color,
        geometry.color_rgb,
        geometry.bbox_local.min,
        geometry.bbox_local.max
    )
}

fn geometry_kind_name(kind: &GeometryKind) -> &'static str {
    match kind {
        GeometryKind::Pyramid(_) => "Pyramid",
        GeometryKind::Box(_) => "Box",
        GeometryKind::RectangularTorus(_) => "RectangularTorus",
        GeometryKind::CircularTorus(_) => "CircularTorus",
        GeometryKind::EllipticalDish(_) => "EllipticalDish",
        GeometryKind::SphericalDish(_) => "SphericalDish",
        GeometryKind::Snout(_) => "Snout",
        GeometryKind::Cylinder(_) => "Cylinder",
        GeometryKind::Sphere(_) => "Sphere",
        GeometryKind::Line(_) => "Line",
        GeometryKind::FacetGroup(_) => "FacetGroup",
    }
}

fn geometry_detail_payload(kind: &GeometryKind) -> serde_json::Value {
    match kind {
        GeometryKind::Pyramid(data) => json!({
            "pyramid": {
                "bottom": data.bottom,
                "top": data.top,
                "offset": data.offset,
                "height": data.height,
            }
        }),
        GeometryKind::Box(data) => json!({
            "box": {
                "lengths": data.lengths,
            }
        }),
        GeometryKind::RectangularTorus(data) => json!({
            "rectangular_torus": {
                "inner_radius": data.inner_radius,
                "outer_radius": data.outer_radius,
                "height": data.height,
                "angle": data.angle,
            }
        }),
        GeometryKind::CircularTorus(data) => json!({
            "circular_torus": {
                "offset": data.offset,
                "radius": data.radius,
                "angle": data.angle,
            }
        }),
        GeometryKind::EllipticalDish(data) => json!({
            "elliptical_dish": {
                "base_radius": data.base_radius,
                "height": data.height,
            }
        }),
        GeometryKind::SphericalDish(data) => json!({
            "spherical_dish": {
                "base_radius": data.base_radius,
                "height": data.height,
            }
        }),
        GeometryKind::Snout(data) => json!({
            "snout": {
                "radius_bottom": data.radius_bottom,
                "radius_top": data.radius_top,
                "height": data.height,
                "offset_x": data.offset_x,
                "offset_y": data.offset_y,
                "bottom_shear_x": data.bottom_shear_x,
                "bottom_shear_y": data.bottom_shear_y,
                "top_shear_x": data.top_shear_x,
                "top_shear_y": data.top_shear_y,
            }
        }),
        GeometryKind::Cylinder(data) => json!({
            "cylinder": {
                "radius": data.radius,
                "height": data.height,
            }
        }),
        GeometryKind::Sphere(data) => json!({
            "sphere": {
                "radius": data.radius,
            }
        }),
        GeometryKind::Line(data) => json!({
            "line": {
                "start_radius": data.start_radius,
                "end_radius": data.end_radius,
            }
        }),
        GeometryKind::FacetGroup(data) => json!({
            "facet_group": {
                "polygons": data.polygons.iter().map(|polygon| {
                    json!({
                        "contours": polygon.contours.iter().map(|contour| {
                            json!({
                                "vertices": contour.vertices.iter().map(|v| [v.x, v.y, v.z]).collect::<Vec<_>>(),
                                "normals": contour.normals.iter().map(|n| [n.x, n.y, n.z]).collect::<Vec<_>>(),
                            })
                        }).collect::<Vec<_>>()
                    })
                }).collect::<Vec<_>>()
            }
        }),
    }
}

fn geometry_type_name(geo_type: GeometryType) -> &'static str {
    match geo_type {
        GeometryType::Primitive => "Primitive",
        GeometryType::Obstruction => "Obstruction",
        GeometryType::Insulation => "Insulation",
    }
}

fn affine_from_translation(translation: Vec3) -> Affine3A {
    let mut affine = Affine3A::IDENTITY;
    affine.translation = Vec3A::new(translation.x, translation.y, translation.z);
    affine
}

fn encode_affine_blob(affine: &Affine3A) -> Result<Vec<u8>> {
    let matrix = matrix3_to_array(&affine.matrix3);
    let translation = [
        affine.translation.x,
        affine.translation.y,
        affine.translation.z,
    ];
    serialize(&(matrix, translation)).context("序列化 world_matrix 失败")
}

fn matrix3_to_array(matrix: &Mat3A) -> [f32; 9] {
    [
        matrix.x_axis.x,
        matrix.x_axis.y,
        matrix.x_axis.z,
        matrix.y_axis.x,
        matrix.y_axis.y,
        matrix.y_axis.z,
        matrix.z_axis.x,
        matrix.z_axis.y,
        matrix.z_axis.z,
    ]
}

fn scale_bbox(bbox: rvm_rs::math::BBox3, scale: f32) -> rvm_rs::math::BBox3 {
    if !bbox.is_valid() {
        return bbox;
    }
    rvm_rs::math::BBox3::from_min_max(bbox.min * scale, bbox.max * scale)
}

fn encode_geometry_blob(
    geometry: &Geometry,
    group_path: &str,
    geometry_index: usize,
    final_bbox: rvm_rs::math::BBox3,
) -> Result<Vec<u8>> {
    let transform = json!({
        "matrix3": matrix3_to_array(&geometry.transform.matrix3),
        "translation": [
            geometry.transform.translation.x * 1000.0,
            geometry.transform.translation.y * 1000.0,
            geometry.transform.translation.z * 1000.0,
        ],
    });
    let bbox_local = json!({
        "min": [geometry.bbox_local.min.x, geometry.bbox_local.min.y, geometry.bbox_local.min.z],
        "max": [geometry.bbox_local.max.x, geometry.bbox_local.max.y, geometry.bbox_local.max.z],
    });
    let bbox_world = json!({
        "min": [final_bbox.min.x, final_bbox.min.y, final_bbox.min.z],
        "max": [final_bbox.max.x, final_bbox.max.y, final_bbox.max.z],
    });
    let payload = json!({
        "source": "rvm-rs",
        "group_path": group_path,
        "geometry_index": geometry_index,
        "kind": geometry_kind_name(&geometry.kind),
        "kind_debug": format!("{:?}", geometry.kind),
        "detail": geometry_detail_payload(&geometry.kind),
        "geo_type": geometry_type_name(geometry.geo_type),
        "color": geometry.color,
        "color_rgb": geometry.color_rgb,
        "transparency": geometry.transparency,
        "sample_start_angle": geometry.sample_start_angle,
        "transform": transform,
        "bbox_local": bbox_local,
        "bbox_world": bbox_world,
    });
    serde_json::to_vec(&payload).context("序列化 inst_geo.geometry 失败")
}
