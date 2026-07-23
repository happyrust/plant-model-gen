//! 目录反向波及闭包 expander —— ADR-0011 **P2 shadow**（落地方案 §6 P2 / §9 "未做"）。
//!
//! 消费 [`super::cata_ref_index`] 的一跳入边读原语 [`load_inbound_references`]，做**多跳
//! 反查 BFS**（`SPRE→CATR→…→SCOM` 的反方向），得到「引用了被改目录定义的实例集」。
//! 带**环 / 深度 / 规模**三重保护（ADR-0011 Q5：传递闭包/环/深度收敛归 expander）。
//!
//! ## 红线（务必遵守，见 ADR-0011 §后果 / 落地方案 §5）
//! - **本层只旁路计算，不改生成目标**。真正并入 `IncrGeoUpdateLog` 是 P3 接管，
//!   等 specs/027 项目 run barrier 就绪后按项目级落；本轮不建临时 per-db 扇出。
//! - 反向扇出仅用于**目录定义被改**；设计实例改自身 `CATR/SPRE/PRTREF` = direct-only，
//!   由调用方在挑选 seeds 时保证（本 expander 只忠实对给定 seeds 做反查）。
//! - 规模超限（热门 SCOM）时置 `truncated_size`，作为 P3/M4「超集降级
//!   （FullDb/FullProject）」的信号，不在此层决定降级。

use std::collections::HashSet;

use aios_core::RefU64;

use super::cata_ref_index::load_inbound_references;

/// 反查闭包的保护上限。
#[derive(Debug, Clone)]
pub struct ReverseClosureLimits {
    /// BFS 最大跳数（多跳 `SPRE→CATR→SCOM` 反向的深度上限）。
    pub max_depth: usize,
    /// 结果实例规模上限；超过即置 `truncated_size`（超集降级信号）。
    pub max_instances: usize,
    /// 单跳 `load_inbound_references` 的 LIMIT。
    pub per_hop_limit: usize,
}

impl Default for ReverseClosureLimits {
    fn default() -> Self {
        Self {
            max_depth: 8,
            max_instances: 50_000,
            per_hop_limit: 10_000,
        }
    }
}

/// 反查闭包结果（shadow：仅供日志/对账，不消费）。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ReverseClosureResult {
    /// 反查到的引用源实例（去重、已剔除 seeds，按 refno 字符串稳定排序）。
    pub instances: Vec<RefU64>,
    /// 最深的「有新增实例」的跳数（seed 为第 0 层；barren 的末跳不计）。
    pub depth_reached: usize,
    /// 因 `max_depth` 截断（可能还有更深引用未展开）。
    pub truncated_depth: bool,
    /// 因 `max_instances` 截断（热门目录 → P3 应超集降级）。
    pub truncated_size: bool,
    /// 访问过的节点总数（seeds + 各跳 source），用于观测。
    pub visited_count: usize,
}

/// BFS 累加器：把「环/规模/去种子」逻辑收敛到一处，供同步（可测）与异步（真机）两条驱动共用。
struct ClosureAccumulator<'a> {
    limits: &'a ReverseClosureLimits,
    seeds: HashSet<RefU64>,
    visited: HashSet<RefU64>,
    instances: HashSet<RefU64>,
    truncated_size: bool,
}

impl<'a> ClosureAccumulator<'a> {
    fn new(seeds: &[RefU64], limits: &'a ReverseClosureLimits) -> (Self, Vec<RefU64>) {
        let mut me = Self {
            limits,
            seeds: HashSet::new(),
            visited: HashSet::new(),
            instances: HashSet::new(),
            truncated_size: false,
        };
        let mut frontier = Vec::new();
        for &seed in seeds {
            if seed.is_unset() {
                continue;
            }
            me.seeds.insert(seed);
            if me.visited.insert(seed) {
                frontier.push(seed);
            }
        }
        (me, frontier)
    }

    fn instances_len(&self) -> usize {
        self.instances.len()
    }

    /// 吸收一跳的 source 集合，返回下一跳 frontier（本跳新访问到的节点）。
    /// 命中规模上限时置 `truncated_size` 并返回空 frontier（终止扩散）。
    fn absorb(&mut self, sources: impl IntoIterator<Item = RefU64>) -> Vec<RefU64> {
        let mut next = Vec::new();
        for src in sources {
            if src.is_unset() || self.seeds.contains(&src) {
                continue;
            }
            if self.instances.len() >= self.limits.max_instances {
                self.truncated_size = true;
                return Vec::new();
            }
            self.instances.insert(src);
            if self.visited.insert(src) {
                next.push(src);
            }
        }
        next
    }

    fn finish(self, depth_reached: usize, truncated_depth: bool) -> ReverseClosureResult {
        let mut instances: Vec<RefU64> = self.instances.into_iter().collect();
        instances.sort_by_key(RefU64::to_string);
        ReverseClosureResult {
            instances,
            depth_reached,
            truncated_depth,
            truncated_size: self.truncated_size,
            visited_count: self.visited.len(),
        }
    }
}

/// 纯 BFS 反查闭包（无 DB，注入 `lookup` 便于单测）。
///
/// `lookup(frontier)` 语义：返回**引用了 `frontier` 中任一 target 的 source**集合（一跳入边）。
/// seeds（被改目录定义）不计入结果实例；环由 visited 去重防死循环；深度/规模按 `limits` 截断。
pub fn expand_reverse_closure<F>(
    seeds: &[RefU64],
    limits: &ReverseClosureLimits,
    mut lookup: F,
) -> ReverseClosureResult
where
    F: FnMut(&[RefU64]) -> Vec<RefU64>,
{
    let (mut acc, mut frontier) = ClosureAccumulator::new(seeds, limits);
    let mut depth = 0usize;
    let mut depth_reached = 0usize;
    let mut truncated_depth = false;
    while !frontier.is_empty() {
        if depth >= limits.max_depth {
            truncated_depth = true;
            break;
        }
        depth += 1;
        let before = acc.instances_len();
        let sources = lookup(&frontier);
        frontier = acc.absorb(sources);
        if acc.instances_len() > before {
            depth_reached = depth;
        }
        if acc.truncated_size {
            break;
        }
    }
    acc.finish(depth_reached, truncated_depth)
}

/// P2 shadow 入口：对「被改的目录定义 seeds」做反查闭包（多跳），使用 `cata_ref_index` 一跳读。
///
/// `families=None` 表示不按属性族过滤（收录全属性、传播期再滤，ADR-0011 Q2）。
/// **只计算并返回，不改任何生成目标**（P3 才接管）。调用方应先满足项目级 ready 门。
pub async fn expand_catalogue_reverse_targets(
    seeds: &[RefU64],
    families: Option<&[String]>,
    limits: &ReverseClosureLimits,
) -> anyhow::Result<ReverseClosureResult> {
    let (mut acc, mut frontier) = ClosureAccumulator::new(seeds, limits);
    let mut depth = 0usize;
    let mut depth_reached = 0usize;
    let mut truncated_depth = false;
    while !frontier.is_empty() {
        if depth >= limits.max_depth {
            truncated_depth = true;
            break;
        }
        depth += 1;
        let before = acc.instances_len();
        let edges = load_inbound_references(&frontier, families, limits.per_hop_limit).await?;
        // 入边的 source 即「引用了当前 frontier 的元素」；String→RefU64 回解以便下一跳继续反查。
        let sources = edges
            .into_iter()
            .filter_map(|edge| edge.source_refno.parse::<RefU64>().ok());
        frontier = acc.absorb(sources);
        if acc.instances_len() > before {
            depth_reached = depth;
        }
        if acc.truncated_size {
            break;
        }
    }
    Ok(acc.finish(depth_reached, truncated_depth))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn r(v: &str) -> RefU64 {
        v.parse::<RefU64>().expect("refno")
    }

    /// 用 target->sources 的邻接表构造一个可注入 lookup（一跳入边）。
    fn graph_lookup(
        edges: &HashMap<RefU64, Vec<RefU64>>,
    ) -> impl FnMut(&[RefU64]) -> Vec<RefU64> + '_ {
        move |frontier: &[RefU64]| {
            let mut out = Vec::new();
            for t in frontier {
                if let Some(srcs) = edges.get(t) {
                    out.extend(srcs.iter().copied());
                }
            }
            out
        }
    }

    #[test]
    fn multi_hop_reverse_closure_collects_all_referrers() {
        let mut g: HashMap<RefU64, Vec<RefU64>> = HashMap::new();
        g.insert(r("300_1"), vec![r("200_1")]);
        g.insert(r("200_1"), vec![r("100_1"), r("100_2")]);
        let out =
            expand_reverse_closure(&[r("300_1")], &ReverseClosureLimits::default(), graph_lookup(&g));
        assert_eq!(out.instances, vec![r("100_1"), r("100_2"), r("200_1")]);
        assert_eq!(out.depth_reached, 2);
        assert!(!out.truncated_depth && !out.truncated_size);
    }

    #[test]
    fn cycle_is_bounded_and_seed_excluded() {
        let mut g: HashMap<RefU64, Vec<RefU64>> = HashMap::new();
        g.insert(r("1_1"), vec![r("1_2")]);
        g.insert(r("1_2"), vec![r("1_1")]);
        let out =
            expand_reverse_closure(&[r("1_1")], &ReverseClosureLimits::default(), graph_lookup(&g));
        assert_eq!(out.instances, vec![r("1_2")]);
        assert!(!out.truncated_depth);
    }

    #[test]
    fn depth_limit_truncates() {
        let mut g: HashMap<RefU64, Vec<RefU64>> = HashMap::new();
        g.insert(r("5_1"), vec![r("5_2")]);
        g.insert(r("5_2"), vec![r("5_3")]);
        g.insert(r("5_3"), vec![r("5_4")]);
        let limits = ReverseClosureLimits { max_depth: 1, ..Default::default() };
        let out = expand_reverse_closure(&[r("5_1")], &limits, graph_lookup(&g));
        assert_eq!(out.instances, vec![r("5_2")]);
        assert!(out.truncated_depth);
        assert_eq!(out.depth_reached, 1);
    }

    #[test]
    fn size_limit_flags_superset_downgrade() {
        let mut g: HashMap<RefU64, Vec<RefU64>> = HashMap::new();
        g.insert(r("9_1"), vec![r("9_2"), r("9_3"), r("9_4"), r("9_5")]);
        let limits = ReverseClosureLimits { max_instances: 2, ..Default::default() };
        let out = expand_reverse_closure(&[r("9_1")], &limits, graph_lookup(&g));
        assert!(out.truncated_size);
        assert!(out.instances.len() <= 2);
    }

    #[test]
    fn empty_seeds_yield_empty() {
        let g: HashMap<RefU64, Vec<RefU64>> = HashMap::new();
        let out = expand_reverse_closure(&[], &ReverseClosureLimits::default(), graph_lookup(&g));
        assert!(out.instances.is_empty());
        assert_eq!(out.depth_reached, 0);
        assert!(!out.truncated_depth && !out.truncated_size);
    }
}
