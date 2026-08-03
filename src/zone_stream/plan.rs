//! ZONE 规划、依赖 epoch 与范围封存（ADR-0016 D5 / D6，spec 030 Phase 6）。
//!
//! 三个概念的边界，混淆任何一对都会破坏正确性：
//!
//! - [`ZonePlan`]：一次运行**打算**做哪些 dbnum、每个 dbnum 内 ZONE 的执行顺序。
//!   它的哈希进 `initialization_runs`，是 Resume 判等的三要素之一。
//! - [`DepsEpoch`]：某个 dbnum 的共享依赖并集**已经装载完毕并冻结**的证明。
//!   ZONE 流水开始后它不得变化，变了就是不可恢复错误。
//! - [`ZoneScopeSeal`]：单个 ZONE **解析侧**的范围完整性证明，是进入生成的前置条件。
//!   它只说「这个 ZONE 该有的都在」，**不**代表 dbnum 完整，因此绝不触碰
//!   dbnum 级 `pe_owner` Ready（ADR-0016 D6）。

use std::collections::BTreeMap;

use anyhow::{bail, Result};
use sha2::{Digest, Sha256};

/// 稳定哈希：把一串已经排好序的字段喂进 SHA-256，取前 32 个 hex 字符。
///
/// 只用于「同一份输入必须得到同一个值」的判等，不用于安全场景，因此截断可接受。
fn stable_hash(parts: &[String]) -> String {
    let mut hasher = Sha256::new();
    for part in parts {
        // 长度前缀避免 ["ab","c"] 与 ["a","bc"] 撞在一起。
        hasher.update((part.len() as u64).to_le_bytes());
        hasher.update(part.as_bytes());
    }
    format!("{:x}", hasher.finalize())[..32].to_string()
}

/// 一个 ZONE 在计划中的条目。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ZoneEntry {
    /// ZONE 的 refno 字面量（形如 `=24381/144870`）。
    pub refno: String,
    /// ZONE 在本 dbnum 内的执行序号，从 0 开始。
    pub order: u32,
}

/// 一次运行的完整规划。
///
/// dbnum 按升序执行（沿用 Legacy 的既有顺序，便于与 Legacy 对照）；
/// ZONE 在 dbnum 内按稳定序执行 —— 稳定是硬要求，因为 Resume 要靠
/// [`Self::plan_hash`] 判断「还是同一个计划」。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ZonePlan {
    /// dbnum -> 该库内的 ZONE 序列。`BTreeMap` 保证 dbnum 升序遍历。
    pub zones_by_dbnum: BTreeMap<u32, Vec<ZoneEntry>>,
}

impl ZonePlan {
    /// 从「dbnum -> ZONE refno 列表」构造，并按给定顺序固定 `order`。
    ///
    /// 调用方负责先把 refno 排成稳定序（层级序或 refno 升序，见 spec 030 Open Question 1）；
    /// 这里只做校验：同一 dbnum 内不允许重复 refno，空 dbnum 不允许出现。
    pub fn new(zones_by_dbnum: BTreeMap<u32, Vec<String>>) -> Result<Self> {
        let mut built: BTreeMap<u32, Vec<ZoneEntry>> = BTreeMap::new();
        for (dbnum, refnos) in zones_by_dbnum {
            if refnos.is_empty() {
                bail!("dbnum {dbnum} 的 ZONE 列表为空：不应进入 ZONE 规划");
            }
            let mut seen = std::collections::HashSet::new();
            let mut entries = Vec::with_capacity(refnos.len());
            for (order, refno) in refnos.into_iter().enumerate() {
                if !seen.insert(refno.clone()) {
                    bail!("dbnum {dbnum} 的 ZONE 规划里出现重复 refno `{refno}`");
                }
                entries.push(ZoneEntry {
                    refno,
                    order: order as u32,
                });
            }
            built.insert(dbnum, entries);
        }
        if built.is_empty() {
            bail!("ZONE 规划为空：没有任何目标 dbnum");
        }
        Ok(Self {
            zones_by_dbnum: built,
        })
    }

    /// 目标 dbnum，升序。
    pub fn target_dbnums(&self) -> Vec<u32> {
        self.zones_by_dbnum.keys().copied().collect()
    }

    pub fn zone_count(&self) -> usize {
        self.zones_by_dbnum.values().map(Vec::len).sum()
    }

    /// 计划哈希，进 `initialization_runs.zone_plan_hash`，参与 Resume 判等。
    ///
    /// 覆盖 dbnum 集合、每个 dbnum 内的 ZONE 序列**及其顺序**：顺序变了就是另一个计划，
    /// 因为 Verified ZONE 的跳过是按 refno 认的，顺序变化会让「跳过已完成」的语义漂移。
    pub fn plan_hash(&self) -> String {
        let mut parts = Vec::new();
        for (dbnum, entries) in &self.zones_by_dbnum {
            parts.push(format!("db:{dbnum}"));
            for entry in entries {
                parts.push(format!("{}:{}", entry.order, entry.refno));
            }
        }
        stable_hash(&parts)
    }
}

/// 一个 dbnum 的共享依赖并集装载完成后的不可变证明（ADR-0016 D5）。
///
/// `epoch` 在一次运行内单调递增（每个 dbnum 一个），`hash` 描述内容。
/// ZONE 流水期间对 `deps` 的任何改动都属于契约破坏 —— 用
/// [`Self::assert_unchanged`] 在关键边界复核。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DepsEpoch {
    pub dbnum: u32,
    pub epoch: u64,
    pub hash: String,
    /// 依赖并集覆盖到的库，按 dbnum 升序；含 SYSTEM 类（DICT/SYST/GLB/GLOB）与 CATA。
    pub dependency_dbnums: Vec<u32>,
    /// 各依赖库实际装载的元素数，用于复核与指标。
    pub element_counts: BTreeMap<u32, usize>,
}

impl DepsEpoch {
    /// 从装载结果构造，内容哈希由依赖库集合与逐库元素数决定。
    pub fn new(dbnum: u32, epoch: u64, element_counts: BTreeMap<u32, usize>) -> Self {
        let dependency_dbnums: Vec<u32> = element_counts.keys().copied().collect();
        let mut parts = vec![format!("dbnum:{dbnum}"), format!("epoch:{epoch}")];
        for (dep, count) in &element_counts {
            parts.push(format!("{dep}={count}"));
        }
        Self {
            dbnum,
            epoch,
            hash: stable_hash(&parts),
            dependency_dbnums,
            element_counts,
        }
    }

    /// 复核 deps 未被改动。不一致属不可恢复错误（spec 030 R4）。
    pub fn assert_unchanged(&self, observed: &DepsEpoch) -> Result<()> {
        if self.dbnum != observed.dbnum || self.epoch != observed.epoch {
            bail!(
                "deps epoch 身份不匹配：期望 dbnum={} epoch={}，实际 dbnum={} epoch={}",
                self.dbnum,
                self.epoch,
                observed.dbnum,
                observed.epoch
            );
        }
        if self.hash != observed.hash {
            bail!(
                "dbnum {} 的共享依赖库在 ZONE 流水期间发生变化（deps hash {} → {}）：\
                 这会让已完成 ZONE 与后续 ZONE 读到不同的依赖，属不可恢复错误",
                self.dbnum,
                self.hash,
                observed.hash
            );
        }
        Ok(())
    }
}

/// 单个 ZONE 解析完成后的范围完整性证明（ADR-0016 D6）。
///
/// 四个计数分别对应设计稿要求证明的四件事：子树、祖先链、CATA 闭包、transform。
/// 任何一项为 0（除祖先链在根 ZONE 的特例外）都说明装载不完整，不允许进入生成。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ZoneScopeSeal {
    pub zone_refno: String,
    pub dbnum: u32,
    /// 绑定到具体的 deps epoch：换了依赖就是另一个 seal。
    pub deps_epoch: u64,
    pub deps_hash: String,
    /// ZONE 子树内的设计元素数（不含祖先链）。
    pub subtree_elements: usize,
    /// owner 祖先链补齐的元素数。
    pub ancestor_elements: usize,
    /// CATA 闭包覆盖到的库，按 dbnum 升序。
    pub cata_dbnums: Vec<u32>,
    /// CATA 闭包覆盖到的元素总数。
    pub cata_elements: usize,
    /// 该 ZONE 范围内已解析出的 transform 数。
    pub transform_count: usize,
    pub hash: String,
}

impl ZoneScopeSeal {
    #[allow(clippy::too_many_arguments)]
    pub fn seal(
        zone_refno: &str,
        dbnum: u32,
        deps: &DepsEpoch,
        subtree_elements: usize,
        ancestor_elements: usize,
        cata_dbnums: Vec<u32>,
        cata_elements: usize,
        transform_count: usize,
    ) -> Result<Self> {
        if subtree_elements == 0 {
            bail!("ZONE `{zone_refno}` 的子树为空：不允许对空范围封存");
        }
        if cata_dbnums.is_empty() || cata_elements == 0 {
            bail!(
                "ZONE `{zone_refno}` 的 CATA 闭包为空：几何生成必然缺元件定义，\
                 不允许带着空闭包进入生成"
            );
        }
        if transform_count == 0 {
            bail!(
                "ZONE `{zone_refno}` 没有解析出任何 transform：世界矩阵缺失会让产物位置全错"
            );
        }

        let mut parts = vec![
            format!("zone:{zone_refno}"),
            format!("dbnum:{dbnum}"),
            format!("deps:{}:{}", deps.epoch, deps.hash),
            format!("subtree:{subtree_elements}"),
            format!("ancestors:{ancestor_elements}"),
            format!("cata_elements:{cata_elements}"),
            format!("transforms:{transform_count}"),
        ];
        for db in &cata_dbnums {
            parts.push(format!("cata_db:{db}"));
        }

        Ok(Self {
            zone_refno: zone_refno.to_string(),
            dbnum,
            deps_epoch: deps.epoch,
            deps_hash: deps.hash.clone(),
            subtree_elements,
            ancestor_elements,
            cata_dbnums,
            cata_elements,
            transform_count,
            hash: stable_hash(&parts),
        })
    }
}