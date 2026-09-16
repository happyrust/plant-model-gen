//! specs/023 M0/T1：latest（不带 sesno）层级查询原语层 `PeOwnerTreeStore`。
//!
//! 目标：替代旧 `.tree` 文件路径——层级查询全部改走 SurrealDB 3.1
//! **图遍历 / 递归 idiom**（语法依据 `D:\work\plant-code\surrealdb`（dev-3.1）
//! `language-tests/tests/language/graph|idiom` 实测用例），数据源永远是库内最新态：
//!
//! - children（同胞有序）：`SELECT VALUE in FROM <owner><-pe_owner ORDER BY id;`
//!   （边 id = `pe_owner:[<owner>, <order>]`，`ORDER BY id` 保同胞顺序，specs/023 契约）
//! - descendants（子孙收集）：`<root>.{..N+collect}<-pe_owner<-pe`
//!   （递归 idiom，BFS 邻近序 + visited 去重防环；引擎递归上限 256，`recursion_limits` 实测）
//! - ancestors（祖先链）：`<node>.{..N+collect}(.owner)`（owner 记录链接递归）
//! - 批量子节点：按 `pe_owner.order` 查询并分组
//!
//! 铁律：
//! - **禁止 `pe_owner:[..]..[..]` id 区间扫**（specs/023 research C3：VERSION 下静默返回当前态，
//!   latest 同样统一图遍历，不开这个口子）；
//! - **禁止 WHERE 全表扫做层级查询**（noun 枚举/计数是表级统计，不属层级查询，见文件尾注释）；
//! - 不读取 `pe.children`；存量站点必须先执行 `model-version rebuild-pe-owner`。
//!
//! 本模块 M0 阶段纯新增；M1/M2 起逐域替换旧 `.tree` 消费面。

use std::collections::{HashMap, HashSet};

use aios_core::tool::db_tool::{db1_dehash, db1_hash};
use aios_core::{RefnoEnum, SurrealQueryExt, project_primary_db};
use serde::Deserialize;
use surrealdb::types::SurrealValue;

/// 引擎单次递归 idiom 的深度硬上限（surrealdb dev-3.1 `recursion_limits` 实测：
/// 显式上界 ≤256 且超深时**截断**；无上界 `{..}` 超深会直接报错——因此一律显式带上界）。
pub const MAX_RECURSE_DEPTH: usize = 256;

/// 批量点查/展开的分片大小（对齐 sesno_increment `exec_statements` 粒度）。
const CHUNK: usize = 500;

/// 单节点层级元信息（latest 态；对齐 `TreeIndex::node_meta` 的消费面）。
#[derive(Debug, Clone)]
pub struct PeNodeMeta {
    pub refno: RefnoEnum,
    pub owner: Option<RefnoEnum>,
    /// noun 名称（pe.noun 字段原值，如 "BRAN"）
    pub noun: String,
    /// `db1_hash(noun)`，对齐 TreeIndex 消费面的 hash 分组语义
    pub noun_hash: u32,
    /// pe.cata_hash 字段（M0/T2 落地后有值；缺失 = None，消费侧回退 attmap 计算）
    pub cata_hash: Option<u64>,
}

#[derive(Debug, Deserialize, SurrealValue)]
struct PeMetaRow {
    id: RefnoEnum,
    #[serde(default)]
    noun: Option<String>,
    #[serde(default)]
    owner: Option<RefnoEnum>,
    /// cata_hash 以 string 存储（u64 哈希可能超出 Surreal int/i64 范围）
    #[serde(default)]
    cata_hash: Option<String>,
}

#[derive(Debug, Deserialize, SurrealValue)]
struct EdgeRow {
    child: RefnoEnum,
    parent: RefnoEnum,
    ordinal: i64,
}

/// 边行 + 顺带把 child 的 meta 投影出来（`in` 是 child 的 record link）。
///
/// 省掉下一层的 `fetch_node_metas`——同一批节点本来就要问一遍 noun。
/// `child_noun` 为 `None` 时调用方回退点查，所以即使投影语法在某个引擎版本上
/// 失效，也只是退回原来的两轮往返，不会静默把节点判成「无 noun」。
#[derive(Debug, Deserialize, SurrealValue)]
struct EdgeMetaRow {
    child: RefnoEnum,
    parent: RefnoEnum,
    ordinal: i64,
    #[serde(default)]
    child_noun: Option<String>,
    #[serde(default)]
    child_owner: Option<RefnoEnum>,
    #[serde(default)]
    child_cata_hash: Option<String>,
}

#[derive(Debug, Deserialize, SurrealValue)]
struct CountRow {
    p: RefnoEnum,
    #[serde(default)]
    n: Option<i64>,
}

/// 子树下探白名单：目标 noun 集合沿 owner 方向在 AVEVA 字典里的传递闭包。
///
/// 等价于 core.dll `DB_Noun::recurseEleTypes(ATT_OWNER, ...)` 喂给
/// `DB_IteratorCreator` 的 pred2 —— 类型上不可能装下任何目标的节点，整棵子树直接跳过。
/// 闭包由 `aios_core::noun_graph`（desvir.dat 模板库导出）算出，是纯 schema 信息。
pub struct DescendGate {
    allowed: HashSet<u32>,
    /// 字典图里存在的全部 noun。用于 fail-open：字典没收录的 noun（UDA、字典版本差异）
    /// 一律放行。宁可多走一棵子树，也不能静默漏元素。
    known: HashSet<u32>,
    /// 字典里查不到的目标 noun。它们的祖先闭包为空，**通往它们的路径可能被误剪**——
    /// 节点级 fail-open 只能救到该节点本身，救不了半路。E3D 3.1 实测 `PRTELE` / `GPART`
    /// 落在这里（attlib 有记录但 desvir 无模板，且没有任何类型把它们列为成员）。
    unknown_targets: Vec<String>,
}

impl DescendGate {
    pub fn for_targets(nouns: &[&str]) -> Self {
        use aios_core::noun_graph::{NOUN_GRAPH, descend_gate};

        // 目标自身也放行：prune=false 时本来就会穿过命中节点继续走，
        // 而 BRAN/SCTN 这类既是目标又是容器。闭包通常已包含它们，这里只是显式兜底。
        let mut allowed: HashSet<u32> = descend_gate(nouns).iter().map(|n| db1_hash(n)).collect();
        allowed.extend(nouns.iter().map(|n| db1_hash(n)));

        let known: HashSet<u32> = NOUN_GRAPH
            .node_indices()
            .map(|idx| NOUN_GRAPH[idx])
            .collect();

        let unknown_targets = nouns
            .iter()
            .filter(|n| !known.contains(&db1_hash(n)))
            .map(|n| n.to_string())
            .collect::<Vec<_>>();

        let gate = Self {
            allowed,
            known,
            unknown_targets,
        };
        if !gate.unknown_targets.is_empty() {
            log::warn!(
                "[pe_owner_tree] 下探白名单有 {} 个目标 noun 不在 AVEVA 字典图内，\
                 通往它们的路径可能被误剪，务必用对拍验证：{:?}",
                gate.unknown_targets.len(),
                gate.unknown_targets
            );
        }
        log::info!(
            "[pe_owner_tree] 下探白名单就绪：{}/{} 个类型可下探，可剪 {} 个类型的子树",
            gate.allowed.len(),
            gate.known.len(),
            gate.known.len().saturating_sub(gate.allowed.len())
        );
        gate
    }

    pub fn allows(&self, noun_hash: u32) -> bool {
        !self.known.contains(&noun_hash) || self.allowed.contains(&noun_hash)
    }

    /// 白名单大小 / 字典总类型数，用于确认剪枝确实生效。
    pub fn coverage(&self) -> (usize, usize) {
        (self.allowed.len(), self.known.len())
    }

    pub fn unknown_targets(&self) -> &[String] {
        &self.unknown_targets
    }
}

/// 目标收集的配置面，对齐 core.dll `DB_IteratorCreator` 的几个 setter。
#[derive(Default, Clone, Copy)]
pub struct CollectOptions<'a> {
    /// 命中目标后不再展开其子树（`member_nouns` 命中的节点不受此约束）。
    pub prune: bool,
    /// 下探闸门，见 [`DescendGate`]。对应 pred2。
    pub descend_gate: Option<&'a DescendGate>,
    /// 命中这些 noun 的节点，其**直接子节点**全部收进
    /// [`TargetCollectResult::member_children`]（不看子节点自身的 noun）。
    /// 对应 core.dll 的 `DB_PredicateOnOwner`：判据是「我的 owner 的类型」。
    pub member_nouns: Option<&'a HashSet<u32>>,
}

#[derive(Debug, Default)]
pub struct TargetCollectResult {
    /// 命中目标 noun 的 refnos，按 noun_hash 分组。
    pub grouped: HashMap<u32, Vec<RefnoEnum>>,
    /// `member_nouns` 命中节点的直接子节点，noun 不限。
    pub member_children: Vec<RefnoEnum>,
}

/// pe_owner 图查询原语层。
///
/// 层级查询（children/ancestors/descendants/…）是纯图操作，不需要 dbnum；
/// noun 枚举/计数等表级统计按构造时传入的 dbnums 收敛范围（依赖 D3 `idx_pe_dbnum_noun` 索引）。
pub struct PeOwnerTreeStore {
    dbnums: Vec<u32>,
}

impl PeOwnerTreeStore {
    pub fn new(dbnums: Vec<u32>) -> Self {
        Self { dbnums }
    }

    pub fn dbnums(&self) -> &[u32] {
        &self.dbnums
    }

    // ========================================================================
    // children（同胞有序）
    // ========================================================================

    /// 查询直接子节点（同胞顺序 = 边 id `[owner, order]` 升序）。
    ///
    pub async fn query_children(parent: RefnoEnum) -> anyhow::Result<Vec<RefnoEnum>> {
        let parent_key = parent.to_pe_key();
        let sql = format!("SELECT VALUE in FROM {parent_key}<-pe_owner ORDER BY id;");
        Ok(project_primary_db().query_take(&sql, 0).await?)
    }

    /// 查询直接子节点并按 noun 过滤（保持同胞顺序）。
    pub async fn query_children_filtered(
        parent: RefnoEnum,
        nouns: &[&str],
    ) -> anyhow::Result<Vec<RefnoEnum>> {
        let children = Self::query_children(parent).await?;
        if children.is_empty() {
            return Ok(children);
        }
        let wanted: HashSet<u32> = nouns.iter().map(|n| db1_hash(n)).collect();
        let metas = Self::fetch_node_metas(&children).await?;
        Ok(children
            .into_iter()
            .filter(|r| {
                metas
                    .get(r)
                    .map(|m| wanted.contains(&m.noun_hash))
                    .unwrap_or(false)
            })
            .collect())
    }

    /// 批量统计直接子节点数量（children_count 展示用）。
    ///
    pub async fn query_children_counts(
        refnos: &[RefnoEnum],
    ) -> anyhow::Result<HashMap<RefnoEnum, usize>> {
        let mut out = HashMap::with_capacity(refnos.len());
        for chunk in refnos.chunks(CHUNK) {
            let keys = chunk
                .iter()
                .map(|r| r.to_pe_key())
                .collect::<Vec<_>>()
                .join(", ");
            let sql = format!("SELECT VALUE {{ p: id, n: count(<-pe_owner) }} FROM [{keys}];");
            let rows: Vec<CountRow> = project_primary_db().query_take(&sql, 0).await?;
            for row in rows {
                out.insert(row.p, row.n.unwrap_or(0).max(0) as usize);
            }
        }
        Ok(out)
    }

    // ========================================================================
    // ancestors（祖先链）
    // ========================================================================

    /// 查询祖先链，返回顺序与 TreeIndex 一致：根→父，不含自身。
    ///
    /// 走 `(.owner)` 记录链接递归 idiom（单条查询）：owner 字段由 PE 写入路径恒维护，
    /// **不依赖 pe_owner 边完整性**，祖先链又是唯一路径——比边遍历更稳。
    /// `+collect` 去重防环（根节点 owner 自指靠 visited 去重终止）。
    pub async fn query_ancestors(node: RefnoEnum) -> anyhow::Result<Vec<RefnoEnum>> {
        let key = node.to_pe_key();
        let sql = format!("RETURN {key}.{{..{MAX_RECURSE_DEPTH}+collect}}(.owner);");
        let chain: Vec<Option<RefnoEnum>> = project_primary_db().query_take(&sql, 0).await?;
        let mut chain: Vec<RefnoEnum> = chain.into_iter().flatten().collect();
        // 根节点 owner 自指会把自身收进链；剔除后反转为 根→父。
        chain.retain(|r| *r != node);
        chain.reverse();
        Ok(chain)
    }

    /// 祖先链按 noun 过滤（保持根→父顺序）。
    pub async fn query_ancestors_filtered(
        node: RefnoEnum,
        nouns: &[&str],
    ) -> anyhow::Result<Vec<RefnoEnum>> {
        let chain = Self::query_ancestors(node).await?;
        if chain.is_empty() {
            return Ok(chain);
        }
        let wanted: HashSet<u32> = nouns.iter().map(|n| db1_hash(n)).collect();
        let metas = Self::fetch_node_metas(&chain).await?;
        Ok(chain
            .into_iter()
            .filter(|r| {
                metas
                    .get(r)
                    .map(|m| wanted.contains(&m.noun_hash))
                    .unwrap_or(false)
            })
            .collect())
    }

    // ========================================================================
    // descendants（子孙收集）
    // ========================================================================

    /// 查询全部子孙（不含自身），BFS 邻近序。
    ///
    /// 单条递归 idiom `<root>.{..N+collect}<-pe_owner<-pe`。
    pub async fn query_descendants(
        root: RefnoEnum,
        max_depth: Option<usize>,
    ) -> anyhow::Result<Vec<RefnoEnum>> {
        let depth = max_depth
            .unwrap_or(MAX_RECURSE_DEPTH)
            .clamp(1, MAX_RECURSE_DEPTH);
        let root_key = root.to_pe_key();
        let sql = format!("RETURN {root_key}.{{..{depth}+collect}}<-pe_owner<-pe;");
        Ok(project_primary_db().query_take(&sql, 0).await?)
    }

    /// 查询子孙并按 noun 过滤（不含自身）。
    ///
    /// 注意：**过滤发生在收集之后**（先图收集再按 meta 过滤），不能用中途过滤
    /// `<-pe_owner<-(pe WHERE ...)`——那是剪枝语义，会在中间层 noun 不匹配时误断整条链。
    pub async fn query_descendants_filtered(
        root: RefnoEnum,
        nouns: &[&str],
        max_depth: Option<usize>,
    ) -> anyhow::Result<Vec<RefnoEnum>> {
        let all = Self::query_descendants(root, max_depth).await?;
        Self::filter_by_nouns(all, nouns).await
    }

    /// 批量多根子孙收集 + noun 过滤（跨根去重，保持发现顺序）。
    pub async fn query_multi_descendants_filtered(
        roots: &[RefnoEnum],
        nouns: &[&str],
    ) -> anyhow::Result<Vec<RefnoEnum>> {
        let mut seen: HashSet<RefnoEnum> = HashSet::new();
        let mut all: Vec<RefnoEnum> = Vec::new();
        for &root in roots {
            for r in Self::query_descendants(root, None).await? {
                if seen.insert(r) {
                    all.push(r);
                }
            }
        }
        Self::filter_by_nouns(all, nouns).await
    }

    async fn filter_by_nouns(
        refnos: Vec<RefnoEnum>,
        nouns: &[&str],
    ) -> anyhow::Result<Vec<RefnoEnum>> {
        if refnos.is_empty() || nouns.is_empty() {
            return Ok(refnos);
        }
        let wanted: HashSet<u32> = nouns.iter().map(|n| db1_hash(n)).collect();
        let metas = Self::fetch_node_metas(&refnos).await?;
        Ok(refnos
            .into_iter()
            .filter(|r| {
                metas
                    .get(r)
                    .map(|m| wanted.contains(&m.noun_hash))
                    .unwrap_or(false)
            })
            .collect())
    }

    /// 批量取直接子节点，按 `pe_owner.order` 保持同胞顺序。
    pub async fn children_batch(
        parents: &[RefnoEnum],
    ) -> anyhow::Result<HashMap<RefnoEnum, Vec<RefnoEnum>>> {
        let mut out: HashMap<RefnoEnum, Vec<RefnoEnum>> = HashMap::with_capacity(parents.len());
        for chunk in parents.chunks(CHUNK) {
            let keys = chunk
                .iter()
                .map(|r| r.to_pe_key())
                .collect::<Vec<_>>()
                .join(", ");
            for parent in chunk {
                out.entry(*parent).or_default();
            }
            let sql = format!(
                "SELECT id, in AS child, out AS parent, record::id(id)[1] AS ordinal \
                 FROM [{keys}]<-pe_owner ORDER BY out, id;"
            );
            let rows: Vec<EdgeRow> = project_primary_db().query_take(&sql, 0).await?;
            for row in rows {
                anyhow::ensure!(row.ordinal >= 0, "pe_owner ordinal 不能为负数");
                out.entry(row.parent).or_default().push(row.child);
            }
        }
        Ok(out)
    }

    /// 同 `children_batch`，但顺带把每个 child 的 meta 投影回来。
    ///
    /// 逐层 BFS 的成本是「往返数 × 层数」，而 `children_batch` 拿到的 child
    /// 下一层还要再点查一次 noun——同一批节点问两遍。`in` 是 child 的 record
    /// link，直接在投影里取即可把每层两轮往返压成一轮。
    pub async fn children_batch_with_meta(
        parents: &[RefnoEnum],
    ) -> anyhow::Result<(
        HashMap<RefnoEnum, Vec<RefnoEnum>>,
        HashMap<RefnoEnum, PeNodeMeta>,
    )> {
        let mut children: HashMap<RefnoEnum, Vec<RefnoEnum>> =
            HashMap::with_capacity(parents.len());
        let mut metas: HashMap<RefnoEnum, PeNodeMeta> = HashMap::new();
        for chunk in parents.chunks(CHUNK) {
            let keys = chunk
                .iter()
                .map(|r| r.to_pe_key())
                .collect::<Vec<_>>()
                .join(", ");
            for parent in chunk {
                children.entry(*parent).or_default();
            }
            let sql = format!(
                "SELECT in AS child, out AS parent, record::id(id)[1] AS ordinal, \
                 in.noun AS child_noun, in.owner AS child_owner, \
                 in.cata_hash AS child_cata_hash \
                 FROM [{keys}]<-pe_owner ORDER BY out, id;"
            );
            let rows: Vec<EdgeMetaRow> = project_primary_db().query_take(&sql, 0).await?;
            for row in rows {
                anyhow::ensure!(row.ordinal >= 0, "pe_owner ordinal 不能为负数");
                children.entry(row.parent).or_default().push(row.child);
                // noun 缺失一律不写进 meta 表，让调用方走点查回退——
                // 把空 noun 当真值会让节点匹配不上任何目标，是静默漏数据。
                if let Some(noun) = row.child_noun {
                    let noun_hash = db1_hash(&noun);
                    metas.insert(
                        row.child,
                        PeNodeMeta {
                            refno: row.child,
                            owner: row.child_owner,
                            noun,
                            noun_hash,
                            cata_hash: row.child_cata_hash.and_then(|s| s.parse::<u64>().ok()),
                        },
                    );
                }
            }
        }
        Ok((children, metas))
    }

    // ========================================================================
    // 目标收集（剪枝 / 分组）——生成管线入口查询
    // ========================================================================

    /// 批量 BFS 收集目标 noun refnos，匹配后剪枝（不再深入其子树）。
    ///
    /// 与 `HierView::collect_target_refnos_pruned` 语义一致（include_self=true）。
    /// 剪枝无法用单条递归 idiom 表达（中途过滤是"断链"语义），因此在 Rust 侧逐层
    /// BFS + 批量 meta 判定，展开走 `children_batch`（边优先/字段回退）。
    pub async fn collect_target_refnos_pruned(
        roots: &[RefnoEnum],
        nouns: &[&str],
    ) -> anyhow::Result<Vec<RefnoEnum>> {
        let result = Self::collect_targets(
            roots,
            nouns,
            CollectOptions {
                prune: true,
                ..Default::default()
            },
        )
        .await?;
        let mut out = Vec::new();
        for (_, refnos) in result.grouped {
            out.extend(refnos);
        }
        Ok(out)
    }

    /// 批量 BFS 收集目标 noun refnos 并按 noun_hash 分组（include_self=true）。
    ///
    /// 三处与 core.dll `DB_IteratorCreator::getIterator()` 对齐的语义：
    ///
    /// - **接受与下探分离**：`options.descend_gate` 是 pred2，只决定钻不钻子树，
    ///   与「收不收这个节点」无关；
    /// - **member 谓词**：`options.member_nouns` 对应 `DB_PredicateOnOwner`，
    ///   在同一趟里收下命中节点的直接子节点，不用事后再补一轮 children 查询；
    /// - **每层一轮往返**：child 的 meta 由边查询顺带投影回来（见
    ///   [`Self::children_batch_with_meta`]），只有根节点这一层需要点查。
    pub async fn collect_targets(
        roots: &[RefnoEnum],
        nouns: &[&str],
        options: CollectOptions<'_>,
    ) -> anyhow::Result<TargetCollectResult> {
        let wanted: HashSet<u32> = nouns.iter().map(|n| db1_hash(n)).collect();
        let mut result = TargetCollectResult::default();
        let mut visited: HashSet<RefnoEnum> = HashSet::new();
        let mut frontier: Vec<RefnoEnum> = Vec::new();
        for &root in roots {
            if visited.insert(root) {
                frontier.push(root);
            }
        }
        // 上一层边查询顺带带回的 child meta；根节点这层没有，走点查。
        let mut carried: HashMap<RefnoEnum, PeNodeMeta> = HashMap::new();
        let mut level = 0usize;
        let mut gate_skipped = 0usize;
        while !frontier.is_empty() && level <= MAX_RECURSE_DEPTH {
            level += 1;
            let metas = Self::metas_for_frontier(&frontier, &mut carried).await?;
            let mut expand: Vec<RefnoEnum> = Vec::new();
            let mut member_owners: HashSet<RefnoEnum> = HashSet::new();
            for &node in &frontier {
                let meta = metas.get(&node);
                let matched = meta.is_some_and(|m| wanted.contains(&m.noun_hash));
                if matched {
                    let hash = meta.map(|m| m.noun_hash).unwrap_or_default();
                    result.grouped.entry(hash).or_default().push(node);
                }
                // member noun 命中的节点必须展开——它的直接子节点正是要收的东西。
                let is_member_owner = options
                    .member_nouns
                    .zip(meta)
                    .is_some_and(|(set, m)| set.contains(&m.noun_hash));
                if is_member_owner {
                    member_owners.insert(node);
                } else {
                    if matched && options.prune {
                        continue;
                    }
                    // 根节点恒放行（对齐 `DB_Iterator::firstIncludeRoot`）；meta 缺失也放行，
                    // 因为"查不到 noun"不等于"这棵子树里没有目标"。
                    if level > 1
                        && let Some(gate) = options.descend_gate
                        && let Some(m) = meta
                        && !gate.allows(m.noun_hash)
                    {
                        gate_skipped += 1;
                        continue;
                    }
                }
                expand.push(node);
            }
            if expand.is_empty() {
                break;
            }
            let (kids_map, kid_metas) = Self::children_batch_with_meta(&expand).await?;
            carried = kid_metas;
            let mut next: Vec<RefnoEnum> = Vec::new();
            for parent in &expand {
                let Some(kids) = kids_map.get(parent) else {
                    continue;
                };
                if member_owners.contains(parent) {
                    result.member_children.extend(kids.iter().copied());
                }
                for &kid in kids {
                    if visited.insert(kid) {
                        next.push(kid);
                    }
                }
            }
            frontier = next;
        }
        if gate_skipped > 0 {
            log::debug!(
                "[pe_owner_tree] 下探白名单剪掉 {gate_skipped} 棵子树 roots={} visited={}",
                roots.len(),
                visited.len()
            );
        }
        Ok(result)
    }

    /// 取本层 meta：优先用上一层边查询带回来的，缺的才点查。
    ///
    /// 投影若在某个引擎版本上失效，这里会整层落到点查，即退回原来的
    /// 「每层两轮往返」，行为不变、只是不省。
    async fn metas_for_frontier(
        frontier: &[RefnoEnum],
        carried: &mut HashMap<RefnoEnum, PeNodeMeta>,
    ) -> anyhow::Result<HashMap<RefnoEnum, PeNodeMeta>> {
        let mut out: HashMap<RefnoEnum, PeNodeMeta> = HashMap::with_capacity(frontier.len());
        let mut missing: Vec<RefnoEnum> = Vec::new();
        for &node in frontier {
            match carried.remove(&node) {
                Some(meta) => {
                    out.insert(node, meta);
                }
                None => missing.push(node),
            }
        }
        carried.clear();
        if !missing.is_empty() {
            out.extend(Self::fetch_node_metas(&missing).await?);
        }
        Ok(out)
    }

    // ========================================================================
    // 节点元信息
    // ========================================================================

    /// 批量点查节点元信息（chunk 500）。
    pub async fn fetch_node_metas(
        refnos: &[RefnoEnum],
    ) -> anyhow::Result<HashMap<RefnoEnum, PeNodeMeta>> {
        let mut out = HashMap::with_capacity(refnos.len());
        for chunk in refnos.chunks(CHUNK) {
            let keys = chunk
                .iter()
                .map(|r| r.to_pe_key())
                .collect::<Vec<_>>()
                .join(", ");
            let sql = format!("SELECT id, noun, owner, cata_hash FROM [{keys}];");
            let rows: Vec<PeMetaRow> = project_primary_db().query_take(&sql, 0).await?;
            for row in rows {
                let noun = row.noun.unwrap_or_default();
                let noun_hash = db1_hash(&noun);
                out.insert(
                    row.id,
                    PeNodeMeta {
                        refno: row.id,
                        owner: row.owner,
                        noun,
                        noun_hash,
                        cata_hash: row.cata_hash.and_then(|s| s.parse::<u64>().ok()),
                    },
                );
            }
        }
        Ok(out)
    }

    /// 单节点元信息。
    pub async fn get_node_meta(refno: RefnoEnum) -> anyhow::Result<Option<PeNodeMeta>> {
        Ok(Self::fetch_node_metas(&[refno]).await?.remove(&refno))
    }

    /// 节点 noun 名称。
    pub async fn get_noun(refno: RefnoEnum) -> anyhow::Result<Option<String>> {
        Ok(Self::get_node_meta(refno).await?.map(|m| m.noun))
    }

    /// 节点是否存在（pe 行点查）。
    pub async fn contains(refno: RefnoEnum) -> anyhow::Result<bool> {
        let sql = format!("SELECT VALUE id FROM {};", refno.to_pe_key());
        let rows: Vec<RefnoEnum> = project_primary_db().query_take(&sql, 0).await?;
        Ok(!rows.is_empty())
    }

    // ========================================================================
    // 表级统计（非层级查询）——依赖 D3 `idx_pe_dbnum_noun` 索引
    // ========================================================================
    //
    // noun 枚举/计数/全量 refno 列表不是图操作（没有遍历起点），仍需 dbnum+noun
    // 维度的表级访问。D3 决策：`DEFINE INDEX idx_pe_dbnum_noun ON TABLE pe FIELDS dbnum, noun;`
    // 性能不达标时按计划降级为 per-run 快照统计（D2）。

    /// 幂等定义 pe 表 (dbnum, noun) 二级索引（D3）。
    ///
    /// 注意：存量大表上建索引有一次性构建成本，调用方（CLI/运维脚本）自行择机执行，
    /// 本模块任何查询都不隐式触发。
    pub async fn ensure_pe_dbnum_noun_index() -> anyhow::Result<()> {
        project_primary_db()
            .query("DEFINE INDEX IF NOT EXISTS idx_pe_dbnum_noun ON TABLE pe FIELDS dbnum, noun;")
            .await?
            .check()?;
        Ok(())
    }

    /// 按 noun 枚举 refnos（范围 = 构造时 dbnums；dbnums 为空 = 不限库）。
    pub async fn query_noun_refnos(
        &self,
        noun: &str,
        limit: Option<usize>,
    ) -> anyhow::Result<Vec<RefnoEnum>> {
        let mut out = Vec::new();
        let noun_escaped = noun.replace('\'', "\\'");
        let scopes: Vec<Option<u32>> = if self.dbnums.is_empty() {
            vec![None]
        } else {
            self.dbnums.iter().map(|d| Some(*d)).collect()
        };
        for scope in scopes {
            if let Some(l) = limit {
                if out.len() >= l {
                    break;
                }
            }
            let where_clause = match scope {
                Some(dbnum) => format!("WHERE dbnum = {dbnum} AND noun = '{noun_escaped}'"),
                None => format!("WHERE noun = '{noun_escaped}'"),
            };
            let limit_clause = limit
                .map(|l| format!(" LIMIT {}", (l - out.len()).max(1)))
                .unwrap_or_default();
            let sql = format!("SELECT VALUE id FROM pe {where_clause}{limit_clause};");
            let rows: Vec<RefnoEnum> = project_primary_db().query_take(&sql, 0).await?;
            out.extend(rows);
        }
        if let Some(l) = limit {
            out.truncate(l);
        }
        Ok(out)
    }

    /// 按 noun 统计数量（GROUP BY noun；范围 = 构造时 dbnums）。
    pub async fn count_by_noun(&self) -> anyhow::Result<HashMap<String, usize>> {
        #[derive(Debug, Deserialize, SurrealValue)]
        struct NounCountRow {
            noun: Option<String>,
            count: i64,
        }
        let mut out: HashMap<String, usize> = HashMap::new();
        let scopes: Vec<Option<u32>> = if self.dbnums.is_empty() {
            vec![None]
        } else {
            self.dbnums.iter().map(|d| Some(*d)).collect()
        };
        for scope in scopes {
            let where_clause = scope
                .map(|dbnum| format!("WHERE dbnum = {dbnum} "))
                .unwrap_or_default();
            let sql = format!("SELECT noun, count() AS count FROM pe {where_clause}GROUP BY noun;");
            let rows: Vec<NounCountRow> = project_primary_db().query_take(&sql, 0).await?;
            for row in rows {
                *out.entry(row.noun.unwrap_or_default()).or_default() += row.count.max(0) as usize;
            }
        }
        Ok(out)
    }

    /// noun_hash 分组版本（对齐 TreeIndex 消费面）。
    pub async fn count_by_noun_hash(&self) -> anyhow::Result<HashMap<u32, usize>> {
        Ok(self
            .count_by_noun()
            .await?
            .into_iter()
            .map(|(noun, cnt)| (db1_hash(&noun), cnt))
            .collect())
    }

    /// 可见几何 noun 的全部 refnos（Parquet 导出等消费面）。
    pub async fn query_visible_geo_refnos(&self) -> anyhow::Result<Vec<RefnoEnum>> {
        use aios_core::pdms_types::VISBILE_GEO_NOUNS;
        let mut out = Vec::new();
        for noun in VISBILE_GEO_NOUNS.iter() {
            out.extend(self.query_noun_refnos(noun, None).await?);
        }
        Ok(out)
    }
}

/// noun_hash → noun 名称（分组结果转译用，与 TreeIndex 消费面同源）。
pub fn noun_hash_to_name(hash: u32) -> String {
    db1_dehash(hash)
}
