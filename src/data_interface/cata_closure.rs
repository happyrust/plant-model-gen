//! 元件库（CATA）按需解析 — refno 级引用闭包的基础原语。
//!
//! 设计见 `specs/002-on-demand-cata-closure/`。本模块提供 T001b 地基：
//! - [`parse_db_refnos`]：对单个 db 文件按 refno 子集做**部分解析**（不整库解析），
//!   复用 `PdmsIO::build_index_map()`（refno→历史偏移，`last()` 为最新）+ `parse_element(offset)`。
//! - [`outbound_refs_of`]：从元素属性里抽出向 `RefU64` 引用（闭包"跟边"的原子操作）。
//!
//! 注：`parse_pdms_db::parse::parse_file` 的第二参数是 `Option<PdmsDatabaseInfo>`（数据库信息），
//! 并非 refno 过滤位，故 bulk 解析器无法按 refno 裁剪；本模块用 `PdmsIO` 的随机访问原语补足
//! （T001 已确认可行、低风险）。

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::path::{Path, PathBuf};

use aios_core::{NamedAttrMap, NamedAttrValue, RefU64};
use anyhow::Result;
use pdms_io::PdmsIO;
use serde::{Deserialize, Serialize};

/// B+树索引起始标记 / 无效 ref0（与 `db_index` 保持一致，需跳过）。
const INVALID_REF0_SENTINEL: u32 = 0x8000_0001;

#[inline]
fn is_valid_ref0(ref0: u32) -> bool {
    ref0 != 0 && ref0 != INVALID_REF0_SENTINEL
}

/// 部分解析得到的单个 CATA 元素（闭包扩展所需的最小信息）。
#[derive(Debug, Clone)]
pub struct ParsedCataEle {
    pub refno: RefU64,
    pub owner: RefU64,
    /// noun 的 `db1_hash`。
    pub noun: u32,
    /// 该元素所有出向 `RefU64` 引用（闭包的横向边）。
    pub outbound: Vec<RefU64>,
    /// 该元素的成员/子节点（容器子树的纵向边；来自 `EleData::children`）。
    pub children: Vec<RefU64>,
}

/// 从元素属性表抽取所有出向 `RefU64` 引用（`RefU64Type` / `RefU64Array`）。
///
/// 与 `db_index::extract_outbound_ref0s` 同源，但保留完整 `RefU64`（而非降为 ref0），
/// 供 refno 级闭包跟边使用。
pub fn outbound_refs_of(att: &NamedAttrMap) -> Vec<RefU64> {
    let mut out = Vec::new();
    for value in att.map.values() {
        match value {
            NamedAttrValue::RefU64Type(r) => {
                if is_valid_ref0(r.get_0()) {
                    out.push(*r);
                }
            }
            NamedAttrValue::RefU64Array(arr) => {
                for &refno_enum in arr {
                    let r = refno_enum.refno();
                    if is_valid_ref0(r.get_0()) {
                        out.push(r);
                    }
                }
            }
            _ => {}
        }
    }
    out
}

/// 单个 db 的已打开读取会话：保持文件打开 + 缓存索引，供跨多次解析复用。
///
/// 复用同一 `PdmsIO` 实例可保留其内部 `PageManager` 页缓存，跨 BFS 轮不重读已读页、
/// 不重建索引 —— 逼近 core.dll 的 db1 页缓存常驻效果。
struct DbSession {
    io: PdmsIO,
    /// refno -> 历史偏移（升序，`last()` 为最新版本）。
    index_map: HashMap<RefU64, Vec<u64>>,
}

/// 打开一个 db 读取会话（`open` + 一次性建索引）。
fn open_db_session(project: &str, path: &Path) -> Result<DbSession> {
    let mut io = PdmsIO::new(project, path, false);
    io.open()?;
    let index_map = io.build_index_map()?;
    Ok(DbSession { io, index_map })
}

/// 用已打开会话解析一批 refno（复用页缓存，不重开文件 / 不重建索引）。
///
/// `attmap_sink`：可选保留完整属性表（T007 惰性兜底落库需要；闭包发现 pass 传 `None` 省内存）。
async fn parse_refnos_with_session(
    io: &mut PdmsIO,
    index_map: &HashMap<RefU64, Vec<u64>>,
    refnos: &[RefU64],
    mut attmap_sink: Option<&mut HashMap<RefU64, (NamedAttrMap, Vec<RefU64>)>>,
) -> Result<HashMap<RefU64, ParsedCataEle>> {
    let mut out = HashMap::with_capacity(refnos.len());
    for &refno in refnos {
        let Some(offsets) = index_map.get(&refno) else {
            continue; // 本库不含此 refno
        };
        let Some(&latest_offset) = offsets.last() else {
            continue;
        };
        match io.parse_element(latest_offset).await {
            Ok(ele) => {
                let outbound = outbound_refs_of(ele.att_map());
                let children: Vec<RefU64> = ele
                    .children
                    .0
                    .iter()
                    .copied()
                    .filter(|r| is_valid_ref0(r.get_0()))
                    .collect();
                if let Some(sink) = attmap_sink.as_deref_mut() {
                    sink.insert(refno, (ele.att_map().clone(), children.clone()));
                }
                out.insert(
                    refno,
                    ParsedCataEle {
                        refno: ele.refno,
                        owner: ele.owner,
                        noun: ele.noun,
                        outbound,
                        children,
                    },
                );
            }
            Err(_) => {
                // 解析失败：跳过，由调用方按 cache-miss 处理。
            }
        }
    }
    Ok(out)
}

/// 对单个 db 文件按 refno 子集做部分解析（一次性 `open` + 建索引；适合单库一次性调用）。
///
/// 多轮 / 多批量复用同一库时优先用 [`CataClosureResolver`]（内部按 dbnum 缓存会话，复用页缓存）。
///
/// # 参数
/// - `project`：工程名（`PdmsIO` 语义需要）。
/// - `path`：db 文件路径。
/// - `refnos`：要解析的 refno 子集。
///
/// # 返回
/// `refno -> ParsedCataEle` 映射（仅包含成功解析的 refno）。
pub async fn parse_db_refnos(
    project: &str,
    path: &Path,
    refnos: &[RefU64],
) -> Result<HashMap<RefU64, ParsedCataEle>> {
    if refnos.is_empty() {
        return Ok(HashMap::new());
    }
    let mut session = open_db_session(project, path)?;
    parse_refnos_with_session(&mut session.io, &session.index_map, refnos, None).await
}

// ─────────────────────────────────────────────────────────────────────────────
// refno 级引用闭包引擎（spec 002, Q2/Q3/Q4/Q6）
// ─────────────────────────────────────────────────────────────────────────────

/// 把 refno 定位到 db（dbnum / db_type / 文件），供闭包跨库扩展。
///
/// 抽象成 trait 是为了让闭包引擎**不强依赖** `sqlite-index` 的 `DbIndexStore`，便于单测与解耦。
pub trait CataDbLocator {
    /// ref0（`RefU64::get_0()`）-> 所属 dbnum。
    fn dbnum_of_ref0(&self, ref0: u32) -> Option<u32>;
    /// dbnum -> db_type（如 "CATA" / "DESI"）。
    fn db_type_of(&self, dbnum: u32) -> Option<String>;
    /// dbnum -> (project, db 文件路径)。
    fn file_of(&self, dbnum: u32) -> Option<(String, PathBuf)>;
}

/// 闭包行为配置。
#[derive(Debug, Clone)]
pub struct CataClosureConfig {
    /// 是否纳入 owner 祖先链（Q4，默认开）。
    pub include_owner_chain: bool,
    /// 是否纳入容器子树（成员，Q4/Q5，默认开）。
    pub follow_children: bool,
    /// 收口的 db_type 集合（大小写不敏感，默认 {"CATA"}）。
    /// 若规格库为单独类型（如 "PADD"），可在此追加。
    pub cata_db_types: HashSet<String>,
    /// 防御性轮数上限。
    pub max_rounds: usize,
}

impl Default for CataClosureConfig {
    fn default() -> Self {
        let mut cata_db_types = HashSet::new();
        cata_db_types.insert("CATA".to_string());
        Self {
            include_owner_chain: true,
            follow_children: true,
            cata_db_types,
            max_rounds: 64,
        }
    }
}

/// 闭包结果：每个 CATA dbnum 需解析的 refno 集合 + 统计。
#[derive(Debug, Clone, Default)]
pub struct CataClosureManifest {
    /// dbnum -> 该库内闭包覆盖到（且成功解析）的 refno 集合。
    pub by_dbnum: BTreeMap<u32, BTreeSet<RefU64>>,
    /// 种子数（含未收口前）。
    pub seed_count: usize,
    /// visited 总数（尝试解析过的 CATA refno）。
    pub visited_count: usize,
    /// BFS 轮数。
    pub rounds: usize,
    /// 缺失计数（无 dbnum 映射 / 库内未找到 / 解析失败）。
    pub missing: usize,
}

/// refno 级 CATA 引用闭包引擎（BFS）。
///
/// 用法：`new` → `seed`(DESI 出向引用) → `resolve()`。
pub struct CataClosureResolver<'a, L: CataDbLocator> {
    locator: &'a L,
    cfg: CataClosureConfig,
    visited: HashSet<RefU64>,
    frontier: Vec<RefU64>,
    /// 每个 dbnum 的打开会话缓存（复用页缓存，逼近 core.dll db1 页缓存）。
    sessions: HashMap<u32, DbSession>,
    /// 是否保留完整属性表（T007 惰性兜底落库用；闭包发现 pass 默认关省内存）。
    retain_attmaps: bool,
    /// `retain_attmaps` 开启时收集：refno -> (完整属性表, children)。
    attmaps: HashMap<RefU64, (NamedAttrMap, Vec<RefU64>)>,
}

impl<'a, L: CataDbLocator> CataClosureResolver<'a, L> {
    pub fn new(locator: &'a L, cfg: CataClosureConfig) -> Self {
        Self {
            locator,
            cfg,
            visited: HashSet::new(),
            frontier: Vec::new(),
            sessions: HashMap::new(),
            retain_attmaps: false,
            attmaps: HashMap::new(),
        }
    }

    /// 开启属性表保留（小闭包惰性兜底场景；大闭包慎用，内存随 visited 线性增长）。
    pub fn with_retain_attmaps(mut self, retain: bool) -> Self {
        self.retain_attmaps = retain;
        self
    }

    /// 取走保留的属性表（`retain_attmaps` 开启时在 `resolve()` 后调用）。
    pub fn take_attmaps(&mut self) -> HashMap<RefU64, (NamedAttrMap, Vec<RefU64>)> {
        std::mem::take(&mut self.attmaps)
    }

    /// 播种：把从 DESI 收集到的出向引用（`outbound_refs_of` 的结果）作为闭包起点。
    /// 非 CATA 的种子会在 `resolve` 的 db_type 收口阶段被丢弃。
    pub fn seed(&mut self, refs: impl IntoIterator<Item = RefU64>) {
        self.frontier.extend(refs);
    }

    /// 跑完整 BFS 闭包：每轮按 dbnum 聚合 frontier → `parse_db_refnos` 部分解析
    /// → 跟随 outbound（横向）+ owner（纵向）+ children（容器子树）→ visited 去重，
    /// 直至 frontier 空或达到 `max_rounds`。
    pub async fn resolve(&mut self) -> Result<CataClosureManifest> {
        let include_owner = self.cfg.include_owner_chain;
        let follow_children = self.cfg.follow_children;
        let max_rounds = self.cfg.max_rounds;
        let cata_types: HashSet<String> = self
            .cfg
            .cata_db_types
            .iter()
            .map(|t| t.to_uppercase())
            .collect();

        let seed_count = self.frontier.len();
        let mut by_dbnum: BTreeMap<u32, BTreeSet<RefU64>> = BTreeMap::new();
        let mut missing = 0usize;
        let mut rounds = 0usize;

        while !self.frontier.is_empty() && rounds < max_rounds {
            rounds += 1;
            let current = std::mem::take(&mut self.frontier);

            // 按 dbnum 聚合本轮 frontier（db_type 收口到 CATA）。
            let mut by_db: HashMap<u32, Vec<RefU64>> = HashMap::new();
            for r in current {
                if self.visited.contains(&r) {
                    continue;
                }
                let ref0 = r.get_0();
                if !is_valid_ref0(ref0) {
                    continue;
                }
                let Some(dbnum) = self.locator.dbnum_of_ref0(ref0) else {
                    missing += 1;
                    continue;
                };
                let is_cata = self
                    .locator
                    .db_type_of(dbnum)
                    .map(|t| cata_types.contains(&t.to_uppercase()))
                    .unwrap_or(false);
                if !is_cata {
                    continue; // 非 CATA（回指 DESI/DICT 等）不下探
                }
                by_db.entry(dbnum).or_default().push(r);
            }

            // 每库部分解析（会话缓存：每库只 open + 建索引一次，跨轮复用页缓存）。
            for (dbnum, refs) in by_db {
                let to_parse: Vec<RefU64> = refs
                    .into_iter()
                    .filter(|r| !self.visited.contains(r))
                    .collect();
                if to_parse.is_empty() {
                    continue;
                }

                // 确保该库会话已打开（一次性 open + 建索引；后续轮复用）。
                if !self.sessions.contains_key(&dbnum) {
                    let Some((project, path)) = self.locator.file_of(dbnum) else {
                        missing += to_parse.len();
                        for r in &to_parse {
                            self.visited.insert(*r); // 无文件信息也标记，避免无限重试
                        }
                        continue;
                    };
                    match open_db_session(&project, &path) {
                        Ok(sess) => {
                            self.sessions.insert(dbnum, sess);
                        }
                        Err(_) => {
                            missing += to_parse.len();
                            for r in &to_parse {
                                self.visited.insert(*r);
                            }
                            continue;
                        }
                    }
                }
                let session = self
                    .sessions
                    .get_mut(&dbnum)
                    .expect("session just ensured");
                let attmap_sink = if self.retain_attmaps {
                    Some(&mut self.attmaps)
                } else {
                    None
                };
                let parsed = parse_refnos_with_session(
                    &mut session.io,
                    &session.index_map,
                    &to_parse,
                    attmap_sink,
                )
                .await?;

                let mut next: Vec<RefU64> = Vec::new();
                for r in &to_parse {
                    if !self.visited.insert(*r) {
                        continue;
                    }
                    match parsed.get(r) {
                        Some(ele) => {
                            by_dbnum.entry(dbnum).or_default().insert(*r);
                            next.extend(ele.outbound.iter().copied());
                            if include_owner && is_valid_ref0(ele.owner.get_0()) {
                                next.push(ele.owner);
                            }
                            if follow_children {
                                next.extend(ele.children.iter().copied());
                            }
                        }
                        None => {
                            missing += 1; // 请求了但本库未找到 / 解析失败
                        }
                    }
                }
                for n in next {
                    if !self.visited.contains(&n) {
                        self.frontier.push(n);
                    }
                }
            }
        }

        Ok(CataClosureManifest {
            by_dbnum,
            seed_count,
            visited_count: self.visited.len(),
            rounds,
            missing,
        })
    }
}

/// `DbIndexStore`（`sqlite-index`）作为闭包的 db 定位器。
#[cfg(feature = "sqlite-index")]
impl CataDbLocator for crate::data_interface::db_index::DbIndexStore {
    fn dbnum_of_ref0(&self, ref0: u32) -> Option<u32> {
        self.dbnum_by_ref0(ref0)
    }

    fn db_type_of(&self, dbnum: u32) -> Option<String> {
        self.file_by_dbnum(dbnum).map(|r| r.db_type)
    }

    fn file_of(&self, dbnum: u32) -> Option<(String, PathBuf)> {
        self.file_by_dbnum(dbnum)
            .map(|r| (r.project, PathBuf::from(r.file_path)))
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// T006: DESI 播种 + 端到端入口
// ─────────────────────────────────────────────────────────────────────────────

/// 从一份 DESI 解析产物收集闭包种子：所有 DESI 元素的出向 `RefU64` 引用（去重）。
///
/// db_type 收口（只保留 CATA）由 [`CataClosureResolver::resolve`] 负责；此处不过滤，避免漏种。
pub fn seed_refs_from_design_data(data: &parse_pdms_db::parse::PdmsDbData) -> Vec<RefU64> {
    let mut set: HashSet<RefU64> = HashSet::new();
    for entry in data.total_attr_map.iter() {
        for r in outbound_refs_of(entry.value()) {
            set.insert(r);
        }
    }
    set.into_iter().collect()
}

/// 端到端：解析单个 DESI 库文件 → 以其出向引用为种子 → 跑 refno 级 CATA 闭包。
///
/// - `index`：db 定位器（`ref0→dbnum` / `db_type` / 文件），通常来自站点级 `db_index.sqlite`。
/// - `project` / `desi_path`：DESI 库的工程名与文件路径。
///
/// DESI 走 bulk `parse_file`（Phase1 必须全解析）；CATA 走 `parse_db_refnos` 部分解析（Phase2）。
/// 多 DESI 库由调用方循环本函数并合并各 `CataClosureManifest`。
#[cfg(feature = "sqlite-index")]
pub async fn resolve_cata_closure_from_design_file(
    index: &crate::data_interface::db_index::DbIndexStore,
    project: &str,
    desi_path: &Path,
    cfg: CataClosureConfig,
) -> Result<CataClosureManifest> {
    let file_name = desi_path
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("")
        .to_string();
    let data =
        parse_pdms_db::parse::parse_file(&desi_path.to_path_buf(), &None, &file_name, project)
            .await?;
    let seeds = seed_refs_from_design_data(&data);

    let mut resolver = CataClosureResolver::new(index, cfg);
    resolver.seed(seeds);
    resolver.resolve().await
}

// ─────────────────────────────────────────────────────────────────────────────
// T006b: sync 流水线接入（manifest 驱动的 CATA 部分解析 + 整库回退开关）
// ─────────────────────────────────────────────────────────────────────────────

/// sync 流水线的 CATA 解析模式（由 env `AIOS_CATA_CLOSURE_MODE` 控制）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CataClosureSyncMode {
    /// 整库解析（默认，与历史行为一致）。
    Off,
    /// manifest 部分解析：CATA 库只解析 `cata_closure.json` 覆盖的 refno；
    /// manifest 缺失 / 未覆盖该 dbnum 时整库回退（仅告警，不失败）。
    Manifest,
}

/// 读取 env `AIOS_CATA_CLOSURE_MODE`：`manifest`（不区分大小写）启用部分解析，
/// 其余取值 / 未设置一律视为 [`CataClosureSyncMode::Off`]（保证默认零行为变化）。
pub fn cata_closure_sync_mode() -> CataClosureSyncMode {
    match std::env::var("AIOS_CATA_CLOSURE_MODE") {
        Ok(v) if v.trim().eq_ignore_ascii_case("manifest") => CataClosureSyncMode::Manifest,
        _ => CataClosureSyncMode::Off,
    }
}

/// manifest 默认落盘路径：`output/<project>/scene_tree/cata_closure.json`
/// （与 `db_index.sqlite` / `db_meta_info.json` 同目录，路径口径同源）。
pub fn default_manifest_path(project_name: &str) -> PathBuf {
    crate::versioned_db::db_meta_info::get_project_tree_dir(project_name)
        .join("cata_closure.json")
}

/// 判断 db_type 是否属于"元件库"类型（与 [`CataClosureConfig::default`] 的收口集合一致）。
pub fn is_cata_db_type(db_type: &str) -> bool {
    CataClosureConfig::default()
        .cata_db_types
        .contains(&db_type.to_uppercase())
}

/// sync 解析用过滤器：dbnum -> 允许解析的 refno 集合。
pub type CataClosureFilter = HashMap<u32, HashSet<RefU64>>;

/// 按当前模式为 sync 加载 CATA 闭包过滤器。
///
/// 返回 `None` 即整库解析（回退路径），出现在：
/// - 模式为 Off（默认）；
/// - 本轮 `db_types` 不含 CATA 类型（无需加载）;
/// - manifest 文件缺失或解析失败（告警后回退）。
pub fn load_sync_filter(project_name: &str, db_types: &[String]) -> Option<CataClosureFilter> {
    if cata_closure_sync_mode() != CataClosureSyncMode::Manifest {
        return None;
    }
    if !db_types.iter().any(|t| is_cata_db_type(t)) {
        return None;
    }
    let path = default_manifest_path(project_name);
    if !path.exists() {
        log::warn!(
            "[cata_closure] AIOS_CATA_CLOSURE_MODE=manifest 但 manifest 不存在: {}（CATA 整库回退）",
            path.display()
        );
        return None;
    }
    match CataClosureManifest::load_json(&path) {
        Ok(manifest) => {
            let filter: CataClosureFilter = manifest
                .by_dbnum
                .iter()
                .map(|(dbnum, refs)| (*dbnum, refs.iter().copied().collect::<HashSet<RefU64>>()))
                .collect();
            log::info!(
                "[cata_closure] 已加载 manifest: {}（{} 个 CATA 库, visited={}, missing={}）",
                path.display(),
                filter.len(),
                manifest.visited_count,
                manifest.missing
            );
            Some(filter)
        }
        Err(e) => {
            log::warn!(
                "[cata_closure] manifest 解析失败: {}: {}（CATA 整库回退）",
                path.display(),
                e
            );
            None
        }
    }
}

/// 对单个 db 文件的 refno 全集应用闭包过滤（sync per-file 调用点）。
///
/// 仅当 `db_type` 属于 CATA 类型、filter 已加载且覆盖该 dbnum 时裁剪；
/// 其余情况原样返回（= 整库解析回退）。
pub fn apply_sync_filter(
    filter: Option<&CataClosureFilter>,
    db_type: &str,
    dbnum: u32,
    all_refnos: Vec<RefU64>,
) -> Vec<RefU64> {
    let Some(filter) = filter else {
        return all_refnos;
    };
    if !is_cata_db_type(db_type) {
        return all_refnos;
    }
    match filter.get(&dbnum) {
        Some(allow) => {
            let before = all_refnos.len();
            let filtered: Vec<RefU64> = all_refnos
                .into_iter()
                .filter(|r| allow.contains(r))
                .collect();
            log::info!(
                "[cata_closure] dbnum={} 按 manifest 部分解析: {}/{} refnos",
                dbnum,
                filtered.len(),
                before
            );
            filtered
        }
        None => {
            log::warn!(
                "[cata_closure] dbnum={} 不在 manifest 覆盖内，整库回退解析（{} refnos）",
                dbnum,
                all_refnos.len()
            );
            all_refnos
        }
    }
}

/// 项目级前置闭包 pass：扫描 `roots` 下全部 DESI 库，逐库
/// [`resolve_cata_closure_from_design_file`] 后合并，原子写入 `out_path`。
///
/// 设计（spec 002 Q8）：闭包是**独立前置 pass**，sync 只消费其产物（manifest 文件），
/// 两者解耦；本函数即该 pass 的编排入口（CLI / 部署脚本调用）。
#[cfg(feature = "sqlite-index")]
pub async fn run_cata_closure_pass_for_roots(
    index: &crate::data_interface::db_index::DbIndexStore,
    roots: &[(String, PathBuf)],
    cfg: CataClosureConfig,
    out_path: &Path,
) -> Result<CataClosureManifest> {
    use parse_pdms_db::parse::parse_db_basic_info;

    let mut total = CataClosureManifest::default();
    let mut seed_sum = 0usize;
    let mut missing_sum = 0usize;
    let mut max_rounds = 0usize;
    let mut seen_dbnums: HashSet<u32> = HashSet::new();

    for (project, root) in roots {
        if !root.exists() {
            continue;
        }
        for entry in walkdir::WalkDir::new(root)
            .max_depth(8)
            .into_iter()
            .filter_map(|e| e.ok())
        {
            if !entry.file_type().is_file() {
                continue;
            }
            let path = entry.path();
            let info = parse_db_basic_info(path.to_path_buf());
            if info.dbnum == 0
                || !info.db_type.eq_ignore_ascii_case("DESI")
                || !seen_dbnums.insert(info.dbnum)
            {
                continue;
            }
            match resolve_cata_closure_from_design_file(index, project, path, cfg.clone()).await {
                Ok(manifest) => {
                    seed_sum += manifest.seed_count;
                    missing_sum += manifest.missing;
                    max_rounds = max_rounds.max(manifest.rounds);
                    total.merge_from(&manifest);
                }
                Err(e) => {
                    log::warn!(
                        "[cata_closure] DESI 闭包失败 {}（跳过该库）: {}",
                        path.display(),
                        e
                    );
                }
            }
        }
    }

    total.seed_count = seed_sum;
    total.missing = missing_sum;
    total.rounds = max_rounds;
    total.save_json(out_path)?;
    log::info!(
        "[cata_closure] 前置闭包 pass 完成: {} 个 CATA 库 / visited={} / missing={} → {}",
        total.by_dbnum.len(),
        total.visited_count,
        total.missing,
        out_path.display()
    );
    Ok(total)
}

// ─────────────────────────────────────────────────────────────────────────────
// T007: 运行期惰性兜底（命中未解析的 CATA refno → 即时小闭包 → 落库 → manifest 增量）
// ─────────────────────────────────────────────────────────────────────────────

/// 内存版 db 定位器：从 `db_index.sqlite` 一次性全量加载。
///
/// rusqlite `Connection` 非 `Sync`，直接把 `DbIndexStore` 当 locator 会让
/// `resolve()`（跨 await 持有 `&L`）的 Future 失去 `Send`；索引表量级很小
/// （每库个位数 ref0），全量载入内存换 `Send` 是稳赚的取舍。
pub struct InMemoryDbLocator {
    ref0_to_dbnum: HashMap<u32, u32>,
    files: HashMap<u32, InMemoryDbFile>,
}

struct InMemoryDbFile {
    db_type: String,
    project: String,
    path: PathBuf,
}

impl InMemoryDbLocator {
    /// 从 `db_index.sqlite` 全量加载（同步 IO，建议放 `spawn_blocking`）。
    #[cfg(feature = "sqlite-index")]
    pub fn load_from_index(path: &Path) -> Result<Self> {
        let store = crate::data_interface::db_index::DbIndexStore::open(path)?;
        let ref0_to_dbnum: HashMap<u32, u32> = store.all_ref0_owners().into_iter().collect();
        let files: HashMap<u32, InMemoryDbFile> = store
            .all_db_files()
            .into_iter()
            .map(|r| {
                (
                    r.dbnum,
                    InMemoryDbFile {
                        db_type: r.db_type,
                        project: r.project,
                        path: PathBuf::from(r.file_path),
                    },
                )
            })
            .collect();
        Ok(Self {
            ref0_to_dbnum,
            files,
        })
    }

    /// 某 dbnum 所属工程名（manifest 增量按工程分目录落盘用）。
    pub fn project_of(&self, dbnum: u32) -> Option<String> {
        self.files.get(&dbnum).map(|f| f.project.clone())
    }
}

impl CataDbLocator for InMemoryDbLocator {
    fn dbnum_of_ref0(&self, ref0: u32) -> Option<u32> {
        self.ref0_to_dbnum.get(&ref0).copied()
    }

    fn db_type_of(&self, dbnum: u32) -> Option<String> {
        self.files.get(&dbnum).map(|f| f.db_type.clone())
    }

    fn file_of(&self, dbnum: u32) -> Option<(String, PathBuf)> {
        self.files
            .get(&dbnum)
            .map(|f| (f.project.clone(), f.path.clone()))
    }
}

/// 惰性兜底结果统计。
#[derive(Debug, Default, Clone)]
pub struct LazyFallbackOutcome {
    /// 成功解析并落库的 CATA refno 数（含闭包扩展出的关联元素）。
    pub parsed: usize,
    /// 闭包过程中无法定位/解析的引用数。
    pub missing: usize,
}

/// 惰性兜底全局互斥：并发 miss 串行化，避免重复建索引/重复解析同一批元素
/// （落库用 INSERT IGNORE，重复执行幂等）。
#[cfg(all(feature = "sqlite-index", feature = "surreal-save"))]
static LAZY_CATA_FALLBACK_LOCK: once_cell::sync::Lazy<tokio::sync::Mutex<()>> =
    once_cell::sync::Lazy::new(|| tokio::sync::Mutex::new(()));

/// T007 运行期惰性兜底：对未解析的 CATA refno 跑**小闭包**并即时落库。
///
/// 流程：seeds → [`InMemoryDbLocator`]（db_index.sqlite）→ [`CataClosureResolver`]
/// （保留属性表）→ `INSERT IGNORE` 写 `pe` + `ATT_{noun}` + `ATT_UDA` → 闭包结果
/// merge 进各工程 `cata_closure.json`（增量 delta，Q8）。
///
/// 调用方约定：命中"pe 缺失"再调用（如 `get_named_attmap` 失败路径），成功后重试原查询；
/// cache-miss 统计由调用方记录（`cache_miss_report` 在 gen_model 层）。
#[cfg(all(feature = "sqlite-index", feature = "surreal-save"))]
pub async fn ensure_cata_refnos_parsed(seeds: &[RefU64]) -> Result<LazyFallbackOutcome> {
    use aios_core::project_primary_db;

    if seeds.is_empty() {
        return Ok(LazyFallbackOutcome::default());
    }
    let _guard = LAZY_CATA_FALLBACK_LOCK.lock().await;

    // 1. 定位器：db_index.sqlite 全量载入内存（保持 Future Send）。
    let site_project = aios_core::get_db_option().project_name.clone();
    let index_path = crate::versioned_db::db_meta_info::get_project_tree_dir(&site_project)
        .join(crate::data_interface::db_index::DB_INDEX_FILE_NAME);
    if !index_path.exists() {
        anyhow::bail!(
            "[cata_closure] 惰性兜底需要 db_index.sqlite（未找到: {}），请先执行预扫描",
            index_path.display()
        );
    }
    let locator = {
        let path = index_path.clone();
        tokio::task::spawn_blocking(move || InMemoryDbLocator::load_from_index(&path)).await??
    };

    // 2. 小闭包（保留属性表与 children）。
    let mut resolver = CataClosureResolver::new(&locator, CataClosureConfig::default())
        .with_retain_attmaps(true);
    resolver.seed(seeds.iter().copied());
    let delta = resolver.resolve().await?;
    let retained = resolver.take_attmaps();

    // 3. 落库：pe（带 children/refno 链接）+ ATT_{noun} + ATT_UDA，全部 INSERT IGNORE 幂等。
    const INSERT_CHUNK: usize = 500;
    let mut parsed = 0usize;
    for (dbnum, refs) in &delta.by_dbnum {
        let mut pe_jsons: Vec<String> = Vec::new();
        let mut att_by_table: HashMap<String, Vec<String>> = HashMap::new();
        let mut uda_jsons: Vec<String> = Vec::new();

        for refno in refs {
            let Some((att, children)) = retained.get(refno) else {
                continue;
            };
            // pe 行（与 versioned_db::pe::save_pes 同构：gen_sur_json + children 注入）。
            let pe_data = att.pe(*dbnum as i32);
            let mut json = pe_data.gen_sur_json(Some(refno.to_pe_key()));
            let children_links = if children.is_empty() {
                String::new()
            } else {
                children
                    .iter()
                    .map(|c| c.to_pe_key())
                    .collect::<Vec<_>>()
                    .join(", ")
            };
            if json.ends_with('}') {
                json.pop();
                let needs_comma = json.ends_with('}') || json.contains(':');
                let sep = if needs_comma { ", " } else { "" };
                json.push_str(&format!("{}children: [{}]}}", sep, children_links));
            }
            pe_jsons.push(json);

            // ATT_{noun} / ATT_UDA 行。
            let table = att.get_type_str().to_string();
            if !table.is_empty() {
                if let Some(att_json) = att.gen_sur_json() {
                    att_by_table.entry(table).or_default().push(att_json);
                }
                if let Some(uda_json) = att.gen_sur_json_uda(&[]) {
                    uda_jsons.push(aios_core::helper::normalize_sql_string(&uda_json));
                }
            }
            parsed += 1;
        }

        for chunk in pe_jsons.chunks(INSERT_CHUNK) {
            let sql = format!("INSERT IGNORE INTO pe [{}]", chunk.join(","));
            project_primary_db().query(&sql).await?;
        }
        for (table, jsons) in att_by_table {
            for chunk in jsons.chunks(INSERT_CHUNK) {
                let sql = format!("INSERT IGNORE INTO {} [{}]", table, chunk.join(","));
                project_primary_db().query(&sql).await?;
            }
        }
        for chunk in uda_jsons.chunks(INSERT_CHUNK) {
            let sql = format!("INSERT IGNORE INTO ATT_UDA [{}]", chunk.join(","));
            project_primary_db().query(&sql).await?;
        }
    }

    // 4. manifest 增量 merge（按 dbnum 所属工程分别落盘，与 sync 读取口径一致）。
    let mut delta_by_project: HashMap<String, CataClosureManifest> = HashMap::new();
    for (dbnum, refs) in &delta.by_dbnum {
        let project = locator
            .project_of(*dbnum)
            .unwrap_or_else(|| site_project.clone());
        delta_by_project
            .entry(project)
            .or_default()
            .by_dbnum
            .insert(*dbnum, refs.clone());
    }
    for (project, project_delta) in delta_by_project {
        let manifest_path = default_manifest_path(&project);
        let mut base = if manifest_path.exists() {
            CataClosureManifest::load_json(&manifest_path).unwrap_or_default()
        } else {
            CataClosureManifest::default()
        };
        base.merge_from(&project_delta);
        base.save_json(&manifest_path)?;
    }

    log::info!(
        "[cata_closure] 惰性兜底完成: seeds={} parsed={} missing={} rounds={}",
        seeds.len(),
        parsed,
        delta.missing,
        delta.rounds
    );
    Ok(LazyFallbackOutcome {
        parsed,
        missing: delta.missing,
    })
}

// ─────────────────────────────────────────────────────────────────────────────
// T005: manifest 持久化（原子写：tmp + rename）
// ─────────────────────────────────────────────────────────────────────────────

/// 持久化 DTO：`RefU64` 落为 `u64`，避免依赖其 serde 实现。
#[derive(Debug, Serialize, Deserialize)]
struct CataClosureManifestDto {
    by_dbnum: BTreeMap<u32, Vec<u64>>,
    seed_count: usize,
    visited_count: usize,
    rounds: usize,
    missing: usize,
}

impl CataClosureManifest {
    /// 原子写入 JSON（`tmp` 落盘后 `rename`）。建议路径：
    /// `output/<project>/scene_tree/cata_closure.json`。
    pub fn save_json(&self, path: &Path) -> Result<()> {
        let dto = CataClosureManifestDto {
            by_dbnum: self
                .by_dbnum
                .iter()
                .map(|(dbnum, refs)| (*dbnum, refs.iter().map(|r| r.0).collect()))
                .collect(),
            seed_count: self.seed_count,
            visited_count: self.visited_count,
            rounds: self.rounds,
            missing: self.missing,
        };
        let json = serde_json::to_string_pretty(&dto)?;
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let mut tmp_os = path.as_os_str().to_owned();
        tmp_os.push(".tmp");
        let tmp = PathBuf::from(tmp_os);
        std::fs::write(&tmp, json)?;
        std::fs::rename(&tmp, path)?;
        Ok(())
    }

    /// 从 JSON 载入 manifest。
    pub fn load_json(path: &Path) -> Result<Self> {
        let s = std::fs::read_to_string(path)?;
        let dto: CataClosureManifestDto = serde_json::from_str(&s)?;
        Ok(Self {
            by_dbnum: dto
                .by_dbnum
                .into_iter()
                .map(|(dbnum, refs)| (dbnum, refs.into_iter().map(RefU64).collect()))
                .collect(),
            seed_count: dto.seed_count,
            visited_count: dto.visited_count,
            rounds: dto.rounds,
            missing: dto.missing,
        })
    }

    /// 把另一个 manifest 的覆盖范围并入自身（增量 delta 合并，Q8 增量）。
    pub fn merge_from(&mut self, other: &CataClosureManifest) {
        for (dbnum, refs) in &other.by_dbnum {
            self.by_dbnum
                .entry(*dbnum)
                .or_default()
                .extend(refs.iter().copied());
        }
        self.visited_count = self.by_dbnum.values().map(|s| s.len()).sum();
    }
}
