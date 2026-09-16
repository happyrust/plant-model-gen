//! 全量初始化解析生成的 PE 图种子。
//!
//! rkyv 只负责序列化；本模块不提供缓存或查询能力。持久层级关系始终以
//! `pe.owner`（向上）和 `pe_owner`（向下）为准。

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

use aios_core::{NamedAttrMap, RefU64, RefnoEnum, SurrealQueryExt, project_primary_db};
use anyhow::{Context, Result};
use serde::Deserialize;
use sha2::{Digest, Sha256};
use surrealdb::types::SurrealValue;
use tokio::sync::OnceCell;

pub const PE_GRAPH_SEED_VERSION: u32 = 1;
pub const ORDER_NONE: u32 = u32::MAX;

const SEED_DIRNAME: &str = "pe_graph";
const HEADER_LEN: usize = 64;
const MAGIC: &[u8; 8] = b"PEGRKYV1";
const META_STATE_NOT_READY: &str = "not_ready";
const META_STATE_READY: &str = "ready";

static PE_GRAPH_SEED_META_SCHEMA_INIT: OnceCell<()> = OnceCell::const_new();

#[derive(Debug, Clone, rkyv::Archive, rkyv::Serialize, rkyv::Deserialize)]
pub struct PeGraphSeedV1 {
    pub version: u32,
    pub dbnum: u32,
    pub sesno: u32,
    pub file_name: String,
    pub scope_hash: String,
    pub ref0_set_hash: String,
    pub node_count: u64,
    pub edge_count: u64,
    pub nodes: Vec<PeGraphNodeV1>,
}

#[derive(Debug, Clone, rkyv::Archive, rkyv::Serialize, rkyv::Deserialize)]
pub struct PeGraphNodeV1 {
    pub refno: u64,
    /// 根节点的 owner 规范化为自身 refno。
    pub owner: u64,
    pub order: u32,
    pub child_count: u32,
    pub noun: String,
    pub name: String,
    pub cata_hash: Option<String>,
}

#[derive(Debug, Default)]
pub struct PeGraphSeedBuilder {
    nodes: HashMap<RefU64, NodeDraft>,
}

#[derive(Debug)]
struct NodeDraft {
    owner: u64,
    noun: String,
    name: String,
    cata_hash: Option<String>,
}

#[derive(Debug, Clone)]
pub struct PublishedSeed {
    pub dbnum: u32,
    pub sesno: u32,
    pub file_name: String,
    pub scope_hash: String,
    pub payload_sha256: String,
    pub node_count: usize,
    pub edge_count: usize,
    pub path: PathBuf,
}

#[derive(Debug, Clone)]
pub struct PeGraphIntegrity {
    pub node_count: usize,
    pub edge_count: usize,
    pub hierarchy_hash: String,
}

#[derive(Debug, Deserialize, SurrealValue)]
struct PeAuditRow {
    id: RefnoEnum,
    #[serde(default)]
    owner: Option<RefnoEnum>,
    #[serde(default)]
    child_count: Option<i64>,
}

#[derive(Debug, Deserialize, SurrealValue)]
struct PeOwnerAuditRow {
    child: RefnoEnum,
    parent: RefnoEnum,
    ordinal: i64,
}

impl PeGraphSeedBuilder {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn is_empty(&self) -> bool {
        self.nodes.is_empty()
    }

    pub fn absorb(&mut self, refno: RefU64, att: &NamedAttrMap) {
        let raw_owner = att.get_owner().refno().0;
        self.nodes.entry(refno).or_insert_with(|| NodeDraft {
            owner: if raw_owner == 0 { refno.0 } else { raw_owner },
            noun: att.get_type_str().to_string(),
            name: att.get_name_or_default(),
            cata_hash: att.cal_cata_hash().map(|hash| hash.to_string()),
        });
    }

    pub fn finish(
        self,
        dbnum: u32,
        sesno: u32,
        file_name: &str,
        children_map: &HashMap<RefU64, Vec<RefU64>>,
    ) -> PeGraphSeedV1 {
        let materialized = self.nodes.keys().copied().collect::<BTreeSet<_>>();
        let mut order_of = HashMap::with_capacity(self.nodes.len());
        for (owner, children) in children_map {
            for (order, child) in children.iter().enumerate() {
                if materialized.contains(owner)
                    && materialized.contains(child)
                    && self
                        .nodes
                        .get(child)
                        .is_some_and(|draft| draft.owner == owner.0)
                {
                    order_of.entry((*owner, *child)).or_insert(order as u32);
                }
            }
        }
        let mut children_by_owner: BTreeMap<RefU64, Vec<RefU64>> = BTreeMap::new();
        for (child, draft) in &self.nodes {
            let owner = RefU64(draft.owner);
            if owner != *child && materialized.contains(&owner) {
                children_by_owner.entry(owner).or_default().push(*child);
            }
        }
        let mut used_orders_by_owner: HashMap<RefU64, BTreeSet<u32>> = HashMap::new();
        for ((owner, _), order) in &order_of {
            used_orders_by_owner
                .entry(*owner)
                .or_default()
                .insert(*order);
        }
        for (owner, children) in &mut children_by_owner {
            children.sort_unstable();
            let used_orders = used_orders_by_owner.entry(*owner).or_default();
            let mut next_order = 0u32;
            for child in children {
                if order_of.contains_key(&(*owner, *child)) {
                    continue;
                }
                while used_orders.contains(&next_order) {
                    next_order += 1;
                }
                order_of.insert((*owner, *child), next_order);
                used_orders.insert(next_order);
                next_order += 1;
            }
        }

        let mut nodes: Vec<_> = self
            .nodes
            .into_iter()
            .map(|(refno, draft)| PeGraphNodeV1 {
                refno: refno.0,
                owner: draft.owner,
                order: order_of
                    .get(&(RefU64(draft.owner), refno))
                    .copied()
                    .unwrap_or(ORDER_NONE),
                child_count: children_by_owner
                    .get(&refno)
                    .map(|children| children.len() as u32)
                    .unwrap_or_default(),
                noun: draft.noun,
                name: draft.name,
                cata_hash: draft.cata_hash,
            })
            .collect();
        nodes.sort_unstable_by_key(|node| node.refno);

        let scope_hash = scope_hash(dbnum, file_name, &nodes);
        let ref0_set_hash = ref0_set_hash(&nodes);
        let edge_count = nodes
            .iter()
            .filter(|node| {
                node.order != ORDER_NONE
                    && node.owner != node.refno
                    && nodes
                        .binary_search_by_key(&node.owner, |candidate| candidate.refno)
                        .is_ok()
            })
            .count() as u64;
        PeGraphSeedV1 {
            version: PE_GRAPH_SEED_VERSION,
            dbnum,
            sesno,
            file_name: file_name.to_string(),
            scope_hash,
            ref0_set_hash,
            node_count: nodes.len() as u64,
            edge_count,
            nodes,
        }
    }
}

/// 悬空 owner：`owner != 自身` 且不在本 seed 物化集内。PDMS 每个 dbfile 层级自闭
/// （跨库是引用而非 ownership），因此这基本等于脏数据。`finish()` 会把它变成孤儿根、
/// 丢边，审计因构造对称必然通过——所以需要在发布前单独统计，用于决定是否跳过种子发布。
/// 返回 (悬空计数, (refno, owner) 样例最多 `sample_limit` 条)。
pub fn dangling_owners(seed: &PeGraphSeedV1, sample_limit: usize) -> (usize, Vec<(u64, u64)>) {
    let refnos: BTreeSet<u64> = seed.nodes.iter().map(|node| node.refno).collect();
    let mut count = 0usize;
    let mut samples = Vec::new();
    for node in &seed.nodes {
        if node.owner != node.refno && !refnos.contains(&node.owner) {
            count += 1;
            if samples.len() < sample_limit {
                samples.push((node.refno, node.owner));
            }
        }
    }
    (count, samples)
}

fn scope_hash(dbnum: u32, file_name: &str, nodes: &[PeGraphNodeV1]) -> String {
    let mut hash = Sha256::new();
    hash.update(dbnum.to_le_bytes());
    hash.update((file_name.len() as u64).to_le_bytes());
    hash.update(file_name.as_bytes());
    for node in nodes {
        hash.update(node.refno.to_le_bytes());
    }
    hex::encode(hash.finalize())
}

fn ref0_set_hash(nodes: &[PeGraphNodeV1]) -> String {
    let ref0s = nodes
        .iter()
        .map(|node| (node.refno >> 32) as u32)
        .filter(|ref0| *ref0 != 0 && *ref0 != 0x8000_0001)
        .collect::<BTreeSet<_>>();
    let mut hash = Sha256::new();
    for ref0 in ref0s {
        hash.update(ref0.to_le_bytes());
    }
    hex::encode(hash.finalize())
}

pub async fn ensure_seed_meta_schema() -> Result<()> {
    PE_GRAPH_SEED_META_SCHEMA_INIT
        .get_or_try_init(|| async {
            let sql = r#"
DEFINE TABLE IF NOT EXISTS pe_graph_seed_meta SCHEMAFULL;
DEFINE FIELD IF NOT EXISTS dbnum ON TABLE pe_graph_seed_meta TYPE int;
DEFINE FIELD IF NOT EXISTS state ON TABLE pe_graph_seed_meta TYPE string DEFAULT 'not_ready' ASSERT $value IN ['not_ready', 'ready'];
DEFINE FIELD IF NOT EXISTS sesno ON TABLE pe_graph_seed_meta TYPE option<int>;
DEFINE FIELD IF NOT EXISTS scope_hash ON TABLE pe_graph_seed_meta TYPE option<string>;
DEFINE FIELD IF NOT EXISTS file_name ON TABLE pe_graph_seed_meta TYPE option<string>;
DEFINE FIELD IF NOT EXISTS payload_sha256 ON TABLE pe_graph_seed_meta TYPE option<string>;
DEFINE FIELD IF NOT EXISTS node_count ON TABLE pe_graph_seed_meta TYPE option<int>;
DEFINE FIELD IF NOT EXISTS edge_count ON TABLE pe_graph_seed_meta TYPE option<int>;
DEFINE FIELD IF NOT EXISTS updated_at ON TABLE pe_graph_seed_meta TYPE datetime DEFAULT time::now();
"#;
            project_primary_db()
                .query(sql)
                .await
                .context("定义 pe_graph_seed_meta schema 失败")?
                .check()
                .context("执行 pe_graph_seed_meta schema 失败")?;
            Ok::<(), anyhow::Error>(())
        })
        .await?;
    Ok(())
}

/// 在任何 PE/pe_owner 变更前失效旧种子。
pub async fn mark_not_ready(dbnum: u32) -> Result<()> {
    ensure_seed_meta_schema().await?;
    let sql = format!(
        "UPSERT pe_graph_seed_meta:{dbnum} SET dbnum = {dbnum}, state = '{META_STATE_NOT_READY}', \
         sesno = NONE, scope_hash = NONE, file_name = NONE, payload_sha256 = NONE, \
         node_count = NONE, edge_count = NONE, updated_at = time::now();"
    );
    project_primary_db().query(sql).await?.check()?;
    Ok(())
}

pub async fn publish_ready(seed: &PublishedSeed) -> Result<()> {
    ensure_seed_meta_schema().await?;
    let sql = format!(
        "UPSERT pe_graph_seed_meta:{} SET dbnum = {}, state = '{}', sesno = {}, \
         scope_hash = $scope_hash, file_name = $file_name, payload_sha256 = $payload_sha256, \
         node_count = {}, edge_count = {}, updated_at = time::now();",
        seed.dbnum, seed.dbnum, META_STATE_READY, seed.sesno, seed.node_count, seed.edge_count
    );
    project_primary_db()
        .query(sql)
        .bind(("scope_hash", seed.scope_hash.clone()))
        .bind(("file_name", seed.file_name.clone()))
        .bind(("payload_sha256", seed.payload_sha256.clone()))
        .await?
        .check()?;
    Ok(())
}

pub fn write_seed_file(tree_dir: &Path, seed: &PeGraphSeedV1) -> Result<PublishedSeed> {
    let payload = rkyv::to_bytes::<rkyv::rancor::Error>(seed)
        .map_err(|error| anyhow::anyhow!("序列化 PE 图种子失败: {error:?}"))?;
    let payload_sha256 = hex::encode(Sha256::digest(&payload));
    // 文件名中的 scope/payload 哈希各截断为 16 位 hex，避免深部署目录下撞 Windows
    // MAX_PATH(260)；完整哈希存 pe_graph_seed_meta（PublishedSeed.scope_hash /
    // payload_sha256），加载器只认 meta 精确文件名 + header/全量 SHA 校验，安全性不变。
    let scope_short = &seed.scope_hash[..seed.scope_hash.len().min(16)];
    let payload_short = &payload_sha256[..payload_sha256.len().min(16)];
    let file_name = format!(
        "pe_graph_{}_{}_{}_{}.rkyv",
        seed.dbnum, seed.sesno, scope_short, payload_short
    );
    let dir = tree_dir.join(SEED_DIRNAME);
    std::fs::create_dir_all(&dir)
        .with_context(|| format!("创建 PE 图种子目录失败: {}", dir.display()))?;
    let path = dir.join(&file_name);

    if path.exists() {
        validate_seed_file(&path, &payload_sha256)?;
    } else {
        let tmp = dir.join(format!(".{file_name}.{}.tmp", std::process::id()));
        let mut header = [0u8; HEADER_LEN];
        header[..8].copy_from_slice(MAGIC);
        header[8..12].copy_from_slice(&PE_GRAPH_SEED_VERSION.to_le_bytes());
        header[12..16].copy_from_slice(&(HEADER_LEN as u32).to_le_bytes());
        header[16..24].copy_from_slice(&(payload.len() as u64).to_le_bytes());
        header[24..56].copy_from_slice(&Sha256::digest(&payload));

        let mut file = std::fs::File::create(&tmp)
            .with_context(|| format!("创建 PE 图种子临时文件失败: {}", tmp.display()))?;
        file.write_all(&header)?;
        file.write_all(&payload)?;
        file.sync_all()?;
        std::fs::rename(&tmp, &path)
            .with_context(|| format!("发布 PE 图种子失败: {}", path.display()))?;
    }

    Ok(PublishedSeed {
        dbnum: seed.dbnum,
        sesno: seed.sesno,
        file_name,
        scope_hash: seed.scope_hash.clone(),
        payload_sha256,
        node_count: seed.node_count as usize,
        edge_count: seed.edge_count as usize,
        path,
    })
}

fn validate_seed_file(path: &Path, expected_sha256: &str) -> Result<()> {
    let mut file = std::fs::File::open(path)?;
    let mut header = [0u8; HEADER_LEN];
    file.read_exact(&mut header)?;
    anyhow::ensure!(&header[..8] == MAGIC, "PE 图种子 magic 不匹配");
    anyhow::ensure!(
        u32::from_le_bytes(header[8..12].try_into().unwrap()) == PE_GRAPH_SEED_VERSION,
        "PE 图种子版本不匹配"
    );
    anyhow::ensure!(
        u32::from_le_bytes(header[12..16].try_into().unwrap()) as usize == HEADER_LEN,
        "PE 图种子 header 长度不匹配"
    );
    anyhow::ensure!(
        hex::encode(&header[24..56]) == expected_sha256,
        "PE 图种子 header SHA-256 不匹配"
    );
    let payload_len = u64::from_le_bytes(header[16..24].try_into().unwrap()) as usize;
    let mut payload = vec![0u8; payload_len];
    file.read_exact(&mut payload)?;
    anyhow::ensure!(
        hex::encode(Sha256::digest(&payload)) == expected_sha256,
        "PE 图种子 payload SHA-256 不匹配"
    );
    Ok(())
}

/// 种子文件的完整路径（`<tree_dir>/pe_graph/<file_name>`）。加载器据此打开 Ready 元数据
/// 指向的精确文件，禁止扫描目录猜测候选（ADR 39 行）。
pub fn seed_file_path(tree_dir: &Path, file_name: &str) -> PathBuf {
    tree_dir.join(SEED_DIRNAME).join(file_name)
}

/// 读取并校验种子文件，返回对齐后的 payload 供 kv-mem 装载器做 rkyv archived access。
/// 依次校验 magic / format_version / header 长度 / header 内 SHA-256 / payload SHA-256
/// （ADR 53 行的前缀/长度/SHA 校验）。
pub fn read_seed_payload_aligned(
    path: &Path,
    expected_payload_sha256: &str,
) -> Result<rkyv::util::AlignedVec<16>> {
    let mut file = std::fs::File::open(path)
        .with_context(|| format!("打开 PE 图种子失败: {}", path.display()))?;
    let mut header = [0u8; HEADER_LEN];
    file.read_exact(&mut header)
        .context("读取 PE 图种子 header 失败")?;
    anyhow::ensure!(&header[..8] == MAGIC, "PE 图种子 magic 不匹配");
    anyhow::ensure!(
        u32::from_le_bytes(header[8..12].try_into().unwrap()) == PE_GRAPH_SEED_VERSION,
        "PE 图种子格式版本不匹配"
    );
    anyhow::ensure!(
        u32::from_le_bytes(header[12..16].try_into().unwrap()) as usize == HEADER_LEN,
        "PE 图种子 header 长度不匹配"
    );
    anyhow::ensure!(
        hex::encode(&header[24..56]) == expected_payload_sha256,
        "PE 图种子 header SHA-256 与元数据不一致"
    );
    let payload_len = u64::from_le_bytes(header[16..24].try_into().unwrap()) as usize;
    let mut raw = vec![0u8; payload_len];
    file.read_exact(&mut raw)
        .context("读取 PE 图种子 payload 失败")?;
    anyhow::ensure!(
        hex::encode(Sha256::digest(&raw)) == expected_payload_sha256,
        "PE 图种子 payload SHA-256 校验失败"
    );
    let mut payload = rkyv::util::AlignedVec::<16>::with_capacity(payload_len);
    payload.extend_from_slice(&raw);
    Ok(payload)
}

/// 仅在 Ready 元数据提交成功后清理旧的不可变文件。
pub fn cleanup_stale_after_ready(tree_dir: &Path, keep: &PublishedSeed) {
    let dir = tree_dir.join(SEED_DIRNAME);
    let prefix = format!("pe_graph_{}_", keep.dbnum);
    // 崩溃残留的临时文件形如 `.pe_graph_{dbnum}_..._{pid}.tmp`（点开头，被上面的
    // seed 前缀漏掉）；一并 best-effort 清理。
    let tmp_prefix = format!(".{prefix}");
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        let is_stale_seed =
            name.starts_with(&prefix) && name.ends_with(".rkyv") && name != keep.file_name;
        let is_tmp_residue = name.starts_with(&tmp_prefix) && name.ends_with(".tmp");
        if (is_stale_seed || is_tmp_residue)
            && let Err(error) = std::fs::remove_file(entry.path())
        {
            log::debug!("[pe_graph_seed] 清理旧种子/临时文件失败 {name}: {error}");
        }
    }
}

pub async fn audit_persistent(seed: &PeGraphSeedV1) -> Result<PeGraphIntegrity> {
    anyhow::ensure!(
        seed.node_count == seed.nodes.len() as u64,
        "dbnum={} seed node_count 不匹配",
        seed.dbnum
    );
    let expected_nodes: BTreeMap<u64, (u64, u32)> = seed
        .nodes
        .iter()
        .map(|node| (node.refno, (node.owner, node.child_count)))
        .collect();
    anyhow::ensure!(
        expected_nodes.len() == seed.nodes.len(),
        "dbnum={} seed 存在重复 refno",
        seed.dbnum
    );

    let mut expected_edges: Vec<(u64, u32, u64)> = seed
        .nodes
        .iter()
        .filter(|node| {
            node.owner != node.refno
                && node.order != ORDER_NONE
                && expected_nodes.contains_key(&node.owner)
        })
        .map(|node| (node.owner, node.order, node.refno))
        .collect();
    expected_edges.sort_unstable();
    anyhow::ensure!(
        seed.edge_count == expected_edges.len() as u64,
        "dbnum={} seed edge_count 不匹配",
        seed.dbnum
    );
    audit_expected(seed.dbnum, &expected_nodes, &expected_edges).await
}

/// worker 落完 PE 后串行固化 owner/child_count 与唯一的向下边。
pub async fn persist_hierarchy(seed: &PeGraphSeedV1) -> Result<()> {
    let ref0s = seed
        .nodes
        .iter()
        .map(|node| (node.refno >> 32) as u32)
        .filter(|ref0| *ref0 != 0 && *ref0 != 0x8000_0001)
        .collect::<BTreeSet<_>>();
    let old_edges = load_pe_owner_edges(&ref0s).await?;

    // Phase 1：删旧边（两种 id 形态都覆盖）。
    let deletes = old_edges
        .into_iter()
        .map(|(parent, _, _)| parent)
        .collect::<BTreeSet<_>>()
        .into_iter()
        .flat_map(|parent| {
            let key = RefU64(parent).to_pe_key();
            [
                format!("DELETE {key}<-pe_owner;"),
                format!("DELETE pe_owner:[{key}, 0]..=[{key}, 4294967295];"),
            ]
        })
        .collect::<Vec<_>>();

    // Phase 2：更新 owner/child_count。
    let updates = seed
        .nodes
        .iter()
        .map(|node| {
            let key = RefU64(node.refno).to_pe_key();
            let owner = RefU64(node.owner).to_pe_key();
            format!(
                "UPDATE {key} SET owner = {owner}, child_count = {};",
                node.child_count
            )
        })
        .collect::<Vec<_>>();

    // Phase 3：重建唯一的向下边。
    let inserts = seed
        .nodes
        .iter()
        .filter(|node| node.owner != node.refno && node.order != ORDER_NONE)
        .map(|node| {
            let child = RefU64(node.refno).to_pe_key();
            let parent = RefU64(node.owner).to_pe_key();
            format!(
                "INSERT RELATION INTO pe_owner {{ id: pe_owner:[{parent}, {}], in: {child}, out: {parent} }};",
                node.order
            )
        })
        .collect::<Vec<_>>();

    // 三段各自分请求提交：删段整体先落库，再改，再插——规避 versioned 引擎
    // “同请求删边→重插同 id”撞 unique_pe_owner 的边界（见 sesno_increment.rs 注释）。
    for (phase, statements) in [("删边", deletes), ("改属性", updates), ("插边", inserts)] {
        for batch in statements.chunks(200) {
            project_primary_db()
                .query(batch.join("\n"))
                .await
                .with_context(|| format!("固化 PE/pe_owner 层级失败({phase})"))?
                .check()
                .with_context(|| format!("执行 PE/pe_owner 层级语句失败({phase})"))?;
        }
    }
    Ok(())
}

/// partial/closure 裁剪解析的 scoped pe_owner 边固化（Q3）。
///
/// 与 full 的 `persist_hierarchy` 不同：只调解“本次写入集 W 中的 parent ∪ W 节点的 owner
/// （在库内存在的）”，用本次文件的 `children_map`（覆盖全文件）∩ 库内存在的 pe 行重建这些
/// parent 的边并重算 `child_count`；**不发 Ready、不碰 bulk_state、不写种子**（partial 只标
/// seed NotReady）。目的：裁剪解析（bran-mem 调试流 / DESI 白名单）落库后 latest 层级查询
/// 可用，同时审计门控消费方继续 fail-closed。
pub async fn persist_partial_hierarchy(
    dbnum: u32,
    written: &BTreeSet<u64>,
    children_map: &HashMap<RefU64, Vec<RefU64>>,
) -> Result<()> {
    // 1) 候选 parent = W 中的 parent ∪ 成员命中 W 的 parent（即 W 节点的 owner）。
    let mut candidate_parents: BTreeSet<u64> = BTreeSet::new();
    for (parent, children) in children_map {
        if written.contains(&parent.0) || children.iter().any(|child| written.contains(&child.0)) {
            candidate_parents.insert(parent.0);
        }
    }
    if candidate_parents.is_empty() {
        return Ok(());
    }

    // 2) 判存：W 节点必存在；候选 parent 与其成员中不在 W 的部分批量查库确认。
    let mut need_check: BTreeSet<u64> = BTreeSet::new();
    for &parent in &candidate_parents {
        if !written.contains(&parent) {
            need_check.insert(parent);
        }
        if let Some(children) = children_map.get(&RefU64(parent)) {
            for child in children {
                if !written.contains(&child.0) {
                    need_check.insert(child.0);
                }
            }
        }
    }
    let mut exists: BTreeSet<u64> = written.iter().copied().collect();
    let need_vec: Vec<u64> = need_check.into_iter().collect();
    for chunk in need_vec.chunks(500) {
        if chunk.is_empty() {
            continue;
        }
        let keys = chunk
            .iter()
            .map(|refno| RefU64(*refno).to_pe_key())
            .collect::<Vec<_>>()
            .join(", ");
        let present: Vec<RefnoEnum> = project_primary_db()
            .query_take(
                &format!("SELECT VALUE id FROM pe WHERE dbnum = {dbnum} AND id IN [{keys}];"),
                0,
            )
            .await
            .context("partial 固化：批量判存 pe 行失败")?;
        for row in present {
            exists.insert(row.refno().0);
        }
    }

    // 3) 逐候选 parent 重建边 + 重算 child_count（删段/改段/插段分请求提交）。
    let mut deletes: Vec<String> = Vec::new();
    let mut updates: Vec<String> = Vec::new();
    let mut inserts: Vec<String> = Vec::new();
    for &parent in &candidate_parents {
        if !exists.contains(&parent) {
            continue;
        }
        let owner_key = RefU64(parent).to_pe_key();
        let desired: Vec<u64> = children_map
            .get(&RefU64(parent))
            .map(|children| {
                children
                    .iter()
                    .map(|child| child.0)
                    .filter(|child| *child != parent && exists.contains(child))
                    .collect()
            })
            .unwrap_or_default();
        deletes.push(format!("DELETE {owner_key}<-pe_owner;"));
        deletes.push(format!(
            "DELETE pe_owner:[{owner_key}, 0]..=[{owner_key}, 4294967295];"
        ));
        updates.push(format!(
            "UPDATE {owner_key} SET child_count = {};",
            desired.len()
        ));
        let rows: Vec<String> = desired
            .iter()
            .enumerate()
            .map(|(order, child)| {
                format!(
                    "{{ id: pe_owner:[{owner_key}, {order}], in: {}, out: {owner_key} }}",
                    RefU64(*child).to_pe_key()
                )
            })
            .collect();
        for chunk in rows.chunks(500) {
            inserts.push(format!("INSERT RELATION INTO pe_owner [{}];", chunk.join(",")));
        }
    }

    for (phase, statements) in [("删边", deletes), ("改属性", updates), ("插边", inserts)] {
        for batch in statements.chunks(200) {
            project_primary_db()
                .query(batch.join("\n"))
                .await
                .with_context(|| format!("partial scoped 固化 pe_owner 失败({phase})"))?
                .check()
                .with_context(|| format!("执行 partial scoped pe_owner 语句失败({phase})"))?;
        }
    }
    Ok(())
}

pub async fn audit_expected(
    dbnum: u32,
    expected_nodes: &BTreeMap<u64, (u64, u32)>,
    expected_edges: &[(u64, u32, u64)],
) -> Result<PeGraphIntegrity> {
    let expected_child_count: usize = expected_nodes
        .iter()
        .map(|(_, (_, child_count))| *child_count as usize)
        .sum();
    anyhow::ensure!(
        expected_child_count == expected_edges.len(),
        "dbnum={} 完整解析闭包不完整: child_count={} edges={}",
        dbnum,
        expected_child_count,
        expected_edges.len()
    );

    let actual_nodes = load_pe_nodes(dbnum).await?;
    anyhow::ensure!(
        &actual_nodes == expected_nodes,
        "dbnum={} PE 审计失败: expected_nodes={} actual_nodes={}",
        dbnum,
        expected_nodes.len(),
        actual_nodes.len()
    );

    let ref0s = expected_nodes
        .keys()
        .map(|refno| (refno >> 32) as u32)
        .filter(|ref0| *ref0 != 0 && *ref0 != 0x8000_0001)
        .collect::<BTreeSet<_>>();
    let mut actual_edges = load_pe_owner_edges(&ref0s).await?;
    actual_edges.sort_unstable();
    anyhow::ensure!(
        actual_edges == expected_edges,
        "dbnum={} pe_owner 审计失败: expected_edges={} actual_edges={}",
        dbnum,
        expected_edges.len(),
        actual_edges.len()
    );

    Ok(PeGraphIntegrity {
        node_count: actual_nodes.len(),
        edge_count: actual_edges.len(),
        hierarchy_hash: hierarchy_hash(&actual_nodes, &actual_edges),
    })
}

async fn load_pe_nodes(dbnum: u32) -> Result<BTreeMap<u64, (u64, u32)>> {
    const PAGE: usize = 2000;
    let mut nodes = BTreeMap::new();
    let mut cursor: Option<String> = None;
    loop {
        let sql = match &cursor {
            Some(last) => format!(
                "SELECT id, owner, child_count FROM pe WHERE dbnum = {dbnum} AND id > {last} \
                 ORDER BY id LIMIT {PAGE};"
            ),
            None => format!(
                "SELECT id, owner, child_count FROM pe WHERE dbnum = {dbnum} \
                 ORDER BY id LIMIT {PAGE};"
            ),
        };
        let rows: Vec<PeAuditRow> = project_primary_db().query_take(&sql, 0).await?;
        let count = rows.len();
        if count == 0 {
            break;
        }
        cursor = rows.last().map(|row| row.id.to_pe_key());
        for row in rows {
            let child_count = row.child_count.unwrap_or_default();
            anyhow::ensure!(child_count >= 0, "PE child_count 不能为负数");
            nodes.insert(
                row.id.refno().0,
                (
                    row.owner.map(|owner| owner.refno().0).unwrap_or_default(),
                    child_count as u32,
                ),
            );
        }
        if count < PAGE {
            break;
        }
    }
    Ok(nodes)
}

async fn load_pe_owner_edges(ref0s: &BTreeSet<u32>) -> Result<Vec<(u64, u32, u64)>> {
    const PAGE: usize = 100_000;
    let mut edges = Vec::new();
    // 游标分页（id > last），对齐 load_pe_nodes；避免 START 偏移分页的页成本递增。
    // pe_owner id 的构造恒为 `pe_owner:[out, ordinal]`（见 persist_hierarchy），
    // 因此可由本页最后一行的 out(parent) + ordinal 精确重建游标。
    let mut cursor: Option<String> = None;
    loop {
        let sql = match &cursor {
            Some(last) => format!(
                "SELECT id, in AS child, out AS parent, record::id(id)[1] AS ordinal \
                 FROM pe_owner WHERE id > {last} ORDER BY id LIMIT {PAGE};"
            ),
            None => format!(
                "SELECT id, in AS child, out AS parent, record::id(id)[1] AS ordinal \
                 FROM pe_owner ORDER BY id LIMIT {PAGE};"
            ),
        };
        let rows: Vec<PeOwnerAuditRow> = project_primary_db().query_take(&sql, 0).await?;
        let count = rows.len();
        if count == 0 {
            break;
        }
        // 游标取本页最后一行的真实 id（未过滤前）。
        if let Some(last) = rows.last() {
            cursor = Some(format!(
                "pe_owner:[{}, {}]",
                RefU64(last.parent.refno().0).to_pe_key(),
                last.ordinal
            ));
        }
        for row in rows {
            if !ref0s.contains(&((row.parent.refno().0 >> 32) as u32)) {
                continue;
            }
            anyhow::ensure!(row.ordinal >= 0, "pe_owner ordinal 不能为负数");
            edges.push((row.parent.refno().0, row.ordinal as u32, row.child.refno().0));
        }
        if count < PAGE {
            break;
        }
    }
    Ok(edges)
}

pub fn hierarchy_hash(nodes: &BTreeMap<u64, (u64, u32)>, edges: &[(u64, u32, u64)]) -> String {
    let mut hash = Sha256::new();
    for (refno, (owner, child_count)) in nodes {
        hash.update(b"N");
        hash.update(refno.to_le_bytes());
        hash.update(owner.to_le_bytes());
        hash.update(child_count.to_le_bytes());
    }
    for (parent, order, child) in edges {
        hash.update(b"E");
        hash.update(parent.to_le_bytes());
        hash.update(order.to_le_bytes());
        hash.update(child.to_le_bytes());
    }
    hex::encode(hash.finalize())
}
