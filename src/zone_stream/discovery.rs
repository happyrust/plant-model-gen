//! 从源 DESI 文件枚举 ZONE（spec 030 Phase 6 的规划输入）。
//!
//! ## 为什么不能用现成的两条路
//!
//! - **`db_index.sqlite` 不行**：它只存 `dbnum / db_type / file / latest_sesno / fingerprint`
//!   与 `ref0 -> dbnum` 归属，**没有元素类型**（`scan_one_db` 走 `build_index_map()`，
//!   index-only、不解析元素记录）。它答得出「这个 ref0 属于哪个库」，答不出「哪些 refno 是 ZONE」。
//! - **`query_type_refnos_by_dbnum(&["ZONE"], ..)` 不行**：它查的是已解析的 SurrealDB。
//!   ZoneStream 是**初始化**，目标库此刻是空的 —— 规划阶段根本没有可查的数据。
//!
//! ## 采用的办法：搭现有的那趟扫描
//!
//! `cata_closure::seed_refs_from_design_file` 已经在做「遍历 `refno_table_map` →
//! 按偏移随机访问 → `parse_ele_data_with_info_sync` 单元素解析」，为的是收集 CATA 种子。
//! 而单元素解析出来的 `NamedAttrMap` 本身就带 `get_type_str()`（noun）和 `get_owner()`。
//!
//! 所以枚举 ZONE **不需要第二趟扫描**：同一趟里顺手把 noun 是 `ZONE` 的挑出来，
//! 同时把 owner 记下来用于排序。多做的只是每个元素一次字符串比较，文件 I/O 一次不多。
//!
//! 备选方案是「从 DESI 根沿 WORL → SITE → ZONE 逐层下降」，理论上只解析三层更省。
//! 没选它的原因：需要先拿到库根 refno（`DbBasicData` 没有直接暴露），且要假定 ZONE 一定
//! 挂在 SITE 下第二层；一旦工程里有嵌套或非常规层级就会漏。而搭车方案的边际成本本来就是零。
//!
//! ## 「当前最新」怎么保证
//!
//! `parse_file_db_basic_data` 读的是文件当前状态。调用方须把 `latest_sesno`
//! （`db_index.sqlite` 的 `db_file_index.latest_sesno`，或 `PdmsIO::get_latest_sesno()`）
//! 一并记入源清单哈希：源漂移由 Resume 判等发现并判为不可恢复错误（spec 030 R15），
//! 而不是靠这里重新扫一遍去比对。

use std::collections::BTreeMap;
use std::path::Path;

use aios_core::RefU64;
use anyhow::{bail, Context, Result};

/// 设计库里发现的一个 ZONE。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiscoveredZone {
    pub refno: RefU64,
    /// owner（通常是 SITE）；稳定序按 (owner, refno) 排。
    pub owner: RefU64,
}

/// 一趟 DESI 扫描的产物。
#[derive(Debug, Clone)]
pub struct DesignSweep {
    pub dbnum: u32,
    /// 已按稳定序排好的 ZONE：先按 owner（SITE）升序，再按 ZONE refno 升序。
    pub zones: Vec<DiscoveredZone>,
    /// 该库全部元素的出向引用，作为 CATA 闭包种子 —— 与 ZONE 枚举同一趟得到。
    pub cata_seeds: Vec<RefU64>,
    /// 索引里的 refno 总数。
    pub indexed_total: usize,
    /// 实际成功解析的元素数。
    pub parsed_total: usize,
}

impl DesignSweep {
    /// 供 [`super::plan::ZonePlan::new`] 使用的 refno 字面量序列（已是稳定序）。
    pub fn zone_refno_strings(&self) -> Vec<String> {
        self.zones
            .iter()
            .map(|zone| zone.refno.to_string())
            .collect()
    }
}

/// 扫描单个 DESI 文件，一趟同时得到 ZONE 列表与 CATA 种子。
///
/// 这是同步的重 I/O + CPU 操作（整文件读入 + 逐元素解析），调用方应放进
/// `tokio::task::spawn_blocking`。
pub fn sweep_design_file(project: &str, desi_path: &Path) -> Result<DesignSweep> {
    use parse_pdms_db::parse::{
        parse_db_basic_info, parse_ele_data_with_info_sync, parse_file_db_basic_data,
    };

    let file_name = desi_path
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or_default();
    let dbnum = parse_db_basic_info(desi_path.to_path_buf()).dbnum;
    let basic = parse_file_db_basic_data(&desi_path.to_path_buf(), file_name, project)
        .with_context(|| format!("读取 DESI 库失败: {}", desi_path.display()))?;

    let indexed_total = basic.refno_table_map.len();
    if indexed_total == 0 {
        bail!("DESI 库没有可解析的 refno: {}", desi_path.display());
    }

    let db_info = aios_core::get_default_pdms_db_info();
    let mut zones: Vec<DiscoveredZone> = Vec::new();
    let mut seeds: std::collections::HashSet<RefU64> = std::collections::HashSet::new();
    let mut parsed_total = 0usize;

    for entry in basic.refno_table_map.iter() {
        let pos = entry.value().pos;
        if pos < 4 || pos > basic.bytes.len() {
            continue;
        }
        let Ok(ele) = parse_ele_data_with_info_sync(&basic.bytes[pos - 4..], &db_info) else {
            continue;
        };
        parsed_total += 1;

        let att = ele.whole_attmap.merge();
        seeds.extend(crate::data_interface::cata_closure::outbound_refs_of(&att));

        if att.get_type_str().eq_ignore_ascii_case("ZONE") {
            // refno 以索引键为准：单元素解析不从文件头注入 dbnum，属性表里的 refno
            // 未必完整（同 parse_refnos_with_session 里补 dbnum 的理由）。
            zones.push(DiscoveredZone {
                refno: *entry.key(),
                owner: att.get_owner().into(),
            });
        }
    }

    if parsed_total == 0 {
        bail!(
            "DESI 库索引包含 {indexed_total} 个 refno，但随机访问解析结果为空: {}",
            desi_path.display()
        );
    }

    // 稳定序：先 owner（SITE）升序，再 ZONE refno 升序。
    // 用 owner 而不是纯 refno 排，是为了让同一个 SITE 下的 ZONE 连续执行 ——
    // 它们的 CATA 闭包重合度最高，deps 命中率也最好。
    zones.sort_by_key(|zone| (zone.owner, zone.refno));
    zones.dedup_by_key(|zone| zone.refno);

    Ok(DesignSweep {
        dbnum,
        zones,
        cata_seeds: seeds.into_iter().collect(),
        indexed_total,
        parsed_total,
    })
}

/// 多个 DESI 库的扫描结果汇总成 `dbnum -> ZONE refno 序列`，可直接喂给
/// [`super::plan::ZonePlan::new`]。
///
/// 没有 ZONE 的库会被跳过并返回在第二个元素里：让调用方决定是「该库确实没有 ZONE，
/// 属正常」还是「扫错了库，该报错」，而不是在这里替它判断。
pub fn zone_plan_input(sweeps: &[DesignSweep]) -> (BTreeMap<u32, Vec<String>>, Vec<u32>) {
    let mut plan_input = BTreeMap::new();
    let mut empty_dbnums = Vec::new();
    for sweep in sweeps {
        if sweep.zones.is_empty() {
            empty_dbnums.push(sweep.dbnum);
            continue;
        }
        plan_input.insert(sweep.dbnum, sweep.zone_refno_strings());
    }
    (plan_input, empty_dbnums)
}
